//! High-level emulation of WinCE / Windows Mobile system DLLs.
//!
//! Each emulated DLL has its own submodule:
//!
//! * [`coredll`] — the catch-all kernel/runtime DLL. Imported almost
//!   exclusively by ordinal, so we ship a JSON ordinal map (see
//!   `data/coredll-ordinals.json`).
//! * [`aygshell`] — Pocket PC shell extensions (`SHFullScreen`,
//!   `SHCreateMenuBar`).
//! * [`gx`] — GAPI (Game API) for direct framebuffer access.
//! * [`hss`] — Hekkus Sound System (popular freeware audio engine
//!   bundled with many Pocket PC games).
//!
//! All four are dispatched through a single [`WinCeDispatcher`] that
//! implements [`pocket_kernel::Dispatcher`].

pub mod aygshell;
pub mod coredll;
pub mod gx;
pub mod hss;
pub mod ordinals;

use std::collections::HashMap;

use pocket_cpu::Cpu;
use pocket_kernel::{DispatchOutcome, Dispatcher, KernelError, Thunk};
use pocket_pe::ImportBinding;

/// Convert an ordinal-only import to a friendly name where possible.
pub fn resolve_ordinal(dll: &str, ordinal: u16) -> Option<String> {
    ordinals::lookup(dll, ordinal)
}

/// Per-call context passed to handler functions.
pub struct CallCtx<'a> {
    pub cpu: &'a mut dyn Cpu,
    pub thunk: &'a Thunk,
}

impl<'a> CallCtx<'a> {
    pub fn arg_u32(&mut self, idx: u8) -> Result<u32, KernelError> {
        use pocket_cpu::regs::ArmReg::*;
        let reg = match idx {
            0 => R0,
            1 => R1,
            2 => R2,
            3 => R3,
            _ => {
                // Fetch from the stack at [sp + (idx-4)*4].
                let sp = self.cpu.read_reg(pocket_cpu::regs::ArmReg::Sp)?;
                let off = sp + (idx - 4) as u32 * 4;
                let bytes = self.cpu.read_mem(off, 4)?;
                return Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
            }
        };
        Ok(self.cpu.read_reg(reg)?)
    }
}

/// Function pointer for a host-side handler.
pub type Handler = fn(&mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError>;

/// Top-level dispatcher that owns per-DLL handler tables.
pub struct WinCeDispatcher {
    /// Key is `(dll_lowercased, friendly_name)`.
    by_name: HashMap<(String, String), Handler>,
    /// Tracks how many times each thunk has been called for stats.
    call_counts: HashMap<u32, u64>,
    /// If `true`, an unimplemented call halts the emulator instead of
    /// returning 0. Useful for the Linux CLI tracing run.
    pub halt_on_unimplemented: bool,
}

impl Default for WinCeDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl WinCeDispatcher {
    pub fn new() -> Self {
        let mut d = Self {
            by_name: HashMap::new(),
            call_counts: HashMap::new(),
            halt_on_unimplemented: false,
        };
        coredll::register(&mut d);
        aygshell::register(&mut d);
        gx::register(&mut d);
        hss::register(&mut d);
        d
    }

    pub fn register_handler(&mut self, dll: &str, name: &str, handler: Handler) {
        self.by_name
            .insert((dll.to_ascii_lowercase(), name.to_string()), handler);
    }

    pub fn registered_count(&self) -> usize {
        self.by_name.len()
    }

    /// Iterate every (dll, name) pair currently registered.
    pub fn registered_iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.by_name.keys().map(|(d, n)| (d.as_str(), n.as_str()))
    }
}

impl Dispatcher for WinCeDispatcher {
    fn dispatch(
        &mut self,
        cpu: &mut dyn Cpu,
        thunk: &Thunk,
    ) -> Result<DispatchOutcome, KernelError> {
        *self.call_counts.entry(thunk.thunk_va).or_default() += 1;
        let dll_key = thunk.dll.to_ascii_lowercase();
        let name = match (&thunk.binding, &thunk.friendly_name) {
            (_, Some(n)) => n.clone(),
            (ImportBinding::Name(n), _) => n.clone(),
            (ImportBinding::Ordinal(o), _) => format!("ord:{o}"),
        };
        let key = (dll_key, name.clone());
        if let Some(handler) = self.by_name.get(&key) {
            log::trace!("call {}", thunk.label());
            let mut ctx = CallCtx { cpu, thunk };
            handler(&mut ctx)
        } else {
            log::warn!("unimplemented call -> {}", thunk.label());
            if self.halt_on_unimplemented {
                Ok(DispatchOutcome::Halt)
            } else {
                Ok(DispatchOutcome::Unimplemented)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_built_in_handlers() {
        let d = WinCeDispatcher::new();
        assert!(d.registered_count() > 0);
    }
}
