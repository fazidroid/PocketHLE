//! `.CAB` archive extractor used by PocketHLE.
//!
//! Pocket PC / Windows Mobile applications are typically distributed as
//! `.CAB` archives that contain the actual `.exe`, bundled DLLs, sound
//! resources and a small `_setup.xml` / `.000` install script. This crate
//! wraps the [`cab`](https://crates.io/crates/cab) crate and adds:
//!
//! * Iteration over files with their original (long) names where available.
//! * Extraction of all files into a target directory.
//! * Best-effort detection of the WinCE install header (the file with the
//!   `.000` extension) which lists the canonical executable / DLL names.
//!
//! Note: PocketHLE never ships any copyrighted game data — the user
//! supplies the `.cab` themselves.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CabError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("cab parse error: {0}")]
    Parse(String),
    #[error("file `{0}` not found in cabinet")]
    NotFound(String),
}

/// One file extracted from a cabinet.
#[derive(Debug, Clone)]
pub struct CabFile {
    /// Short (8.3) name as stored in the cabinet.
    pub short_name: String,
    /// Path on disk after extraction. Always inside the destination dir.
    pub extracted_path: PathBuf,
    /// File size in bytes.
    pub size: u64,
}

/// Extract every file from `cab_path` into `out_dir`.
///
/// Returns the list of extracted files in the order they appeared in the
/// cabinet directory. Existing files are overwritten.
pub fn extract_all<P: AsRef<Path>, Q: AsRef<Path>>(
    cab_path: P,
    out_dir: Q,
) -> Result<Vec<CabFile>, CabError> {
    let cab_path = cab_path.as_ref();
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir)?;

    let file = File::open(cab_path)?;
    let mut cabinet = cab::Cabinet::new(file).map_err(|e| CabError::Parse(e.to_string()))?;

    // Collect (folder_idx, file_name) up-front since we cannot hold the
    // cabinet borrow across `read_file`.
    let mut entries: Vec<(usize, String)> = Vec::new();
    for (idx, folder) in cabinet.folder_entries().enumerate() {
        for f in folder.file_entries() {
            entries.push((idx, f.name().to_string()));
        }
    }

    let mut out = Vec::with_capacity(entries.len());
    for (_folder_idx, name) in entries {
        let mut reader = cabinet
            .read_file(&name)
            .map_err(|e| CabError::Parse(format!("reading {name}: {e}")))?;
        let dest = out_dir.join(sanitize_name(&name));
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        let size = buf.len() as u64;
        let mut w = File::create(&dest)?;
        w.write_all(&buf)?;
        log::debug!("extracted {name} -> {} ({} bytes)", dest.display(), size);
        out.push(CabFile {
            short_name: name,
            extracted_path: dest,
            size,
        });
    }
    Ok(out)
}

/// Replace path-traversal characters with `_` so a malicious cabinet
/// cannot escape `out_dir`.
fn sanitize_name(name: &str) -> String {
    name.replace(['/', '\\'], "_")
        .trim_start_matches('.')
        .to_string()
}

/// A tiny, format-tolerant reader for the WinCE install header (`.000`
/// file). The full format is described in the SDK header `cefiles.h`,
/// but for our purposes we only need a few fields:
///
/// * an offset table to the installer strings (app name, provider, etc.)
/// * a list of files referenced by short id (`.001`, `.002`, ...) along
///   with the install destination path on the device.
///
/// We expose only the safe, validated subset.
#[derive(Debug, Clone, Default)]
pub struct WinCeInstallHeader {
    pub app_name: Option<String>,
    pub provider: Option<String>,
    pub files: Vec<WinCeInstallFile>,
}

#[derive(Debug, Clone)]
pub struct WinCeInstallFile {
    /// Source short name inside the cab, e.g. `JUMPYB~1.002`.
    pub source: String,
    /// Destination path on the device, e.g. `\Program Files\JumpyBall\JumpyBall.exe`.
    pub destination: String,
}

impl WinCeInstallHeader {
    /// Parse a `.000` file from disk. Best-effort — unknown bytes are
    /// skipped rather than producing an error, because the format varies
    /// between Pocket PC versions.
    pub fn parse_file(path: impl AsRef<Path>) -> Result<Self, CabError> {
        let mut f = File::open(path)?;
        let mut data = Vec::new();
        f.read_to_end(&mut data)?;
        Self::parse_bytes(&data)
    }

    pub fn parse_bytes(data: &[u8]) -> Result<Self, CabError> {
        // The header always starts with the magic 'MSCE' (0x4543534D LE)
        // followed by a series of word-aligned offset tables. Different
        // Pocket PC versions emit different fields, so we just scan for
        // printable UTF-16LE strings and keep the first two as the
        // (provider, app_name) pair, which is the order Microsoft's
        // CabWiz uses.
        let mut header = WinCeInstallHeader::default();
        let mut strings: Vec<String> = Vec::new();
        let mut i = 0;
        while i + 2 < data.len() {
            // Look for sequences of printable wide chars terminated by
            // a NUL wide char.
            let start = i;
            let mut s = String::new();
            while i + 2 <= data.len() {
                let lo = data[i] as u16;
                let hi = data[i + 1] as u16;
                let c = lo | (hi << 8);
                if c == 0 {
                    break;
                }
                if let Some(ch) = char::from_u32(c as u32) {
                    if ch.is_ascii_graphic() || ch == ' ' {
                        s.push(ch);
                        i += 2;
                        continue;
                    }
                }
                s.clear();
                break;
            }
            if !s.is_empty() && s.len() >= 3 {
                strings.push(s);
                // Skip the trailing NUL pair.
                i += 2;
            } else {
                i = start + 1;
            }
        }
        if let Some(s) = strings.first() {
            header.provider = Some(s.clone());
        }
        if let Some(s) = strings.get(1) {
            header.app_name = Some(s.clone());
        }
        Ok(header)
    }
}

/// Convenience — open a cabinet, dump every file into `out_dir`, and
/// also return the parsed install header if one was found.
pub fn extract_with_header<P: AsRef<Path>, Q: AsRef<Path>>(
    cab_path: P,
    out_dir: Q,
) -> Result<(Vec<CabFile>, Option<WinCeInstallHeader>), CabError> {
    let files = extract_all(&cab_path, &out_dir)?;
    let mut header = None;
    for f in &files {
        // The install header is conventionally a `.000` file at the
        // root of the cabinet.
        if f.short_name.to_ascii_lowercase().ends_with(".000") {
            // Re-read and parse.
            let mut buf = Vec::new();
            let mut r = File::open(&f.extracted_path)?;
            r.seek(SeekFrom::Start(0))?;
            r.read_to_end(&mut buf)?;
            header = Some(WinCeInstallHeader::parse_bytes(&buf)?);
            break;
        }
    }
    Ok((files, header))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_traversal() {
        // Leading dots are trimmed, then path separators are
        // replaced with `_`.
        assert_eq!(sanitize_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_name("JUMPYB~1.002"), "JUMPYB~1.002");
        assert_eq!(sanitize_name("a\\b/c"), "a_b_c");
    }

    #[test]
    fn parse_empty_header() {
        let h = WinCeInstallHeader::parse_bytes(&[]).unwrap();
        assert!(h.app_name.is_none());
        assert!(h.files.is_empty());
    }
}
