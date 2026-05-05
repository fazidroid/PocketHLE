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
use pocket_pe::{ImportBinding, ImportSymbol, LoadedImage, ResourceEntry};

pub mod framebuffer;
pub mod gdi;
pub mod vfs;

pub use framebuffer::{Framebuffer, FB_BYTES, FB_HEIGHT, FB_WIDTH};
pub use gdi::{GdiState, Surface};

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

/// Base of the WinCE kernel callback / trap region. Real Pocket PC
/// kernels publish a sea of small syscall trampolines starting at
/// `0xF000_0000`; coredll routes things like exception delivery and
/// `KernelIoControl` through fixed offsets into this page. Under HLE
/// we don't run the kernel at all, but several library routines still
/// load function pointers out of this range and `bx` to them, so we
/// have to make the address space at least valid. We map a 64 KiB
/// page filled with `bx lr` so any such jump returns harmlessly.
pub const KERNEL_TRAP_BASE: u32 = 0xF000_0000;
pub const KERNEL_TRAP_SIZE: u32 = 0x0001_0000;

/// Base of the guest-side heap region. 16 MiB is plenty for the
/// little games we target and still leaves headroom for the stack.
pub const HEAP_BASE: u32 = 0x5000_0000;
pub const HEAP_SIZE: u32 = 0x0100_0000;

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
        kernel: &mut KernelState,
    ) -> Result<DispatchOutcome, KernelError>;
}

/// Mutable kernel state that persists across calls and that handlers
/// need to read or modify. Bundled into one struct so we can hand it
/// out by `&mut` without conflicting with the immutable parts of
/// [`Process`] (image bytes, thunk table) that the run loop uses.
pub struct KernelState {
    pub heap: Heap,
    pub vfs: vfs::Vfs,
    /// Software-rendered display the GDI/GAPI handlers paint into.
    pub framebuffer: Framebuffer,
    /// Tracked GDI objects (DCs, bitmaps, brushes, pens, fonts).
    pub gdi: GdiState,
    /// Flat resource table for `FindResourceW` / `LoadResource`.
    pub resources: Vec<ResourceEntry>,
    /// Image base for resource RVA → VA conversion.
    pub image_base: u32,
    /// Set the first time `GXBeginDraw` runs. The dispatcher maps the
    /// framebuffer region into the guest VA space lazily — but that
    /// requires `&mut dyn Cpu`, which isn't available outside a call,
    /// so we let the GAPI handlers do it.
    pub fb_mapped: bool,
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

/// Very small chunk-based heap allocator that hands out chunks from a
/// fixed guest VA range. Implemented as a free list of free blocks
/// keyed by start VA, with coalescing on free.
///
/// The expected use case is *games* that do a couple thousand small
/// allocations — fragmentation behaviour is fine for that. We do not
/// try to compete with `dlmalloc`. Each allocated block is preceded
/// by an 8-byte header so `free()` can recover the size and link the
/// block back into the free list.
#[derive(Debug)]
pub struct Heap {
    base: u32,
    size: u32,
    /// Sorted by start VA. Each entry is `(start, size)` of free space.
    free: Vec<(u32, u32)>,
}

const HEAP_HEADER_BYTES: u32 = 8;
const HEAP_ALIGN: u32 = 8;

impl Heap {
    pub fn new(base: u32, size: u32) -> Self {
        Self {
            base,
            size,
            free: vec![(base, size)],
        }
    }

    pub fn base(&self) -> u32 {
        self.base
    }
    pub fn size(&self) -> u32 {
        self.size
    }

    fn align_up(n: u32) -> u32 {
        (n + (HEAP_ALIGN - 1)) & !(HEAP_ALIGN - 1)
    }

    /// Return the user pointer (after the 8-byte header), or `None`
    /// if the heap has no large-enough free block.
    pub fn alloc(&mut self, requested: u32) -> Option<u32> {
        let need = Self::align_up(requested.max(1)) + HEAP_HEADER_BYTES;
        for i in 0..self.free.len() {
            let (start, sz) = self.free[i];
            if sz >= need {
                if sz == need {
                    self.free.remove(i);
                } else {
                    self.free[i] = (start + need, sz - need);
                }
                return Some(start + HEAP_HEADER_BYTES);
            }
        }
        None
    }

    /// Free a previously allocated chunk. The header at `user_ptr - 8`
    /// stores `(block_start, block_size)`. We trust the caller (the
    /// guest) — bad frees are logged and ignored.
    pub fn free(&mut self, user_ptr: u32, recorded_size: u32) {
        if user_ptr < self.base + HEAP_HEADER_BYTES {
            log::warn!("heap.free: ignoring out-of-range pointer 0x{user_ptr:08x}");
            return;
        }
        let block_start = user_ptr - HEAP_HEADER_BYTES;
        let block_size = recorded_size + HEAP_HEADER_BYTES;
        if block_start + block_size > self.base + self.size {
            log::warn!("heap.free: chunk overflows heap; ignoring");
            return;
        }
        // insert and coalesce
        let pos = self.free.partition_point(|(s, _)| *s < block_start);
        self.free.insert(pos, (block_start, block_size));
        // coalesce with neighbours
        let mut merged = Vec::with_capacity(self.free.len());
        for (s, sz) in self.free.drain(..) {
            if let Some((ps, psz)) = merged.last_mut() {
                if *ps + *psz == s {
                    *psz += sz;
                    continue;
                }
            }
            merged.push((s, sz));
        }
        self.free = merged;
    }

    pub fn free_bytes(&self) -> u32 {
        self.free.iter().map(|(_, s)| *s).sum()
    }
}

/// The whole emulated process state owned by the kernel.
pub struct Process {
    pub image: LoadedImage,
    pub thunks: Vec<Thunk>,
    pub thunk_by_va: HashMap<u32, usize>,
    pub stack_top: u32,
    pub stack_size: u32,
    pub state: KernelState,
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

        // 4. Map a heap.
        cpu.map_region(HEAP_BASE, HEAP_SIZE, Prot::READ | Prot::WRITE)?;
        let heap = Heap::new(HEAP_BASE, HEAP_SIZE);

        // 5. Map the WinCE kernel trap region. Real WinCE kernels
        //    publish syscall entry points at fixed offsets inside
        //    `0xF000_0000+`. We don't know the exact callsites
        //    coredll routes through this range, so we fill the page
        //    with `bx lr` — any guest jump there returns harmlessly.
        cpu.map_region(KERNEL_TRAP_BASE, KERNEL_TRAP_SIZE, Prot::READ | Prot::EXEC)?;
        let mut trap_page = Vec::with_capacity(KERNEL_TRAP_SIZE as usize);
        while trap_page.len() < KERNEL_TRAP_SIZE as usize {
            trap_page.extend_from_slice(&ARM_BX_LR);
        }
        cpu.write_mem(KERNEL_TRAP_BASE, &trap_page)?;

        let resources = image.resources.clone();
        let img_base = image.image_base;
        Ok(Process {
            image,
            thunks,
            thunk_by_va,
            stack_top,
            stack_size,
            state: KernelState {
                heap,
                vfs: vfs::Vfs::new(),
                framebuffer: Framebuffer::default(),
                gdi: GdiState::new(),
                resources,
                image_base: img_base,
                fb_mapped: false,
            },
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

/// Returned from a [`FrameHook`] to indicate whether emulation
/// should continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameAction {
    Continue,
    Stop,
}

/// Callback that observes the framebuffer between dispatch slices.
/// Used by the host-side frontend to display the rendered frame and
/// pump window events.
pub trait FrameHook {
    /// Called between dispatcher slices. The hook receives the
    /// kernel state — typically the framebuffer — and returns
    /// whether emulation should keep running.
    fn on_frame(&mut self, state: &KernelState) -> FrameAction;
}

impl<F: FnMut(&KernelState) -> FrameAction> FrameHook for F {
    fn on_frame(&mut self, state: &KernelState) -> FrameAction {
        self(state)
    }
}

/// Drive emulated execution in a loop, dispatching each thunk hit
/// through `dispatcher` until a [`DispatchOutcome::Halt`] is returned
/// or the configured instruction budget is exhausted.
pub fn run_main_loop(
    cpu: &mut dyn Cpu,
    process: &mut Process,
    dispatcher: &mut dyn Dispatcher,
    instruction_budget_per_slice: u64,
    max_slices: u64,
) -> Result<(), KernelError> {
    run_main_loop_with_hook(
        cpu,
        process,
        dispatcher,
        instruction_budget_per_slice,
        max_slices,
        None,
    )
}

/// Same as [`run_main_loop`], but also calls `frame_hook` between
/// each slice so the host-side window can repaint and pump events.
pub fn run_main_loop_with_hook(
    cpu: &mut dyn Cpu,
    process: &mut Process,
    dispatcher: &mut dyn Dispatcher,
    instruction_budget_per_slice: u64,
    max_slices: u64,
    mut frame_hook: Option<&mut dyn FrameHook>,
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
                let thunk = {
                    let t = process.find_thunk(addr).ok_or_else(|| {
                        KernelError::Dispatch(format!("hook fired at unmapped 0x{addr:08x}"))
                    })?;
                    t.clone()
                };
                let outcome = dispatcher.dispatch(cpu, &thunk, &mut process.state)?;
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
        if let Some(hook) = frame_hook.as_deref_mut() {
            if hook.on_frame(&process.state) == FrameAction::Stop {
                log::info!("frame hook requested stop");
                return Ok(());
            }
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
    fn heap_alloc_then_free_round_trips() {
        let mut h = Heap::new(0x1000_0000, 0x1_0000);
        let initial_free = h.free_bytes();
        let a = h.alloc(64).unwrap();
        let b = h.alloc(128).unwrap();
        assert!(b > a);
        assert!(h.free_bytes() < initial_free);
        h.free(a, 64);
        h.free(b, 128);
        // After freeing both, the heap should be fully coalesced.
        assert_eq!(h.free_bytes(), initial_free);
    }

    #[test]
    fn heap_returns_aligned_pointers() {
        let mut h = Heap::new(0x1000_0000, 0x1_0000);
        let a = h.alloc(1).unwrap();
        let b = h.alloc(7).unwrap();
        assert_eq!(a % 8, 0);
        assert_eq!(b % 8, 0);
    }

    #[test]
    fn heap_exhaustion_returns_none() {
        let mut h = Heap::new(0x1000_0000, 0x80);
        let _ = h.alloc(60).unwrap();
        // Header overhead leaves only ~52 bytes free; 60 should fail.
        assert!(h.alloc(60).is_none());
    }

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
            resources: vec![],
        };
        let mut cpu = StubCpu::new();
        let p = Process::map_into(img, &mut cpu, &|_, _| None).unwrap();
        assert_eq!(p.image.entry_va(), 0x11000);
    }
}
