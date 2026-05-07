//! Auto-extraction of `.cab` and `.zip` archives so that
//! `pockethle run game.cab` (or `game.zip`) just works.
//!
//! Pocket PC titles are almost always shipped as a single `.cab` that
//! contains the executable, helper DLLs and game assets, or as a
//! `.zip` snapshot of an already-installed program. Both shapes need
//! the same handling: extract everything into a sandboxed directory,
//! locate the ARM PE that is the actual game, and mount the directory
//! as the guest's `\Application\` so `CreateFileW` can find the
//! resources next to the binary.
//!
//! Returned [`Launcher`] keeps the temp directory alive — drop it and
//! the extracted files are removed.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use tempfile::TempDir;

/// Result of preparing an archive (or a plain `.exe`) for emulation.
pub struct Launcher {
    /// Absolute path to the PE32 ARM executable to load.
    pub exe: PathBuf,
    /// If we extracted an archive, the directory holding all
    /// extracted files. Mount this as `\Application\` so the guest's
    /// `CreateFileW` finds the resources that sat next to the EXE.
    pub mount_dir: Option<PathBuf>,
    /// Hint about what we did, printed to the user.
    pub origin: String,
    /// Owns the temp directory; kept here so it is not removed until
    /// the emulator is done.
    _tempdir: Option<TempDir>,
}

/// Inspect `path` and produce a [`Launcher`].
///
/// * `.cab` — extract via [`pocket_core::cab::extract_with_header`]
///   and pick the largest `IMAGE_FILE_MACHINE_ARM` PE.
/// * `.zip` — extract every entry, pick the largest ARM PE.
/// * anything else — treated as a PE on disk, no extraction.
///
/// Returns an error if no ARM PE is found. The user can still call
/// `pockethle pe-info` for diagnostics on a single file.
pub fn prepare(path: &Path) -> Result<Launcher> {
    let kind = ArchiveKind::detect(path);
    match kind {
        ArchiveKind::Cab => prepare_cab(path),
        ArchiveKind::Zip => prepare_zip(path),
        ArchiveKind::Pe => Ok(Launcher {
            exe: path.to_path_buf(),
            mount_dir: None,
            origin: format!("PE file {}", path.display()),
            _tempdir: None,
        }),
    }
}

#[derive(Debug, Clone, Copy)]
enum ArchiveKind {
    Cab,
    Zip,
    Pe,
}

impl ArchiveKind {
    fn detect(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("cab") => Self::Cab,
            Some("zip") => Self::Zip,
            _ => Self::Pe,
        }
    }
}

fn prepare_cab(path: &Path) -> Result<Launcher> {
    let tmp = TempDir::with_prefix("pockethle-cab-")
        .with_context(|| format!("creating temp dir for {}", path.display()))?;
    let (files, header) = pocket_core::cab::extract_with_header(path, tmp.path())
        .with_context(|| format!("extracting {}", path.display()))?;

    if files.is_empty() {
        return Err(anyhow!(
            "{} contains no files (corrupt cabinet?)",
            path.display()
        ));
    }

    let exe_path = pick_arm_pe(files.iter().map(|f| f.extracted_path.as_path()))
        .with_context(|| format!("looking for an ARM PE inside {}", path.display()))?;

    let mut origin = format!("CAB {} -> {}", path.display(), exe_path.display());
    if let Some(h) = header {
        if let (Some(provider), Some(app)) = (&h.provider, &h.app_name) {
            origin = format!("{origin} ({provider} / {app})");
        }
    }

    Ok(Launcher {
        exe: exe_path,
        mount_dir: Some(tmp.path().to_path_buf()),
        origin,
        _tempdir: Some(tmp),
    })
}

fn prepare_zip(path: &Path) -> Result<Launcher> {
    let tmp = TempDir::with_prefix("pockethle-zip-")
        .with_context(|| format!("creating temp dir for {}", path.display()))?;
    let f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive =
        zip::ZipArchive::new(f).with_context(|| format!("parsing zip {}", path.display()))?;
    let mut written: Vec<PathBuf> = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name().map(Path::to_path_buf) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let dest = tmp.path().join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest)?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&dest)?;
        std::io::copy(&mut entry, &mut out)?;
        written.push(dest);
    }
    if written.is_empty() {
        return Err(anyhow!("{} contains no files", path.display()));
    }

    // Pocket PC titles are sometimes shipped as a `.zip` whose only
    // entry is itself a `.cab` (or the desktop ActiveSync installer
    // bundles the .cab next to the desktop wrapper). Recurse into
    // any nested `.cab` so the user-facing UX is still
    // "pockethle run game.zip".
    if let Some(nested_cab) = written
        .iter()
        .find(|p| p.extension().and_then(|e| e.to_str()) == Some("cab"))
    {
        log::info!(
            "zip contains nested cab {}, recursing",
            nested_cab.display()
        );
        let mut inner = prepare_cab(nested_cab)?;
        inner.origin = format!("ZIP {} -> {}", path.display(), inner.origin);
        // Keep the ZIP's tempdir alive as long as the CAB tempdir is
        // alive: stash both by piggy-backing on the inner launcher's
        // origin and making the outer tmpdir the new owner.
        inner._tempdir = Some(merge_tempdirs(tmp, inner._tempdir));
        return Ok(inner);
    }

    let exe_path = pick_arm_pe(written.iter().map(PathBuf::as_path)).with_context(|| {
        format!(
            "no ARM PE found in {}: {} contains only desktop binaries; \
             try the matching `.cab` instead",
            path.display(),
            path.file_name().unwrap_or_default().to_string_lossy(),
        )
    })?;

    let origin = format!("ZIP {} -> {}", path.display(), exe_path.display());
    Ok(Launcher {
        exe: exe_path,
        mount_dir: Some(tmp.path().to_path_buf()),
        origin,
        _tempdir: Some(tmp),
    })
}

/// Keep `inner` on disk for the rest of the process's lifetime and
/// return `outer` as the single owner. We can only stash one
/// `TempDir` on `Launcher`, so when a `.zip` recurses into a `.cab`
/// we deliberately leak the inner directory — both live under
/// `$TMPDIR` and are cleaned up by the OS at reboot.
fn merge_tempdirs(outer: TempDir, inner: Option<TempDir>) -> TempDir {
    if let Some(i) = inner {
        let _ = i.keep();
    }
    outer
}

/// IMAGE_FILE_MACHINE_ARM. `pocket-pe` exposes the same constant via
/// `Image::machine_name`, but we deliberately read raw bytes here so
/// we can scan thousands of files quickly without parsing every PE.
const IMAGE_FILE_MACHINE_ARM: u16 = 0x01c0;
const IMAGE_FILE_MACHINE_THUMB: u16 = 0x01c2;
const IMAGE_FILE_MACHINE_ARMNT: u16 = 0x01c4;

/// Walk `paths` and return the largest one whose PE header advertises
/// an ARM machine. We sort by file size descending so games whose cab
/// also bundles a tiny `setup.exe` still pick the actual game binary.
fn pick_arm_pe<'a, I>(paths: I) -> Result<PathBuf>
where
    I: IntoIterator<Item = &'a Path>,
{
    let mut candidates: Vec<(u64, PathBuf)> = Vec::new();
    for p in paths {
        let Ok(meta) = std::fs::metadata(p) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        if is_arm_pe(p).unwrap_or(false) {
            candidates.push((meta.len(), p.to_path_buf()));
        }
    }
    candidates.sort_by_key(|c| std::cmp::Reverse(c.0));
    candidates
        .into_iter()
        .next()
        .map(|(_, p)| p)
        .ok_or_else(|| anyhow!("no PE32 ARM executable found"))
}

/// Cheap check for the PE/COFF header: read 0x40 bytes, follow the
/// `e_lfanew` offset, verify `PE\0\0` and read the machine type.
/// Returns `Ok(false)` for short reads or non-PE files (so we skip
/// them silently rather than failing the whole launch).
fn is_arm_pe(path: &Path) -> std::io::Result<bool> {
    let mut f = File::open(path)?;
    let mut head = [0u8; 0x40];
    let n = f.read(&mut head)?;
    if n < 0x40 {
        return Ok(false);
    }
    if &head[0..2] != b"MZ" {
        return Ok(false);
    }
    let lfanew = u32::from_le_bytes(head[0x3c..0x40].try_into().unwrap()) as u64;
    use std::io::{Seek, SeekFrom};
    f.seek(SeekFrom::Start(lfanew))?;
    let mut sig = [0u8; 6];
    if f.read(&mut sig)? < 6 {
        return Ok(false);
    }
    if &sig[0..4] != b"PE\0\0" {
        return Ok(false);
    }
    let machine = u16::from_le_bytes([sig[4], sig[5]]);
    Ok(matches!(
        machine,
        IMAGE_FILE_MACHINE_ARM | IMAGE_FILE_MACHINE_THUMB | IMAGE_FILE_MACHINE_ARMNT
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn detect_kinds() {
        assert!(matches!(
            ArchiveKind::detect(Path::new("game.CAB")),
            ArchiveKind::Cab
        ));
        assert!(matches!(
            ArchiveKind::detect(Path::new("game.zip")),
            ArchiveKind::Zip
        ));
        assert!(matches!(
            ArchiveKind::detect(Path::new("Game.exe")),
            ArchiveKind::Pe
        ));
        assert!(matches!(
            ArchiveKind::detect(Path::new("noext")),
            ArchiveKind::Pe
        ));
    }

    #[test]
    fn arm_pe_detection() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fake.exe");
        let mut buf = vec![0u8; 0x100];
        buf[0..2].copy_from_slice(b"MZ");
        // e_lfanew at 0x80
        buf[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        buf.resize(0x90, 0);
        buf[0x80..0x84].copy_from_slice(b"PE\0\0");
        buf[0x84..0x86].copy_from_slice(&IMAGE_FILE_MACHINE_ARM.to_le_bytes());
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&buf)
            .unwrap();
        assert!(is_arm_pe(&path).unwrap());

        // Now overwrite to x86 — should be rejected.
        buf[0x84..0x86].copy_from_slice(&0x014cu16.to_le_bytes());
        std::fs::write(&path, &buf).unwrap();
        assert!(!is_arm_pe(&path).unwrap());
    }
}
