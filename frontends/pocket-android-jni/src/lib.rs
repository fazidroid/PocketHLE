//! JNI bridge between the PocketHLE core/library and the Android
//! frontend. The Java side talks to this crate exclusively through
//! UTF-8 JSON strings — that keeps the FFI surface small and removes
//! any need for shared Kotlin/Rust data classes.
//!
//! Exposed methods (all on `com.pockethle.app.NativeBridge`):
//!
//! * `banner()` — sanity string showing the loaded version.
//! * `listGames(libraryRoot)` — JSON array of [`pocket_library::GameEntry`].
//! * `importCab(libraryRoot, cabPath)` — JSON of the freshly-imported entry.
//! * `removeGame(libraryRoot, id)` — `"ok"` or `"err: ..."`.
//! * `readConfig(libraryRoot)` — JSON of [`pocket_library::LauncherConfig`].
//! * `writeConfig(libraryRoot, json)` — overwrite config with a JSON blob.
//! * `readGameSettings(libraryRoot, id)` — JSON of per-game settings.
//! * `writeGameSettings(libraryRoot, id, json)` — persist per-game settings.
//! * `runGame(libraryRoot, id)` — emulate one round, returns JSON
//!   `{summary, frame: {width, height, rgba_b64}|null}`.

use std::path::Path;
use std::path::PathBuf;

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

use pocket_core::Emulator;
use pocket_library::{CpuBackendPref, GameEntry, GameSettings, Library};
use serde::Serialize;

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_NativeBridge_banner<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    init_logger();
    let banner = format!("PocketHLE v{} (Android)", env!("CARGO_PKG_VERSION"));
    new_jstring(&env, banner)
}

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_MainActivity_banner<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    init_logger();
    new_jstring(
        &env,
        format!("PocketHLE v{} (Android)", env!("CARGO_PKG_VERSION")),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_NativeBridge_listGames<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    library_root: JString<'local>,
) -> jstring {
    init_logger();
    let root = match jstring_to_path(&mut env, library_root) {
        Some(p) => p,
        None => return new_jstring(&env, error_json("missing library root")),
    };
    let result = (|| -> anyhow::Result<String> {
        let lib = Library::open(&root)?;
        let games: Vec<&GameEntry> = lib.games().iter().collect();
        Ok(serde_json::to_string(&games)?)
    })();
    match result {
        Ok(j) => new_jstring(&env, j),
        Err(e) => new_jstring(&env, error_json(&format!("listGames: {e:#}"))),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_NativeBridge_importCab<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    library_root: JString<'local>,
    cab_path: JString<'local>,
) -> jstring {
    init_logger();
    let root = match jstring_to_path(&mut env, library_root) {
        Some(p) => p,
        None => return new_jstring(&env, error_json("missing library root")),
    };
    let cab = match jstring_to_path(&mut env, cab_path) {
        Some(p) => p,
        None => return new_jstring(&env, error_json("missing cab path")),
    };
    let result = (|| -> anyhow::Result<String> {
        let mut lib = Library::open(&root)?;
        let entry = lib.import_cab(&cab)?;
        Ok(serde_json::to_string(&entry)?)
    })();
    match result {
        Ok(j) => new_jstring(&env, j),
        Err(e) => new_jstring(&env, error_json(&format!("importCab: {e:#}"))),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_NativeBridge_removeGame<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    library_root: JString<'local>,
    id: JString<'local>,
) -> jstring {
    init_logger();
    let root = match jstring_to_path(&mut env, library_root) {
        Some(p) => p,
        None => return new_jstring(&env, error_json("missing library root")),
    };
    let id = match jstring_to_string(&mut env, id) {
        Some(s) => s,
        None => return new_jstring(&env, error_json("missing game id")),
    };
    let result = (|| -> anyhow::Result<()> {
        let mut lib = Library::open(&root)?;
        lib.remove(&id)?;
        Ok(())
    })();
    match result {
        Ok(()) => new_jstring(&env, "{\"ok\":true}"),
        Err(e) => new_jstring(&env, error_json(&format!("removeGame: {e:#}"))),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_NativeBridge_readConfig<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    library_root: JString<'local>,
) -> jstring {
    init_logger();
    let root = match jstring_to_path(&mut env, library_root) {
        Some(p) => p,
        None => return new_jstring(&env, error_json("missing library root")),
    };
    let result = (|| -> anyhow::Result<String> {
        let lib = Library::open(&root)?;
        Ok(serde_json::to_string(lib.config())?)
    })();
    match result {
        Ok(j) => new_jstring(&env, j),
        Err(e) => new_jstring(&env, error_json(&format!("readConfig: {e:#}"))),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_NativeBridge_writeConfig<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    library_root: JString<'local>,
    config_json: JString<'local>,
) -> jstring {
    init_logger();
    let root = match jstring_to_path(&mut env, library_root) {
        Some(p) => p,
        None => return new_jstring(&env, error_json("missing library root")),
    };
    let body = match jstring_to_string(&mut env, config_json) {
        Some(s) => s,
        None => return new_jstring(&env, error_json("missing config json")),
    };
    let result = (|| -> anyhow::Result<()> {
        let mut lib = Library::open(&root)?;
        let cfg = serde_json::from_str(&body)?;
        *lib.config_mut() = cfg;
        lib.save()?;
        Ok(())
    })();
    match result {
        Ok(()) => new_jstring(&env, "{\"ok\":true}"),
        Err(e) => new_jstring(&env, error_json(&format!("writeConfig: {e:#}"))),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_NativeBridge_readGameSettings<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    library_root: JString<'local>,
    id: JString<'local>,
) -> jstring {
    init_logger();
    let root = match jstring_to_path(&mut env, library_root) {
        Some(p) => p,
        None => return new_jstring(&env, error_json("missing library root")),
    };
    let id = match jstring_to_string(&mut env, id) {
        Some(s) => s,
        None => return new_jstring(&env, error_json("missing game id")),
    };
    let result = (|| -> anyhow::Result<String> {
        let lib = Library::open(&root)?;
        let entry = lib
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown game"))?;
        Ok(serde_json::to_string(&entry.settings)?)
    })();
    match result {
        Ok(j) => new_jstring(&env, j),
        Err(e) => new_jstring(&env, error_json(&format!("readGameSettings: {e:#}"))),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_NativeBridge_writeGameSettings<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    library_root: JString<'local>,
    id: JString<'local>,
    settings_json: JString<'local>,
) -> jstring {
    init_logger();
    let root = match jstring_to_path(&mut env, library_root) {
        Some(p) => p,
        None => return new_jstring(&env, error_json("missing library root")),
    };
    let id = match jstring_to_string(&mut env, id) {
        Some(s) => s,
        None => return new_jstring(&env, error_json("missing game id")),
    };
    let body = match jstring_to_string(&mut env, settings_json) {
        Some(s) => s,
        None => return new_jstring(&env, error_json("missing settings json")),
    };
    let result = (|| -> anyhow::Result<()> {
        let mut lib = Library::open(&root)?;
        let new_settings: GameSettings = serde_json::from_str(&body)?;
        lib.update_settings(&id, new_settings)?;
        Ok(())
    })();
    match result {
        Ok(()) => new_jstring(&env, "{\"ok\":true}"),
        Err(e) => new_jstring(&env, error_json(&format!("writeGameSettings: {e:#}"))),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_NativeBridge_runGame<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    library_root: JString<'local>,
    id: JString<'local>,
) -> jstring {
    init_logger();
    let root = match jstring_to_path(&mut env, library_root) {
        Some(p) => p,
        None => return new_jstring(&env, error_json("missing library root")),
    };
    let id = match jstring_to_string(&mut env, id) {
        Some(s) => s,
        None => return new_jstring(&env, error_json("missing game id")),
    };
    let outcome = run_game(&root, &id);
    let payload = serde_json::to_string(&outcome)
        .unwrap_or_else(|e| format!("{{\"ok\":false,\"error\":\"json: {e}\"}}"));
    new_jstring(&env, payload)
}

#[derive(Serialize)]
struct RunOutcomeJson {
    ok: bool,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<FrameJson>,
}

#[derive(Serialize)]
struct FrameJson {
    width: u32,
    height: u32,
    /// RGBA8888 bytes encoded as base64 (standard alphabet, padded).
    rgba_b64: String,
}

fn run_game(root: &Path, id: &str) -> RunOutcomeJson {
    let result = (|| -> anyhow::Result<RunOutcomeJson> {
        let lib = Library::open(root)?;
        let entry = lib.get(id).ok_or_else(|| anyhow::anyhow!("unknown game"))?;
        let mut summary_lines = vec![
            format!("Game: {}", entry.display_name),
            format!("Backend: {}", entry.settings.cpu_backend.label()),
        ];
        let exe = entry.executable_path(root);
        summary_lines.push(format!("Executable: {}", exe.display()));
        let mut emu = match entry.settings.cpu_backend {
            CpuBackendPref::Stub => Emulator::with_stub_cpu(),
            CpuBackendPref::Unicorn => match build_unicorn() {
                Ok(emu) => emu,
                Err(e) => {
                    summary_lines.push(format!("Unicorn unavailable, falling back to stub: {e}"));
                    Emulator::with_stub_cpu()
                }
            },
        };
        emu.set_halt_on_unimplemented(entry.settings.halt_on_unimplemented);
        emu.max_slices = entry.settings.max_slices;
        emu.instruction_budget_per_slice = entry.settings.instructions_per_slice;
        emu.load_pe(&exe)?;
        emu.mount_dir("\\Application\\", entry.extracted_dir(root));
        let run_status = match emu.run() {
            Ok(()) => "Emulator exited cleanly.".to_string(),
            Err(e) => format!("Emulator stopped: {e:#}"),
        };
        summary_lines.push(run_status);
        let frame = emu.process().map(|p| {
            let fb = &p.state.framebuffer;
            FrameJson {
                width: fb.width,
                height: fb.height,
                rgba_b64: base64_encode(&fb.snapshot_rgba8888()),
            }
        });
        Ok(RunOutcomeJson {
            ok: true,
            summary: summary_lines.join("\n"),
            error: None,
            frame,
        })
    })();
    match result {
        Ok(j) => j,
        Err(e) => RunOutcomeJson {
            ok: false,
            summary: "Run failed before emulator started".to_string(),
            error: Some(format!("{e:#}")),
            frame: None,
        },
    }
}

#[cfg(feature = "unicorn")]
fn build_unicorn() -> anyhow::Result<Emulator> {
    Emulator::with_unicorn_cpu()
}

#[cfg(not(feature = "unicorn"))]
fn build_unicorn() -> anyhow::Result<Emulator> {
    Err(anyhow::anyhow!(
        "binary was not compiled with the `unicorn` feature"
    ))
}

fn jstring_to_string<'a>(env: &mut JNIEnv<'a>, value: JString<'a>) -> Option<String> {
    if value.is_null() {
        return None;
    }
    match env.get_string(&value) {
        Ok(s) => Some(s.into()),
        Err(e) => {
            log::error!("could not read jstring: {e}");
            None
        }
    }
}

fn jstring_to_path<'a>(env: &mut JNIEnv<'a>, value: JString<'a>) -> Option<PathBuf> {
    jstring_to_string(env, value).map(PathBuf::from)
}

fn new_jstring(env: &JNIEnv<'_>, body: impl AsRef<str>) -> jstring {
    match env.new_string(body.as_ref()) {
        Ok(s) => s.into_raw(),
        Err(e) => {
            log::error!("could not allocate JString: {e}");
            std::ptr::null_mut()
        }
    }
}

fn error_json(msg: &str) -> String {
    let escaped = msg
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("{{\"ok\":false,\"error\":\"{escaped}\"}}")
}

const B64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Tiny standalone base64 encoder so we don't need to add a `base64`
/// dep just to ship the framebuffer over JNI.
fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | input[i + 2] as u32;
        out.push(B64_TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(B64_TABLE[((n >> 6) & 0x3f) as usize] as char);
        out.push(B64_TABLE[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(B64_TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(B64_TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(B64_TABLE[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

fn init_logger() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("PocketHLE"),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_examples() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
}
