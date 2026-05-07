//! Background runner that drives [`pocket_core::Emulator`] for the
//! desktop GUI.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::sync::Mutex;

use pocket_core::kernel::{FrameAction, FrameHook, InputEvent, KernelState};
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

    pub fn run_game(
        &self,
        library_root: PathBuf,
        game: GameEntry,
        live_tx: Option<Sender<FrameSnapshot>>,
        input_rx: Option<Receiver<InputCommand>>,
    ) -> RunOutcome {
        let _guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let exe = game.executable_path(&library_root);
        let mut summary_lines = vec![format!("Game: {}", game.display_name)];

        // The Stub CPU does not interpret instructions — it is a
        // trace-only harness that exists so loader-level code can be
        // unit-tested without pulling in unicorn-engine. Trying to
        // actually `run` a game on it is guaranteed to crash with
        // "guest jumped to unmapped address 0x00000000" because the
        // stub never sets LR while pretending to call a guest
        // function. End users who click "Run" in the GUI always
        // want the real ARM core, regardless of what is persisted in
        // their `library.json` — so when a Stub-backed game is
        // launched and unicorn is compiled in, we silently promote
        // the run to Unicorn. This is defense-in-depth on top of
        // [`pocket_library::Library::migrate_legacy_entries`], which
        // covers users who downgrade or who have a stale
        // `library.json` from before that migration existed.
        let requested_backend = game.settings.cpu_backend;
        let mut effective_backend = requested_backend;
        let mut emu = match requested_backend {
            CpuBackendPref::Unicorn => match build_unicorn() {
                Ok(emu) => emu,
                Err(e) => {
                    summary_lines.push(format!("Unicorn unavailable, falling back to stub: {e}"));
                    effective_backend = CpuBackendPref::Stub;
                    Emulator::with_stub_cpu()
                }
            },
            CpuBackendPref::Stub => match build_unicorn() {
                Ok(emu) => {
                    summary_lines.push(
                        "Saved CPU backend was Stub (trace-only); promoting to \
                         Unicorn so the game can actually execute."
                            .to_string(),
                    );
                    effective_backend = CpuBackendPref::Unicorn;
                    emu
                }
                Err(_) => Emulator::with_stub_cpu(),
            },
        };
        summary_lines.push(format!("Backend: {}", effective_backend.label()));
        summary_lines.push(format!("Executable: {}", exe.display()));

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
        // GUI users actually play the game, so don't auto-fire
        // `WM_QUIT` after a fixed number of synthetic messages —
        // budget=0 means "run until the user stops or the game
        // calls ExitProcess". Real input from the virtual D-pad /
        // stylus tap arrives through `input_rx` and feeds the
        // synthetic message pump in pocket-winceapi.
        emu.set_synthetic_message_budget(0);

        let run_result = {
            let mut hook = RunHook::new(live_tx, input_rx);
            emu.run_with_hook(&mut hook)
        };
        match run_result {
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

/// Command pushed by the GUI thread into the running emulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputCommand {
    /// User input (tap / D-pad / key) to forward to the guest.
    Input(InputEvent),
    /// "Back to library" button — ask the run loop to stop cleanly.
    Stop,
}

/// Frame hook that:
///  - pushes one [`FrameSnapshot`] across `frame_tx` every time the
///    guest produces a new frame, so the GUI paints a live preview;
///  - drains pending [`InputCommand`]s from `input_rx` and forwards
///    them into the kernel's input queue / stop flag.
struct RunHook {
    frame_tx: Option<Sender<FrameSnapshot>>,
    input_rx: Option<Receiver<InputCommand>>,
    last_frame: u64,
    frame_send_failed: bool,
    input_disconnected: bool,
}

impl RunHook {
    fn new(
        frame_tx: Option<Sender<FrameSnapshot>>,
        input_rx: Option<Receiver<InputCommand>>,
    ) -> Self {
        Self {
            frame_tx,
            input_rx,
            last_frame: 0,
            frame_send_failed: false,
            input_disconnected: false,
        }
    }
}

impl FrameHook for RunHook {
    fn on_frame(&mut self, state: &mut KernelState) -> FrameAction {
        // Forward any UI input the user has produced since the last
        // slice into the kernel's pending input queue. Stop signals
        // turn into a `FrameAction::Stop`.
        let mut stop_requested = false;
        if !self.input_disconnected {
            if let Some(rx) = self.input_rx.as_ref() {
                loop {
                    match rx.try_recv() {
                        Ok(InputCommand::Input(ev)) => state.pending_input.push_back(ev),
                        Ok(InputCommand::Stop) => stop_requested = true,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            self.input_disconnected = true;
                            break;
                        }
                    }
                }
            }
        }

        // Stream the latest framebuffer up to the GUI.
        if !self.frame_send_failed {
            if let Some(tx) = self.frame_tx.as_ref() {
                let counter = state.framebuffer.frame_counter;
                if counter != self.last_frame {
                    self.last_frame = counter;
                    let snapshot = FrameSnapshot::from_framebuffer(&state.framebuffer);
                    if tx.send(snapshot).is_err() {
                        // GUI thread dropped the receiver — the user
                        // closed the run screen / quit the launcher.
                        self.frame_send_failed = true;
                    }
                }
            }
        }

        if stop_requested {
            state.should_stop = true;
            FrameAction::Stop
        } else {
            FrameAction::Continue
        }
    }
}
