//! Game library and persistent configuration shared by all GUI
//! frontends (the egui desktop launcher and the Android launcher).
//!
//! The library lives on disk under a single root directory:
//!
//! ```text
//! <library_root>/
//!     library.json     # registry of installed games (this crate)
//!     config.json      # global launcher settings (this crate)
//!     games/
//!         <id>/
//!             game.json    # per-game manifest (this crate)
//!             extracted/   # files extracted from the imported .CAB
//! ```
//!
//! `<library_root>` is platform-specific:
//!
//! * Linux/Windows: a path supplied by the desktop frontend, typically
//!   the user's `Documents/PocketHLE` folder.
//! * Android: `Context.getExternalFilesDir(null)` or any other path the
//!   Java side hands across JNI.
//!
//! The crate is designed to be `Send + Sync` so the Android side can
//! call into it from any thread without locking.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors returned by [`Library`] operations.
#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("cab error: {0}")]
    Cab(#[from] pocket_cab::CabError),
    #[error("pe error: {0}")]
    Pe(String),
    #[error("game with id `{0}` not found")]
    NotFound(String),
    #[error("the cabinet does not contain any ARM PE32 executable")]
    NoExecutable,
    #[error("invalid game id `{0}`")]
    InvalidId(String),
}

fn default_schema_version() -> u32 {
    1
}

/// One installed game entry. Stored both inline in `library.json`
/// (for fast listing) and as a separate `game.json` inside the game
/// directory (for per-game settings and so the directory is
/// self-describing if a user copies it around).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEntry {
    /// Stable identifier, derived from the source cab name. Used as
    /// the directory name. Always a-z 0-9 _ - .
    pub id: String,
    /// Human-readable name shown in the launcher.
    pub display_name: String,
    /// Optional one-line subtitle (provider / publisher).
    #[serde(default)]
    pub provider: Option<String>,
    /// Path to the main ARM `.exe` inside the extracted directory.
    /// Stored relative to `<library_root>/games/<id>/`.
    pub executable: PathBuf,
    /// Source cab basename, kept for display purposes.
    pub source_cab: String,
    /// Best-effort UNIX timestamp of when the game was imported.
    #[serde(default)]
    pub imported_at: i64,
    /// Per-game runtime settings.
    #[serde(default)]
    pub settings: GameSettings,
}

impl GameEntry {
    /// Path to this game's directory, relative to the library root.
    pub fn relative_dir(&self) -> PathBuf {
        PathBuf::from("games").join(&self.id)
    }

    /// Absolute path to the directory holding the extracted cab.
    pub fn extracted_dir(&self, library_root: &Path) -> PathBuf {
        library_root.join(self.relative_dir()).join("extracted")
    }

    /// Absolute path to the main executable.
    pub fn executable_path(&self, library_root: &Path) -> PathBuf {
        library_root
            .join(self.relative_dir())
            .join(&self.executable)
    }
}

/// Runtime settings stored per game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameSettings {
    /// Which CPU backend the user prefers for this game.
    #[serde(default)]
    pub cpu_backend: CpuBackendPref,
    /// Maximum number of host-resumed slices per run.
    #[serde(default = "default_max_slices")]
    pub max_slices: u64,
    /// Instructions per slice budget passed to the CPU.
    #[serde(default = "default_instructions_per_slice")]
    pub instructions_per_slice: u64,
    /// If true, the run loop halts as soon as an unimplemented API
    /// is encountered (great for debugging).
    #[serde(default)]
    pub halt_on_unimplemented: bool,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            cpu_backend: CpuBackendPref::default(),
            max_slices: default_max_slices(),
            instructions_per_slice: default_instructions_per_slice(),
            halt_on_unimplemented: false,
        }
    }
}

fn default_max_slices() -> u64 {
    // Real PPC2003 games typically need a few hundred thousand
    // slices to finish their CRT init / soft-float lookup tables /
    // bitmap loading before the first WM_PAINT is delivered, and
    // millions more to clear the splash and reach gameplay.
    // 1024 was effectively a smoke test, not a game launcher: a
    // freshly imported game timed out long before the title
    // screen and looked frozen in the GUI. 50 million is enough
    // to land on the JumpyBall main menu in roughly ten seconds
    // on a modern x86 machine.
    50_000_000
}

fn default_instructions_per_slice() -> u64 {
    1_000_000
}

/// User preference for the CPU backend. `Unicorn` is the only backend
/// that actually executes ARM code — `Stub` is trace-only and cannot
/// run a real game. Both frontends default to `Unicorn` and only fall
/// back to `Stub` when the user explicitly picks it (e.g. for API
/// tracing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CpuBackendPref {
    Stub,
    #[default]
    Unicorn,
}

impl CpuBackendPref {
    pub fn label(self) -> &'static str {
        match self {
            CpuBackendPref::Stub => "Stub (trace-only)",
            CpuBackendPref::Unicorn => "Unicorn (ARM)",
        }
    }
}

/// Persistent global configuration. Lives at
/// `<library_root>/config.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Default CPU backend used when importing a new game.
    #[serde(default)]
    pub default_cpu_backend: CpuBackendPref,
    /// Default verbosity level (0..=3).
    #[serde(default)]
    pub verbosity: u8,
    /// Last folder the user picked a `.cab` from. Used to remember
    /// the file dialog start directory.
    #[serde(default)]
    pub last_import_dir: Option<PathBuf>,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            default_cpu_backend: CpuBackendPref::default(),
            verbosity: 1,
            last_import_dir: None,
        }
    }
}

/// Top-level handle to an on-disk PocketHLE library.
///
/// Cheap to clone: the only state is the root path and the in-memory
/// registry; mutations are written through to disk immediately.
#[derive(Debug, Clone)]
pub struct Library {
    root: PathBuf,
    library: LibraryFile,
    config: LauncherConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LibraryFile {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default)]
    games: Vec<GameEntry>,
}

impl Library {
    /// Open the library rooted at `root`, creating the directory and
    /// default `library.json` / `config.json` if they don't exist.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, LibraryError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join("games"))?;

        let library = read_or_default::<LibraryFile>(&root.join("library.json"))?;
        let config = read_or_default::<LauncherConfig>(&root.join("config.json"))?;
        Ok(Self {
            root,
            library,
            config,
        })
    }

    /// Library root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// All known games, sorted by display name.
    pub fn games(&self) -> &[GameEntry] {
        &self.library.games
    }

    /// Look up one game by id.
    pub fn get(&self, id: &str) -> Option<&GameEntry> {
        self.library.games.iter().find(|g| g.id == id)
    }

    /// Mutable access to one game by id.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut GameEntry> {
        self.library.games.iter_mut().find(|g| g.id == id)
    }

    /// Persist the current `library.json` and `config.json` to disk.
    pub fn save(&self) -> Result<(), LibraryError> {
        write_json(&self.root.join("library.json"), &self.library)?;
        write_json(&self.root.join("config.json"), &self.config)?;
        Ok(())
    }

    /// Read-only view of the global launcher config.
    pub fn config(&self) -> &LauncherConfig {
        &self.config
    }

    /// Mutable view of the global launcher config. The caller is
    /// responsible for calling [`Library::save`] when finished.
    pub fn config_mut(&mut self) -> &mut LauncherConfig {
        &mut self.config
    }

    /// Import a Pocket PC `.CAB` into the library.
    ///
    /// Returns the freshly created [`GameEntry`]. The cab is extracted
    /// into `<library_root>/games/<id>/extracted/`, where `<id>` is
    /// derived from the source cab's filename. Existing entries with
    /// the same id are replaced (the directory is wiped first).
    pub fn import_cab(&mut self, cab_path: impl AsRef<Path>) -> Result<&GameEntry, LibraryError> {
        let cab_path = cab_path.as_ref();
        let source_cab = cab_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown.cab".to_string());
        let id = sanitize_id(cab_path.file_stem().map(|s| s.to_string_lossy()).as_deref());
        if id.is_empty() {
            return Err(LibraryError::InvalidId(source_cab));
        }
        let game_dir = self.root.join("games").join(&id);
        if game_dir.exists() {
            fs::remove_dir_all(&game_dir)?;
        }
        let extracted_dir = game_dir.join("extracted");
        fs::create_dir_all(&extracted_dir)?;

        let (files, header) = pocket_cab::extract_with_header(cab_path, &extracted_dir)?;
        // Pick the largest ARM PE32 executable as the entry point.
        let mut best: Option<(PathBuf, u64)> = None;
        for f in &files {
            let lower = f.short_name.to_ascii_lowercase();
            // Skip obvious non-executables. WinCE installer files like
            // `.000` headers must never be treated as the game .exe.
            if lower.ends_with(".000") || lower.ends_with(".dll") {
                continue;
            }
            // Cheap PE check — read the first bytes and look for "MZ".
            if !is_pe_file(&f.extracted_path) {
                continue;
            }
            if best.as_ref().map(|(_, sz)| *sz).unwrap_or(0) < f.size {
                best = Some((f.extracted_path.clone(), f.size));
            }
        }
        let (exe_abs, _) = best.ok_or(LibraryError::NoExecutable)?;
        let executable = exe_abs
            .strip_prefix(&game_dir)
            .map(|p| p.to_path_buf())
            .unwrap_or(exe_abs.clone());

        let display_name = header
            .as_ref()
            .and_then(|h| h.app_name.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| pretty_id(&id));
        let provider = header.as_ref().and_then(|h| h.provider.clone());

        let entry = GameEntry {
            id: id.clone(),
            display_name,
            provider,
            executable,
            source_cab,
            imported_at: now_unix_seconds(),
            settings: GameSettings {
                cpu_backend: self.config.default_cpu_backend,
                ..GameSettings::default()
            },
        };

        // Persist a per-game manifest so the game directory is
        // self-describing.
        write_json(&game_dir.join("game.json"), &entry)?;

        // Replace any existing entry with the same id.
        self.library.games.retain(|g| g.id != id);
        self.library.games.push(entry);
        self.library
            .games
            .sort_by(|a, b| a.display_name.cmp(&b.display_name));
        self.save()?;

        // Return a stable reference.
        Ok(self
            .library
            .games
            .iter()
            .find(|g| g.id == id)
            .expect("just inserted"))
    }

    /// Remove a game and its on-disk files.
    pub fn remove(&mut self, id: &str) -> Result<(), LibraryError> {
        let game_dir = self.root.join("games").join(id);
        if game_dir.exists() {
            fs::remove_dir_all(&game_dir)?;
        }
        self.library.games.retain(|g| g.id != id);
        self.save()
    }

    /// Update the per-game settings and save the library.
    pub fn update_settings(
        &mut self,
        id: &str,
        settings: GameSettings,
    ) -> Result<(), LibraryError> {
        let game = self
            .get_mut(id)
            .ok_or_else(|| LibraryError::NotFound(id.to_string()))?;
        game.settings = settings;
        let cloned = game.clone();
        let game_dir = self.root.join("games").join(id);
        write_json(&game_dir.join("game.json"), &cloned)?;
        self.save()
    }
}

fn read_or_default<T>(path: &Path) -> Result<T, LibraryError>
where
    T: Default + serde::de::DeserializeOwned,
{
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<T>(&bytes) {
            Ok(v) => Ok(v),
            Err(e) => {
                log::warn!(
                    "could not parse {}: {e}; falling back to default",
                    path.display()
                );
                Ok(T::default())
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(LibraryError::Io(e)),
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), LibraryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(path, bytes)?;
    Ok(())
}

fn sanitize_id(stem: Option<&str>) -> String {
    let raw = stem.unwrap_or("game");
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '_' | '-' | '.') {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('.').trim_matches('_').trim_matches('-');
    if trimmed.is_empty() {
        "game".to_string()
    } else {
        trimmed.to_string()
    }
}

fn pretty_id(id: &str) -> String {
    let cleaned = id.replace(['_', '-', '.'], " ");
    let mut out = String::with_capacity(cleaned.len());
    let mut new_word = true;
    for ch in cleaned.chars() {
        if ch == ' ' {
            new_word = true;
            out.push(' ');
        } else if new_word {
            out.extend(ch.to_uppercase());
            new_word = false;
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

fn is_pe_file(path: &Path) -> bool {
    let mut head = [0u8; 2];
    match fs::File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)) {
        Ok(()) => &head == b"MZ",
        Err(_) => false,
    }
}

fn now_unix_seconds() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmpdir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "pockethle-library-test-{}-{}",
            name,
            now_unix_seconds()
        ));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn open_creates_layout() {
        let root = tmpdir("layout");
        let _lib = Library::open(&root).unwrap();
        assert!(root.join("games").is_dir());
        assert!(!root.join("library.json").exists() || root.join("library.json").is_file());
    }

    #[test]
    fn save_and_reload_round_trips() {
        let root = tmpdir("roundtrip");
        let mut lib = Library::open(&root).unwrap();
        lib.config_mut().verbosity = 2;
        lib.save().unwrap();

        let lib2 = Library::open(&root).unwrap();
        assert_eq!(lib2.config().verbosity, 2);
    }

    #[test]
    fn sanitize_id_strips_garbage() {
        assert_eq!(sanitize_id(Some("JumpyBall PPC")), "jumpyball_ppc");
        assert_eq!(sanitize_id(Some("../../etc/passwd")), "etcpasswd");
        assert_eq!(sanitize_id(Some("")), "game");
        assert_eq!(sanitize_id(None), "game");
    }

    #[test]
    fn pretty_id_titlecases_words() {
        assert_eq!(pretty_id("jumpy_ball"), "Jumpy Ball");
        assert_eq!(pretty_id("foo-bar.baz"), "Foo Bar Baz");
    }
}
