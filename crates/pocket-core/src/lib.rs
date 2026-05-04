//! Top-level emulator object that the frontends drive.
//!
//! Usage:
//!
//! ```no_run
//! use pocket_core::Emulator;
//! let mut emu = Emulator::with_stub_cpu();
//! emu.load_pe("/path/to/JumpyBallPPC.exe").unwrap();
//! emu.run().unwrap();
//! ```
//!
//! When compiled with `--features unicorn`, [`Emulator::with_unicorn_cpu`]
//! provides a fully working ARM backend.

use std::path::Path;

use anyhow::{Context, Result};

use pocket_cpu::{stub::StubCpu, Cpu};
use pocket_kernel::{run_main_loop, Process};
use pocket_winceapi::{resolve_ordinal, WinCeDispatcher};

pub use pocket_cab as cab;
pub use pocket_cpu as cpu;
pub use pocket_kernel as kernel;
pub use pocket_pe as pe;
pub use pocket_winceapi as winceapi;

pub struct Emulator {
    cpu: Box<dyn Cpu>,
    process: Option<Process>,
    dispatcher: WinCeDispatcher,
    pub instruction_budget_per_slice: u64,
    pub max_slices: u64,
}

impl Emulator {
    pub fn with_stub_cpu() -> Self {
        Self {
            cpu: Box::new(StubCpu::new()),
            process: None,
            dispatcher: WinCeDispatcher::new(),
            instruction_budget_per_slice: 1_000_000,
            max_slices: 1024,
        }
    }

    /// Build with the unicorn-engine backed CPU. Requires the
    /// `unicorn` Cargo feature.
    #[cfg(feature = "unicorn")]
    pub fn with_unicorn_cpu() -> Result<Self> {
        let cpu = pocket_cpu::unicorn::UnicornCpu::new()
            .context("creating unicorn-engine CPU instance")?;
        Ok(Self {
            cpu: Box::new(cpu),
            process: None,
            dispatcher: WinCeDispatcher::new(),
            instruction_budget_per_slice: 1_000_000,
            max_slices: 1024,
        })
    }

    /// Halt the emulator the first time an unimplemented API is hit.
    /// Useful for the tracing CLI mode.
    pub fn set_halt_on_unimplemented(&mut self, halt: bool) {
        self.dispatcher.halt_on_unimplemented = halt;
    }

    /// Forward every dispatched API call as JSON-lines to `sink`.
    pub fn set_trace_sink(&mut self, sink: Box<dyn std::io::Write + Send>) {
        self.dispatcher.set_trace_sink(sink);
    }

    /// Load and map a PE file into the emulator. Existing process
    /// state is replaced.
    pub fn load_pe(&mut self, path: impl AsRef<Path>) -> Result<&Process> {
        let image = pe::load_file(path).context("loading PE")?;
        let process = Process::map_into(image, self.cpu.as_mut(), &|dll, ord| {
            resolve_ordinal(dll, ord)
        })
        .context("mapping image into CPU")?;
        self.process = Some(process);
        Ok(self.process.as_ref().unwrap())
    }

    /// Run until the emulator halts. Returns the number of slices
    /// consumed.
    pub fn run(&mut self) -> Result<()> {
        let process = self
            .process
            .as_ref()
            .context("no PE loaded — call load_pe() first")?;
        run_main_loop(
            self.cpu.as_mut(),
            process,
            &mut self.dispatcher,
            self.instruction_budget_per_slice,
            self.max_slices,
        )
        .context("main emulator loop")
    }

    pub fn process(&self) -> Option<&Process> {
        self.process.as_ref()
    }

    pub fn dispatcher(&self) -> &WinCeDispatcher {
        &self.dispatcher
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_emulator_constructs() {
        let _ = Emulator::with_stub_cpu();
    }
}
