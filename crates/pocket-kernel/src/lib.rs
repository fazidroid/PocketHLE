//! Kernel-side scaffolding: virtual address space, thunk allocator,
//! thread state, scheduling.
//!
//! In PocketHLE every emulated process owns a single 32-bit address
//! space. The kernel is responsible for:
//!
//! * Mapping the loaded PE image into the CPU.
//! * Allocating a contiguous "thunk" region — one 4-byte slot per
//!   imported symbol — and patching the IAT so that calls into a
//!   foreign DLL transfer control to a known address that the CPU
//!   has marked with a code hook. When the hook fires, the host
//!   dispatches the call through [`Dispatcher`].
//! * Maintaining a stack and minimal heap for the emulated thread.
//!
//! The kernel does **not** implement individual API functions — that
//! is the responsibility of `pocket-winceapi`. Instead, the kernel
//! exposes a [`Dispatcher`] trait that an API layer registers itself
//! against.

use std::collections::HashMap;

use byteorder::{ByteOrder, LittleEndian};
use indexmap::IndexMap;
use thiserror::Error;

use pocket_cpu::{dump_mem_around, dump_regs, regs::ArmReg, Cpu, CpuError, Prot, StopReason};
use pocket_pe::{ImportBinding, ImportSymbol, LoadedImage};

/// Default base address of the synthetic IAT thunk pool.
pub const THUNK_REGION_BASE: u32 = 0x7000_0000;
/// Each thunk is exactly one 32-bit instruction. We never execute it
/// — the CPU hook stops us first — but we still write a `bx lr` so
/// that an accidental fall-through returns rather than crashes.
pub const THUNK_STRIDE: u32 = 4;
/// Default stack size (256 KiB).
pub const DEFAULT_STACK_SIZE: u32 = 0x40000;
/// Default top of stack — chosen so that ARM-style descending stacks
/// stay below the thunk region.
pub const DEFAULT_STACK_TOP: u32 = 0x6000_0000;

/// "bx lr" in ARM mode (little endian).
pub const ARM_BX_LR: [u8; 4] = [0x1e, 0xff, 0x2f, 0xe1];

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("cpu error: {0}")]
    Cpu(#[from] CpuError),
    #[error("loader error: {0}")]
    Loader(String),
    #[error("dispatcher error: {0}")]
    Dispatch(String),
}

/// Result of dispatching a hooked call back to the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// Returned a value via R0; emulator should resume from LR.
    ReturnedR0(u32),
    /// Returned a 64-bit value via R0:R1.
    ReturnedR0R1(u32, u32),
    /// The host wants the emulator to stop entirely (graceful exit).
    Halt,
    /// The host has not implemented this API. PocketHLE will log a
    /// loud warning and synthesize a `0` return.
    Unimplemented,
}

/// Trait an API layer registers with the kernel. Called every time
/// emulated code reaches a thunk address.
pub trait Dispatcher {
    fn dispatch(
        &mut self,
        cpu: &mut dyn Cpu,
        thunk: &Thunk,
    ) -> Result<DispatchOutcome, KernelError>;
}

/// One IAT entry that has been resolved to a host-side stub.
#[derive(Debug, Clone)]
pub struct Thunk {
    pub thunk_va: u32,
    pub iat_va: u32,
    pub dll: String,
    pub binding: ImportBinding,
    /// Optional human-readable name used in logs (e.g. resolved from
    /// an ordinal map).
    pub friendly_name: Option<String>,
}

impl Thunk {
    pub fn label(&self) -> String {
        match (&self.binding, &self.friendly_name) {
            (_, Some(n)) => format!("{}!{}", self.dll, n),
            (ImportBinding::Name(n), _) => format!("{}!{}", self.dll, n),
            (ImportBinding::Ordinal(o), _) => format!("{}!#{}", self.dll, o),
        }
    }
}

/// The whole emulated process state owned by the kernel.
pub struct Process {
    pub image: LoadedImage,
    pub thunks: Vec<Thunk>,
    pub thunk_by_va: HashMap<u32, usize>,
    pub stack_top: u32,
    pub stack_size: u32,
}

impl Process {
    /// Map the image and synthesize thunks. Does **not** start the
    /// CPU.
    pub fn map_into(
        image: LoadedImage,
        cpu: &mut dyn Cpu,
        ordinal_resolver: &dyn Fn(&str, u16) -> Option<String>,
    ) -> Result<Self, KernelError> {
        // 1. Map every section.
        for s in &image.sections {
            let mut prot = Prot::READ;
            if s.is_writable() {
                prot |= Prot::WRITE;
            }
            if s.is_executable() {
                prot |= Prot::EXEC;
            }
            let aligned = pocket_cpu::round_up_to_page(s.virtual_size.max(s.data.len() as u32));
            cpu.map_region(image.image_base + s.virtual_address, aligned, prot)?;
            cpu.write_mem(image.image_base + s.virtual_address, &s.data)?;
            log::debug!(
                "mapped section {:>8} va=0x{:08x} size=0x{:x} prot={:?}",
                s.name,
                image.image_base + s.virtual_address,
                aligned,
                prot
            );
        }

        // 2. Allocate a thunk pool and patch the IAT to point into it.
        let thunk_count = image.imports.len() as u32;
        let thunk_size = pocket_cpu::round_up_to_page(thunk_count * THUNK_STRIDE).max(0x1000);
        cpu.map_region(THUNK_REGION_BASE, thunk_size, Prot::READ | Prot::EXEC)?;
        let mut thunks = Vec::with_capacity(image.imports.len());
        let mut thunk_by_va = HashMap::with_capacity(image.imports.len());
        for (i, imp) in image.imports.iter().enumerate() {
            let thunk_va = THUNK_REGION_BASE + (i as u32) * THUNK_STRIDE;
            cpu.write_mem(thunk_va, &ARM_BX_LR)?;
            cpu.add_code_hook(thunk_va)?;
            let friendly_name = match &imp.binding {
                ImportBinding::Name(n) => Some(n.clone()),
                ImportBinding::Ordinal(o) => ordinal_resolver(&imp.dll, *o),
            };
            let mut iat_bytes = [0u8; 4];
            LittleEndian::write_u32(&mut iat_bytes, thunk_va);
            cpu.write_mem(imp.iat_va, &iat_bytes)?;
            thunks.push(Thunk {
                thunk_va,
                iat_va: imp.iat_va,
                dll: imp.dll.clone(),
                binding: imp.binding.clone(),
                friendly_name,
            });
            thunk_by_va.insert(thunk_va, i);
        }

        // 3. Map a stack.
        let stack_size = DEFAULT_STACK_SIZE;
        let stack_top = DEFAULT_STACK_TOP;
        let stack_base = stack_top - stack_size;
        cpu.map_region(stack_base, stack_size, Prot::READ | Prot::WRITE)?;
        cpu.write_reg(ArmReg::Sp, stack_top - 16)?;

        Ok(Process {
            image,
            thunks,
            thunk_by_va,
            stack_top,
            stack_size,
        })
    }

    /// Look up the thunk by its hook address.
    pub fn find_thunk(&self, va: u32) -> Option<&Thunk> {
        self.thunk_by_va.get(&va).and_then(|i| self.thunks.get(*i))
    }

    /// Group import symbols by DLL — useful for printing a summary.
    pub fn imports_by_dll(&self) -> IndexMap<String, Vec<&ImportSymbol>> {
        let mut by_dll: IndexMap<String, Vec<&ImportSymbol>> = IndexMap::new();
        for imp in &self.image.imports {
            by_dll
                .entry(imp.dll.to_ascii_lowercase())
                .or_default()
                .push(imp);
        }
        by_dll
    }
}

/// Drive emulated execution in a loop, dispatching each thunk hit
/// through `dispatcher` until a [`DispatchOutcome::Halt`] is returned
/// or the configured instruction budget is exhausted.
pub fn run_main_loop(
    cpu: &mut dyn Cpu,
    process: &Process,
    dispatcher: &mut dyn Dispatcher,
    instruction_budget_per_slice: u64,
    max_slices: u64,
) -> Result<(), KernelError> {
    let mut pc = process.image.entry_va();
    log::info!(
        "entering emulated main: entry=0x{:08x}, stack_top=0x{:08x}",
        pc,
        process.stack_top
    );
    for _slice in 0..max_slices {
        let stop = match cpu.run_until_hook(pc, instruction_budget_per_slice) {
            Ok(s) => s,
            Err(e) => {
                let pc_now = cpu.read_reg(ArmReg::Pc).unwrap_or(pc);
                log::error!(
                    "cpu crashed: {e}\n  last requested pc=0x{pc:08x}, current pc=0x{pc_now:08x}\n{regs}{mem}",
                    regs = dump_regs(cpu),
                    mem = dump_mem_around(cpu, pc_now, 16),
                );
                return Err(e.into());
            }
        };
        match stop {
            StopReason::InstructionLimit => {
                log::trace!("instruction slice exhausted; resuming");
                pc = cpu.read_reg(ArmReg::Pc)?;
                continue;
            }
            StopReason::Hook(addr) => {
                let thunk = process.find_thunk(addr).cloned().ok_or_else(|| {
                    KernelError::Dispatch(format!("hook fired at unmapped 0x{addr:08x}"))
                })?;
                let outcome = dispatcher.dispatch(cpu, &thunk)?;
                let r0_default = match outcome {
                    DispatchOutcome::Halt => {
                        log::info!("dispatcher requested halt at {}", thunk.label());
                        return Ok(());
                    }
                    DispatchOutcome::ReturnedR0(v) => Some((v, None)),
                    DispatchOutcome::ReturnedR0R1(a, b) => Some((a, Some(b))),
                    DispatchOutcome::Unimplemented => Some((0, None)),
                };
                if let Some((v, maybe_hi)) = r0_default {
                    cpu.write_reg(ArmReg::R0, v)?;
                    if let Some(hi) = maybe_hi {
                        cpu.write_reg(ArmReg::R1, hi)?;
                    }
                }
                let lr = cpu.read_reg(ArmReg::Lr)?;
                pc = lr & !1; // strip Thumb bit
            }
            StopReason::Requested | StopReason::OutOfBounds => return Ok(()),
        }
    }
    log::warn!("main loop hit max_slices={max_slices}; exiting");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pocket_cpu::stub::StubCpu;
    use pocket_pe::{LoadedImage, LoadedSection};

    #[test]
    fn map_simple_image() {
        let img = LoadedImage {
            source_path: "test".into(),
            machine: pocket_pe::machine::ARM,
            subsystem: pocket_pe::subsystem::WINDOWS_CE_GUI,
            image_base: 0x10000,
            size_of_image: 0x2000,
            entry_point: 0x1000,
            sections: vec![LoadedSection {
                name: ".text".into(),
                virtual_address: 0x1000,
                virtual_size: 0x800,
                characteristics: 0x6000_0020,
                data: vec![0u8; 0x800],
            }],
            imports: vec![],
            exports: IndexMap::new(),
        };
        let mut cpu = StubCpu::new();
        let p = Process::map_into(img, &mut cpu, &|_, _| None).unwrap();
        assert_eq!(p.image.entry_va(), 0x11000);
    }
}
