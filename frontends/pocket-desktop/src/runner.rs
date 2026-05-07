//! Background runner that drives [`pocket_core::Emulator`] for the
//! desktop GUI.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use pocket_core::Emulator;
use pocket_library::{CpuBackendPref, GameEntry};

#[derive(Debug, Clone, Default)]
pub struct Runner {
    inner: Arc<Mutex<()>>,
}

impl Runner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn run_game(&self, library_root: PathBuf, game: GameEntry) -> RunOutcome {
        let _guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let exe = game.executable_path(&library_root);
        let mut summary_lines = vec![format!("Game: {}", game.display_name)];
        summary_lines.push(format!("Backend: {}", game.settings.cpu_backend.label()));
        summary_lines.push(format!("Executable: {}", exe.display()));

        let mut emu = match game.settings.cpu_backend {
            CpuBackendPref::Stub => Emulator::with_stub_cpu(),
            CpuBackendPref::Unicorn => match build_unicorn() {
                Ok(emu) => emu,
                Err(e) => {
                    summary_lines.push(format!("Unicorn unavailable, falling back to stub: {e}"));
                    Emulator::with_stub_cpu()
                }
            },
        };
        emu.set_halt_on_unimplemented(game.settings.halt_on_unimplemented);
        emu.max_slices = game.settings.max_slices;
        emu.instruction_budget_per_slice = game.settings.instructions_per_slice;

        if let Err(e) = emu.load_pe(&exe) {
            summary_lines.push(format!("load_pe failed: {e:#}"));
            return RunOutcome {
                summary: summary_lines.join("\n"),
                framebuffer: None,
            };
        }

        let extracted = game.extracted_dir(&library_root);
        emu.mount_dir("\\Application\\", &extracted);

        match emu.run() {
            Ok(()) => summary_lines.push("Emulator exited cleanly.".to_string()),
            Err(e) => summary_lines.push(format!("Emulator stopped: {e:#}")),
        }

        let framebuffer = emu
            .process()
            .map(|p| FrameSnapshot::from_framebuffer(&p.state.framebuffer));
        RunOutcome {
            summary: summary_lines.join("\n"),
            framebuffer,
        }
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

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub summary: String,
    pub framebuffer: Option<FrameSnapshot>,
}

#[derive(Debug, Clone)]
pub struct FrameSnapshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl FrameSnapshot {
    fn from_framebuffer(fb: &pocket_core::kernel::Framebuffer) -> Self {
        Self {
            width: fb.width,
            height: fb.height,
            rgba: fb.snapshot_rgba8888(),
        }
    }
}
