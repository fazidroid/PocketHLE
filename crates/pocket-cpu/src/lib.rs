//! CPU emulation abstractions.
//!
//! `pocket-cpu` exposes a backend-agnostic [`Cpu`] trait that the rest
//! of the emulator talks to. Two backends are provided:
//!
//! * [`stub`] — a no-op CPU that just stores register & memory state.
//!   Always built; lets the workspace compile without native deps and
//!   is enough for tests that only exercise the loader / kernel stubs.
//! * [`unicorn`] — wraps the [Unicorn](https://www.unicorn-engine.org/)
//!   emulator. Behind the `unicorn` Cargo feature because it requires
//!   cmake / a C compiler at build time.
//!
//! The trait is intentionally tiny — we only need ARM-mode features
//! relevant to user-mode PE32 execution under HLE.

use thiserror::Error;

pub mod stub;
#[cfg(feature = "unicorn")]
pub mod unicorn;

pub mod regs {
    /// ARM general-purpose register indices.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(u8)]
    pub enum ArmReg {
        R0 = 0,
        R1 = 1,
        R2 = 2,
        R3 = 3,
        R4 = 4,
        R5 = 5,
        R6 = 6,
        R7 = 7,
        R8 = 8,
        R9 = 9,
        R10 = 10,
        R11 = 11,
        R12 = 12,
        Sp = 13,
        Lr = 14,
        Pc = 15,
        Cpsr = 16,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    /// 32-bit ARM, little endian.
    Arm,
}

bitflags::bitflags! {
    /// Memory protection flags. Mirrors Unicorn / `mmap` semantics.
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub struct Prot: u32 {
        const NONE = 0;
        const READ = 1;
        const WRITE = 2;
        const EXEC = 4;
        const ALL = Self::READ.bits() | Self::WRITE.bits() | Self::EXEC.bits();
    }
}

#[derive(Debug, Error)]
pub enum CpuError {
    #[error("unsupported feature: {0}")]
    Unsupported(&'static str),
    #[error("invalid memory access at va=0x{va:08x} size={size}")]
    BadMemory { va: u32, size: u32 },
    #[error("execution stopped at va=0x{0:08x}")]
    Stopped(u32),
    #[error("backend error: {0}")]
    Backend(String),
}

/// Reason `run_until_hook` returned to the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Execution counter exhausted.
    InstructionLimit,
    /// One of the registered hooks fired.
    Hook(u32),
    /// `Cpu::request_stop` was called.
    Requested,
    /// The CPU executed beyond mapped memory.
    OutOfBounds,
}

/// Common interface to a CPU emulation backend.
pub trait Cpu {
    fn arch(&self) -> Arch;

    fn map_region(&mut self, va: u32, size: u32, prot: Prot) -> Result<(), CpuError>;
    fn write_mem(&mut self, va: u32, data: &[u8]) -> Result<(), CpuError>;
    fn read_mem(&mut self, va: u32, len: u32) -> Result<Vec<u8>, CpuError>;

    fn read_reg(&mut self, reg: regs::ArmReg) -> Result<u32, CpuError>;
    fn write_reg(&mut self, reg: regs::ArmReg, value: u32) -> Result<(), CpuError>;

    /// Register an executable address that should immediately stop
    /// emulation when the PC reaches it. Used to install IAT thunks
    /// for unimplemented imports.
    fn add_code_hook(&mut self, va: u32) -> Result<(), CpuError>;

    /// Run starting at `start_va` until either an [`StopReason::Hook`]
    /// fires, `max_instructions` is reached, or `request_stop` is
    /// called from another hook. Set `max_instructions` to `0` to run
    /// without a limit.
    fn run_until_hook(
        &mut self,
        start_va: u32,
        max_instructions: u64,
    ) -> Result<StopReason, CpuError>;

    fn request_stop(&mut self);
}

pub fn round_up_to_page(size: u32) -> u32 {
    const PAGE: u32 = 0x1000;
    (size + PAGE - 1) & !(PAGE - 1)
}
