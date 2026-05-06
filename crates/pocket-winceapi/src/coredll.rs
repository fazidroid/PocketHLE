//! Skeleton handlers for `coredll.dll`.
//!
//! Coverage strategy: every coredll symbol that JumpyBall (our test
//! ROM) imports has a handler so that the trace is never silent. The
//! handlers fall into three buckets:
//!
//! 1. **Real implementations** — string/memory CRT routines that read
//!    and write the guest's address space. These have to behave
//!    correctly for the game to make any progress.
//! 2. **Fake handle / non-zero stubs** — for `Create*` functions, we
//!    return a non-null but obviously fake handle (`0xDEAD_xxxx`).
//!    The game's `if (h != NULL)` checks succeed and execution
//!    continues into the rendering path.
//! 3. **`zero_returning` / `one_returning` placeholders** — for
//!    everything else we just answer with `0` or `TRUE` and rely on
//!    the trace log to tell us when a deeper implementation is needed.
//!
//! The `__chkstk` / `_setjmp` / `longjmp` / `_except_handler3` quartet
//! deserves its own attention: those are CRT helpers the MS C compiler
//! emits in nearly every function prologue and `try`/`except` block,
//! and they get called many thousands of times before the game ever
//! reaches `WinMain`.

use pocket_cpu::regs::ArmReg;
use pocket_kernel::framebuffer::{colorref_to_rgb565, FB_HEIGHT, FB_WIDTH};
use pocket_kernel::gdi::{
    Surface, GDI_SCREEN_DC, STOCK_BLACK_BRUSH, STOCK_BLACK_PEN, STOCK_NULL_BRUSH, STOCK_NULL_PEN,
    STOCK_WHITE_BRUSH, STOCK_WHITE_PEN,
};
use pocket_kernel::{DispatchOutcome, KernelError};
use pocket_pe::ResourceKey;

use crate::{CallCtx, WinCeDispatcher};

const FAKE_MODULE_HANDLE: u32 = 0x1000_0000;
const FAKE_HWND: u32 = 0xDEAD_0001;
const INVALID_HANDLE_VALUE: u32 = 0xFFFF_FFFF;
const PAINTSTRUCT_BYTES: u32 = 32;

pub fn register(d: &mut WinCeDispatcher) {
    let dll = "coredll.dll";

    // ---- Process / module / library ----
    d.register_handler(dll, "GetTickCount", get_tick_count);
    d.register_handler(dll, "Sleep", sleep);
    d.register_handler(dll, "ExitProcess", exit_process);
    d.register_handler(dll, "TerminateProcess", exit_process);
    d.register_handler(dll, "GetLastError", zero_returning);
    d.register_handler(dll, "SetLastError", zero_returning);
    d.register_handler(dll, "GetCommandLineW", null_returning);
    d.register_handler(dll, "GetModuleHandleW", get_module_handle_w);
    d.register_handler(dll, "GetModuleFileNameW", zero_returning);
    d.register_handler(dll, "GetProcAddress", null_returning);
    d.register_handler(dll, "LoadLibraryW", load_library_w);
    d.register_handler(dll, "FreeLibrary", one_returning);

    // ---- CRT prologue helpers ----
    d.register_handler(dll, "__chkstk", chkstk);
    d.register_handler(dll, "_setjmp", setjmp);
    d.register_handler(dll, "longjmp", longjmp);
    d.register_handler(dll, "_except_handler3", except_handler3);

    // ---- Memory / string CRT ----
    d.register_handler(dll, "memset", memset);
    d.register_handler(dll, "memcpy", memcpy);
    d.register_handler(dll, "memmove", memcpy);
    d.register_handler(dll, "memcmp", memcmp);
    d.register_handler(dll, "strlen", strlen);
    d.register_handler(dll, "wcslen", wcslen);
    d.register_handler(dll, "strcpy", strcpy);
    d.register_handler(dll, "strncpy", strncpy);
    d.register_handler(dll, "strcat", strcat);
    d.register_handler(dll, "strncat", strncat);
    d.register_handler(dll, "strcmp", strcmp);
    d.register_handler(dll, "strncmp", strncmp);
    d.register_handler(dll, "strchr", strchr);
    d.register_handler(dll, "strrchr", strrchr);
    d.register_handler(dll, "strstr", strstr);
    d.register_handler(dll, "wcscpy", wcscpy);
    d.register_handler(dll, "wcsncpy", wcsncpy);
    d.register_handler(dll, "wcscat", wcscat);
    d.register_handler(dll, "wcsncat", wcsncat);
    d.register_handler(dll, "wcscmp", wcscmp);
    d.register_handler(dll, "wcsncmp", wcsncmp);
    d.register_handler(dll, "_wcsicmp", wcsicmp);
    d.register_handler(dll, "wcschr", wcschr);
    d.register_handler(dll, "wcsrchr", wcsrchr);
    d.register_handler(dll, "wcsstr", wcsstr);
    d.register_handler(dll, "swprintf", zero_returning);
    d.register_handler(dll, "wsprintfW", zero_returning);

    // ---- File I/O backed by the VFS ----
    d.register_handler(dll, "CreateFileW", create_file_w);
    d.register_handler(dll, "ReadFile", read_file);
    d.register_handler(dll, "WriteFile", write_file);
    d.register_handler(dll, "CloseHandle", close_handle);
    d.register_handler(dll, "GetFileSize", get_file_size);
    d.register_handler(dll, "SetFilePointer", set_file_pointer);
    d.register_handler(dll, "FindFirstFileW", invalid_handle_returning);
    d.register_handler(dll, "FindNextFileW", zero_returning);
    d.register_handler(dll, "FindClose", one_returning);
    d.register_handler(dll, "DeleteFileW", one_returning);
    d.register_handler(dll, "SetFileAttributesW", one_returning);
    d.register_handler(dll, "GetFileAttributesW", get_file_attributes_w);
    d.register_handler(dll, "CreateDirectoryW", one_returning);

    // ---- Heap ----
    d.register_handler(dll, "LocalAlloc", local_alloc);
    d.register_handler(dll, "LocalFree", local_free);
    d.register_handler(dll, "LocalReAlloc", local_realloc);
    d.register_handler(dll, "LocalSize", local_size);
    d.register_handler(dll, "_msize", local_size);
    d.register_handler(dll, "HeapCreate", heap_create);
    d.register_handler(dll, "HeapDestroy", one_returning);
    d.register_handler(dll, "HeapAlloc", heap_alloc);
    d.register_handler(dll, "HeapFree", heap_free);
    d.register_handler(dll, "HeapReAlloc", heap_realloc);
    d.register_handler(dll, "GetProcessHeap", get_process_heap);
    d.register_handler(dll, "VirtualAlloc", virtual_alloc);
    d.register_handler(dll, "VirtualFree", one_returning);
    d.register_handler(dll, "malloc", malloc);
    d.register_handler(dll, "calloc", calloc);
    d.register_handler(dll, "free", free);
    d.register_handler(dll, "realloc", realloc);
    d.register_handler(dll, "_new", malloc);
    d.register_handler(dll, "_delete", free);

    // ---- Resources ----
    d.register_handler(dll, "FindResourceW", find_resource_w);
    d.register_handler(dll, "LoadResource", load_resource);
    d.register_handler(dll, "LockResource", lock_resource);
    d.register_handler(dll, "SizeofResource", sizeof_resource);

    // ---- Window / message stubs ----
    d.register_handler(dll, "RegisterClassW", register_class_w);
    d.register_handler(dll, "CreateWindowExW", create_window_ex_w);
    d.register_handler(dll, "ShowWindow", one_returning);
    d.register_handler(dll, "UpdateWindow", one_returning);
    d.register_handler(dll, "DefWindowProcW", zero_returning);
    d.register_handler(dll, "DispatchMessageW", dispatch_message_w);
    d.register_handler(dll, "GetMessageW", get_message_w);
    d.register_handler(dll, "PeekMessageW", peek_message_w);
    d.register_handler(dll, "TranslateMessage", one_returning);
    d.register_handler(dll, "PostQuitMessage", post_quit_message);
    d.register_handler(dll, "PostMessageW", one_returning);
    d.register_handler(dll, "SendMessageW", zero_returning);
    d.register_handler(dll, "InvalidateRect", one_returning);
    d.register_handler(dll, "GetSystemMetrics", get_system_metrics);

    // ---- GDI (real, framebuffer-backed) ----
    d.register_handler(dll, "GetDC", get_dc);
    d.register_handler(dll, "ReleaseDC", one_returning);
    d.register_handler(dll, "BeginPaint", begin_paint);
    d.register_handler(dll, "EndPaint", end_paint);
    d.register_handler(dll, "CreateCompatibleDC", create_compatible_dc);
    d.register_handler(dll, "CreateCompatibleBitmap", create_compatible_bitmap);
    d.register_handler(dll, "CreateSolidBrush", create_solid_brush);
    d.register_handler(dll, "CreatePen", create_pen);
    d.register_handler(dll, "CreateFontIndirectW", create_font_indirect);
    d.register_handler(dll, "GetStockObject", get_stock_object);
    d.register_handler(dll, "SelectObject", select_object);
    d.register_handler(dll, "DeleteObject", delete_object);
    d.register_handler(dll, "DeleteDC", delete_object);
    d.register_handler(dll, "BitBlt", bit_blt);
    d.register_handler(dll, "Rectangle", rectangle);
    d.register_handler(dll, "FillRect", fill_rect);
    d.register_handler(dll, "SetBkMode", set_bk_mode);
    d.register_handler(dll, "SetBkColor", set_bk_color);
    d.register_handler(dll, "SetTextColor", set_text_color);
    d.register_handler(dll, "TextOutW", one_returning);
    d.register_handler(dll, "ExtTextOutW", one_returning);
}

// ---------- generic helpers ----------

fn zero_returning(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn one_returning(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn null_returning(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn invalid_handle_returning(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(INVALID_HANDLE_VALUE))
}

// ---------- process / time ----------

fn get_tick_count(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static START: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if START.load(Ordering::Relaxed) == 0 {
        START.store(now, Ordering::Relaxed);
    }
    let delta = now - START.load(Ordering::Relaxed);
    Ok(DispatchOutcome::ReturnedR0(delta as u32))
}

fn sleep(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let ms = ctx.arg_u32(0)?;
    log::trace!("Sleep({} ms) — skipped in HLE", ms);
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn exit_process(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    log::info!("ExitProcess called by guest");
    Ok(DispatchOutcome::Halt)
}

fn get_module_handle_w(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(FAKE_MODULE_HANDLE))
}

fn load_library_w(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(FAKE_MODULE_HANDLE))
}

// ---------- CRT prologue helpers ----------

/// `void __chkstk(void)` on Windows ARM is the stack-probe routine
/// inserted by the MS C compiler for any function whose locals exceed
/// one page. The real implementation walks down the stack a page at a
/// time, touching each page so the OS can grow the stack guard.
///
/// Under HLE we map the entire stack up front, so there is nothing to
/// probe — we just return immediately.
fn chkstk(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(0))
}

/// `int _setjmp(jmp_buf env)` — saves callee-saved registers + SP +
/// LR into the buffer at `r0` and returns 0. On a subsequent
/// [`longjmp`] the dispatcher restores the registers and resumes at
/// the saved LR.
///
/// jmp_buf layout used by the MS ARM compiler (32 bytes is more than
/// enough for the registers we care about):
///   `[r4, r5, r6, r7, r8, r9, r10, r11, sp, lr]`
fn setjmp(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let buf = ctx.arg_u32(0)?;
    let regs_to_save = [
        ArmReg::R4,
        ArmReg::R5,
        ArmReg::R6,
        ArmReg::R7,
        ArmReg::R8,
        ArmReg::R9,
        ArmReg::R10,
        ArmReg::R11,
        ArmReg::Sp,
        ArmReg::Lr,
    ];
    let mut blob = Vec::with_capacity(regs_to_save.len() * 4);
    for r in regs_to_save {
        let v = ctx.cpu.read_reg(r)?;
        blob.extend_from_slice(&v.to_le_bytes());
    }
    ctx.cpu.write_mem(buf, &blob)?;
    Ok(DispatchOutcome::ReturnedR0(0))
}

/// `void longjmp(jmp_buf env, int value)` — restores the buffer and
/// returns from the matching `_setjmp` with `value`.
fn longjmp(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let buf = ctx.arg_u32(0)?;
    let val = ctx.arg_u32(1)?;
    let regs_to_restore = [
        ArmReg::R4,
        ArmReg::R5,
        ArmReg::R6,
        ArmReg::R7,
        ArmReg::R8,
        ArmReg::R9,
        ArmReg::R10,
        ArmReg::R11,
        ArmReg::Sp,
        ArmReg::Lr,
    ];
    // A NULL or otherwise unmapped jmp_buf typically means the C++
    // SEH unwinder is asking for cleanup without a matching setjmp.
    // Treat it as a no-op (`R0=value`, resume from LR) and let the
    // caller continue. If that path turns out to be a fatal abort
    // signal in some game we can revisit.
    let blob = match ctx.cpu.read_mem(buf, regs_to_restore.len() as u32 * 4) {
        Ok(b) => b,
        Err(_) => {
            log::debug!(
                "longjmp(buf=0x{buf:08x}, val={val}) with unmapped jmp_buf; treating as no-op"
            );
            let ret = if val == 0 { 1 } else { val };
            return Ok(DispatchOutcome::ReturnedR0(ret));
        }
    };
    for (i, r) in regs_to_restore.iter().enumerate() {
        let off = i * 4;
        let v = u32::from_le_bytes([blob[off], blob[off + 1], blob[off + 2], blob[off + 3]]);
        ctx.cpu.write_reg(*r, v)?;
    }
    // longjmp must return `value` (or 1 if value == 0) from setjmp's
    // call site. The dispatcher will write our return into r0 and
    // resume at LR — and the LR we just restored is exactly the
    // return address of the original setjmp.
    let ret = if val == 0 { 1 } else { val };
    Ok(DispatchOutcome::ReturnedR0(ret))
}

/// `_except_handler3` is the per-frame handler the MS C compiler
/// installs for `__try`/`__except` blocks. With no SEH machinery in
/// HLE we simply tell the runtime that we did not handle the
/// exception — `ExceptionContinueSearch == 1`.
fn except_handler3(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}

// ---------- mem / string CRT ----------

fn memset(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dst = ctx.arg_u32(0)?;
    let val = ctx.arg_u32(1)? as u8;
    let len = ctx.arg_u32(2)?;
    let buf = vec![val; len as usize];
    ctx.cpu.write_mem(dst, &buf)?;
    Ok(DispatchOutcome::ReturnedR0(dst))
}

fn memcpy(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dst = ctx.arg_u32(0)?;
    let src = ctx.arg_u32(1)?;
    let len = ctx.arg_u32(2)?;
    let buf = ctx.cpu.read_mem(src, len)?;
    ctx.cpu.write_mem(dst, &buf)?;
    Ok(DispatchOutcome::ReturnedR0(dst))
}

fn memcmp(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let a = ctx.arg_u32(0)?;
    let b = ctx.arg_u32(1)?;
    let len = ctx.arg_u32(2)?;
    let av = ctx.cpu.read_mem(a, len)?;
    let bv = ctx.cpu.read_mem(b, len)?;
    let r = match av.cmp(&bv) {
        std::cmp::Ordering::Less => -1i32,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    Ok(DispatchOutcome::ReturnedR0(r as u32))
}

fn read_cstr(ctx: &mut CallCtx<'_>, p: u32, max: u32) -> Result<Vec<u8>, KernelError> {
    let mut out = Vec::new();
    for i in 0..max {
        let b = ctx.cpu.read_mem(p + i, 1)?;
        if b[0] == 0 {
            break;
        }
        out.push(b[0]);
    }
    Ok(out)
}

fn read_wstr(ctx: &mut CallCtx<'_>, p: u32, max: u32) -> Result<Vec<u16>, KernelError> {
    let mut out = Vec::new();
    for i in 0..max {
        let b = ctx.cpu.read_mem(p + i * 2, 2)?;
        let c = u16::from_le_bytes([b[0], b[1]]);
        if c == 0 {
            break;
        }
        out.push(c);
    }
    Ok(out)
}

fn strlen(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let s = ctx.arg_u32(0)?;
    let len = read_cstr(ctx, s, 0x10000)?.len() as u32;
    Ok(DispatchOutcome::ReturnedR0(len))
}

fn wcslen(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let s = ctx.arg_u32(0)?;
    let chars = read_wstr(ctx, s, 0x10000)?.len() as u32;
    Ok(DispatchOutcome::ReturnedR0(chars))
}

fn strcpy(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dst = ctx.arg_u32(0)?;
    let src = ctx.arg_u32(1)?;
    let mut s = read_cstr(ctx, src, 0x10000)?;
    s.push(0);
    ctx.cpu.write_mem(dst, &s)?;
    Ok(DispatchOutcome::ReturnedR0(dst))
}

fn strncpy(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dst = ctx.arg_u32(0)?;
    let src = ctx.arg_u32(1)?;
    let n = ctx.arg_u32(2)?;
    let s = read_cstr(ctx, src, n)?;
    let mut buf = s;
    buf.resize(n as usize, 0);
    ctx.cpu.write_mem(dst, &buf)?;
    Ok(DispatchOutcome::ReturnedR0(dst))
}

fn strcat(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dst = ctx.arg_u32(0)?;
    let src = ctx.arg_u32(1)?;
    let dst_len = read_cstr(ctx, dst, 0x10000)?.len() as u32;
    let mut s = read_cstr(ctx, src, 0x10000)?;
    s.push(0);
    ctx.cpu.write_mem(dst + dst_len, &s)?;
    Ok(DispatchOutcome::ReturnedR0(dst))
}

fn strncat(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dst = ctx.arg_u32(0)?;
    let src = ctx.arg_u32(1)?;
    let n = ctx.arg_u32(2)?;
    let dst_len = read_cstr(ctx, dst, 0x10000)?.len() as u32;
    let mut s = read_cstr(ctx, src, n)?;
    s.push(0);
    ctx.cpu.write_mem(dst + dst_len, &s)?;
    Ok(DispatchOutcome::ReturnedR0(dst))
}

fn strcmp(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let pa = ctx.arg_u32(0)?;
    let pb = ctx.arg_u32(1)?;
    let a = read_cstr(ctx, pa, 0x10000)?;
    let b = read_cstr(ctx, pb, 0x10000)?;
    Ok(DispatchOutcome::ReturnedR0(cmp_to_int(a.cmp(&b)) as u32))
}

fn strncmp(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let pa = ctx.arg_u32(0)?;
    let pb = ctx.arg_u32(1)?;
    let n = ctx.arg_u32(2)?;
    let a = read_cstr(ctx, pa, n)?;
    let b = read_cstr(ctx, pb, n)?;
    Ok(DispatchOutcome::ReturnedR0(cmp_to_int(a.cmp(&b)) as u32))
}

fn strchr(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let s = ctx.arg_u32(0)?;
    let c = ctx.arg_u32(1)? as u8;
    let bytes = read_cstr(ctx, s, 0x10000)?;
    for (i, b) in bytes.iter().enumerate() {
        if *b == c {
            return Ok(DispatchOutcome::ReturnedR0(s + i as u32));
        }
    }
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn strrchr(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let s = ctx.arg_u32(0)?;
    let c = ctx.arg_u32(1)? as u8;
    let bytes = read_cstr(ctx, s, 0x10000)?;
    let mut found = None;
    for (i, b) in bytes.iter().enumerate() {
        if *b == c {
            found = Some(i);
        }
    }
    Ok(DispatchOutcome::ReturnedR0(
        found.map(|i| s + i as u32).unwrap_or(0),
    ))
}

fn strstr(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let h = ctx.arg_u32(0)?;
    let n = ctx.arg_u32(1)?;
    let hay = read_cstr(ctx, h, 0x10000)?;
    let needle = read_cstr(ctx, n, 0x10000)?;
    if needle.is_empty() {
        return Ok(DispatchOutcome::ReturnedR0(h));
    }
    if let Some(pos) = hay.windows(needle.len()).position(|w| w == needle) {
        Ok(DispatchOutcome::ReturnedR0(h + pos as u32))
    } else {
        Ok(DispatchOutcome::ReturnedR0(0))
    }
}

fn wcscpy(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dst = ctx.arg_u32(0)?;
    let src = ctx.arg_u32(1)?;
    let mut s = read_wstr(ctx, src, 0x10000)?;
    s.push(0);
    let bytes = wide_to_bytes(&s);
    ctx.cpu.write_mem(dst, &bytes)?;
    Ok(DispatchOutcome::ReturnedR0(dst))
}

fn wcsncpy(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dst = ctx.arg_u32(0)?;
    let src = ctx.arg_u32(1)?;
    let n = ctx.arg_u32(2)?;
    let s = read_wstr(ctx, src, n)?;
    let mut buf = s;
    buf.resize(n as usize, 0);
    let bytes = wide_to_bytes(&buf);
    ctx.cpu.write_mem(dst, &bytes)?;
    Ok(DispatchOutcome::ReturnedR0(dst))
}

fn wcscat(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dst = ctx.arg_u32(0)?;
    let src = ctx.arg_u32(1)?;
    let dst_len = read_wstr(ctx, dst, 0x10000)?.len() as u32;
    let mut s = read_wstr(ctx, src, 0x10000)?;
    s.push(0);
    let bytes = wide_to_bytes(&s);
    ctx.cpu.write_mem(dst + dst_len * 2, &bytes)?;
    Ok(DispatchOutcome::ReturnedR0(dst))
}

fn wcsncat(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dst = ctx.arg_u32(0)?;
    let src = ctx.arg_u32(1)?;
    let n = ctx.arg_u32(2)?;
    let dst_len = read_wstr(ctx, dst, 0x10000)?.len() as u32;
    let mut s = read_wstr(ctx, src, n)?;
    s.push(0);
    let bytes = wide_to_bytes(&s);
    ctx.cpu.write_mem(dst + dst_len * 2, &bytes)?;
    Ok(DispatchOutcome::ReturnedR0(dst))
}

fn wcscmp(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let pa = ctx.arg_u32(0)?;
    let pb = ctx.arg_u32(1)?;
    let a = read_wstr(ctx, pa, 0x10000)?;
    let b = read_wstr(ctx, pb, 0x10000)?;
    Ok(DispatchOutcome::ReturnedR0(cmp_to_int(a.cmp(&b)) as u32))
}

fn wcsncmp(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let pa = ctx.arg_u32(0)?;
    let pb = ctx.arg_u32(1)?;
    let n = ctx.arg_u32(2)?;
    let a = read_wstr(ctx, pa, n)?;
    let b = read_wstr(ctx, pb, n)?;
    Ok(DispatchOutcome::ReturnedR0(cmp_to_int(a.cmp(&b)) as u32))
}

fn wcsicmp(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let pa = ctx.arg_u32(0)?;
    let pb = ctx.arg_u32(1)?;
    let a: Vec<u16> = read_wstr(ctx, pa, 0x10000)?
        .into_iter()
        .map(to_lower_w)
        .collect();
    let b: Vec<u16> = read_wstr(ctx, pb, 0x10000)?
        .into_iter()
        .map(to_lower_w)
        .collect();
    Ok(DispatchOutcome::ReturnedR0(cmp_to_int(a.cmp(&b)) as u32))
}

fn wcschr(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let s = ctx.arg_u32(0)?;
    let c = ctx.arg_u32(1)? as u16;
    let chars = read_wstr(ctx, s, 0x10000)?;
    for (i, w) in chars.iter().enumerate() {
        if *w == c {
            return Ok(DispatchOutcome::ReturnedR0(s + i as u32 * 2));
        }
    }
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn wcsrchr(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let s = ctx.arg_u32(0)?;
    let c = ctx.arg_u32(1)? as u16;
    let chars = read_wstr(ctx, s, 0x10000)?;
    let mut found = None;
    for (i, w) in chars.iter().enumerate() {
        if *w == c {
            found = Some(i);
        }
    }
    Ok(DispatchOutcome::ReturnedR0(
        found.map(|i| s + i as u32 * 2).unwrap_or(0),
    ))
}

fn wcsstr(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let h = ctx.arg_u32(0)?;
    let n = ctx.arg_u32(1)?;
    let hay = read_wstr(ctx, h, 0x10000)?;
    let needle = read_wstr(ctx, n, 0x10000)?;
    if needle.is_empty() {
        return Ok(DispatchOutcome::ReturnedR0(h));
    }
    if let Some(pos) = hay.windows(needle.len()).position(|w| w == needle) {
        Ok(DispatchOutcome::ReturnedR0(h + pos as u32 * 2))
    } else {
        Ok(DispatchOutcome::ReturnedR0(0))
    }
}

fn wide_to_bytes(s: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for c in s {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out
}

fn cmp_to_int(o: std::cmp::Ordering) -> i32 {
    match o {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn to_lower_w(c: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&c) {
        c + 0x20
    } else {
        c
    }
}

// ---------- file I/O ----------

/// `HANDLE CreateFileW(LPCWSTR name, DWORD access, DWORD share, ...,
///                     DWORD creation, DWORD flags, HANDLE template)`
///
/// We honour `access` (`GENERIC_READ` 0x80000000, `GENERIC_WRITE`
/// 0x40000000) and `creation` (`CREATE_ALWAYS` 2, `CREATE_NEW` 1,
/// `OPEN_ALWAYS` 4) loosely — enough to satisfy a game that just
/// wants to load assets and persist a save file.
fn create_file_w(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    use pocket_kernel::vfs::Access;
    let name_p = ctx.arg_u32(0)?;
    let access_flags = ctx.arg_u32(1)?;
    let creation = ctx.arg_u32(4)?;
    if name_p == 0 {
        return Ok(DispatchOutcome::ReturnedR0(INVALID_HANDLE_VALUE));
    }
    let name_w = match read_wstr(ctx, name_p, 260) {
        Ok(n) => n,
        Err(_) => return Ok(DispatchOutcome::ReturnedR0(INVALID_HANDLE_VALUE)),
    };
    let path = String::from_utf16_lossy(&name_w);
    let access = match (
        access_flags & 0x8000_0000 != 0,
        access_flags & 0x4000_0000 != 0,
    ) {
        (true, true) => Access::ReadWrite,
        (false, true) => Access::Write,
        _ => Access::Read,
    };
    let create = matches!(creation, 1 | 2 | 4);
    match ctx.kernel.vfs.open(&path, access, create) {
        Some(h) => {
            log::trace!("CreateFileW({path:?}, access={access:?}) -> 0x{h:08x}");
            Ok(DispatchOutcome::ReturnedR0(h))
        }
        None => {
            log::trace!("CreateFileW({path:?}) -> INVALID_HANDLE_VALUE");
            Ok(DispatchOutcome::ReturnedR0(INVALID_HANDLE_VALUE))
        }
    }
}

/// `BOOL ReadFile(HANDLE h, void* buf, DWORD count, DWORD* read,
///                LPOVERLAPPED ov)`
fn read_file(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let handle = ctx.arg_u32(0)?;
    let buf_p = ctx.arg_u32(1)?;
    let count = ctx.arg_u32(2)?;
    let out_read_p = ctx.arg_u32(3)?;
    if !ctx.kernel.vfs.is_open(handle) {
        return Ok(DispatchOutcome::ReturnedR0(0));
    }
    let mut buf = vec![0u8; count as usize];
    let n = ctx.kernel.vfs.read(handle, &mut buf).unwrap_or(0);
    if buf_p != 0 && n > 0 {
        ctx.cpu.write_mem(buf_p, &buf[..n])?;
    }
    if out_read_p != 0 {
        ctx.cpu.write_mem(out_read_p, &(n as u32).to_le_bytes())?;
    }
    Ok(DispatchOutcome::ReturnedR0(1))
}

/// `BOOL WriteFile(HANDLE h, const void* buf, DWORD count, DWORD* written, ...)`
fn write_file(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let handle = ctx.arg_u32(0)?;
    let buf_p = ctx.arg_u32(1)?;
    let count = ctx.arg_u32(2)?;
    let out_written_p = ctx.arg_u32(3)?;
    if !ctx.kernel.vfs.is_open(handle) || count == 0 {
        return Ok(DispatchOutcome::ReturnedR0(if count == 0 { 1 } else { 0 }));
    }
    let bytes = ctx.cpu.read_mem(buf_p, count)?;
    let n = ctx.kernel.vfs.write(handle, &bytes).unwrap_or(0);
    if out_written_p != 0 {
        ctx.cpu
            .write_mem(out_written_p, &(n as u32).to_le_bytes())?;
    }
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn close_handle(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let handle = ctx.arg_u32(0)?;
    let _ = ctx.kernel.vfs.close(handle);
    Ok(DispatchOutcome::ReturnedR0(1))
}

/// `DWORD GetFileSize(HANDLE h, DWORD* high)`
fn get_file_size(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let handle = ctx.arg_u32(0)?;
    let high_p = ctx.arg_u32(1)?;
    let size = ctx.kernel.vfs.size(handle).unwrap_or(0);
    if high_p != 0 {
        ctx.cpu
            .write_mem(high_p, &((size >> 32) as u32).to_le_bytes())?;
    }
    Ok(DispatchOutcome::ReturnedR0(size as u32))
}

/// `DWORD GetFileAttributesW(LPCWSTR path)` — query the VFS so that
/// games which probe asset paths before opening them get sensible
/// answers. Returns `FILE_ATTRIBUTE_NORMAL` (0x80) for regular files
/// and `FILE_ATTRIBUTE_DIRECTORY` (0x10) for directories. Missing
/// files / NULL pointers / unmounted prefixes return
/// `INVALID_FILE_ATTRIBUTES` (0xFFFF_FFFF) just like Windows does.
fn get_file_attributes_w(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
    let name_p = ctx.arg_u32(0)?;
    if name_p == 0 {
        return Ok(DispatchOutcome::ReturnedR0(INVALID_FILE_ATTRIBUTES));
    }
    let name_w = match read_wstr(ctx, name_p, 260) {
        Ok(n) => n,
        Err(_) => return Ok(DispatchOutcome::ReturnedR0(INVALID_FILE_ATTRIBUTES)),
    };
    let path = String::from_utf16_lossy(&name_w);
    let host = match ctx.kernel.vfs.resolve(&path) {
        Some(p) => p,
        None => {
            log::trace!("GetFileAttributesW({path:?}) -> INVALID (no mount)");
            return Ok(DispatchOutcome::ReturnedR0(INVALID_FILE_ATTRIBUTES));
        }
    };
    let meta = match std::fs::metadata(&host) {
        Ok(m) => m,
        Err(_) => {
            log::trace!("GetFileAttributesW({path:?}) -> INVALID (host miss {host:?})");
            return Ok(DispatchOutcome::ReturnedR0(INVALID_FILE_ATTRIBUTES));
        }
    };
    let attrs = if meta.is_dir() {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    };
    log::trace!("GetFileAttributesW({path:?}) -> 0x{attrs:08x}");
    Ok(DispatchOutcome::ReturnedR0(attrs))
}

/// `DWORD SetFilePointer(HANDLE h, LONG distance, LONG* hi, DWORD whence)`
fn set_file_pointer(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    use pocket_kernel::vfs::SeekKind;
    let handle = ctx.arg_u32(0)?;
    let distance = ctx.arg_u32(1)? as i32 as i64;
    let whence = ctx.arg_u32(3)?;
    let kind = match whence {
        0 => SeekKind::Begin,
        1 => SeekKind::Current,
        2 => SeekKind::End,
        _ => SeekKind::Begin,
    };
    let pos = ctx.kernel.vfs.seek(handle, distance, kind).unwrap_or(0);
    Ok(DispatchOutcome::ReturnedR0(pos as u32))
}

// ---------- heap ----------

const FAKE_PROCESS_HEAP: u32 = 0x4242_4242;

fn local_alloc(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // LMEM_ZEROINIT flag = 0x0040
    let flags = ctx.arg_u32(0)?;
    let size = ctx.arg_u32(1)?;
    do_alloc(ctx, size, flags & 0x0040 != 0)
}

fn local_free(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let p = ctx.arg_u32(0)?;
    if p != 0 {
        do_free(ctx, p);
    }
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn local_realloc(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let p = ctx.arg_u32(0)?;
    let size = ctx.arg_u32(1)?;
    do_realloc(ctx, p, size)
}

/// `LocalSize(HLOCAL hMem)` — return the size of the block, or 0 for
/// an unknown pointer. Doubles as the C runtime `_msize`.
fn local_size(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let p = ctx.arg_u32(0)?;
    let sz = if p == 0 {
        0
    } else {
        ctx.kernel.heap.msize(p).unwrap_or(0)
    };
    Ok(DispatchOutcome::ReturnedR0(sz))
}

fn heap_create(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(FAKE_PROCESS_HEAP))
}

fn heap_alloc(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // HeapAlloc(HANDLE hHeap, DWORD flags, SIZE_T size); HEAP_ZERO_MEMORY = 0x8
    let flags = ctx.arg_u32(1)?;
    let size = ctx.arg_u32(2)?;
    do_alloc(ctx, size, flags & 0x8 != 0)
}

fn heap_free(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let p = ctx.arg_u32(2)?;
    if p != 0 {
        do_free(ctx, p);
    }
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn heap_realloc(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let p = ctx.arg_u32(2)?;
    let size = ctx.arg_u32(3)?;
    do_realloc(ctx, p, size)
}

fn get_process_heap(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(FAKE_PROCESS_HEAP))
}

fn virtual_alloc(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // VirtualAlloc(LPVOID addr, SIZE_T size, DWORD type, DWORD protect)
    let size = ctx.arg_u32(1)?;
    do_alloc(ctx, size, true)
}

fn malloc(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let size = ctx.arg_u32(0)?;
    do_alloc(ctx, size, false)
}

fn calloc(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let nmemb = ctx.arg_u32(0)?;
    let size = ctx.arg_u32(1)?;
    do_alloc(ctx, nmemb.saturating_mul(size), true)
}

fn free(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let p = ctx.arg_u32(0)?;
    if p != 0 {
        do_free(ctx, p);
    }
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn realloc(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let p = ctx.arg_u32(0)?;
    let size = ctx.arg_u32(1)?;
    do_realloc(ctx, p, size)
}

/// Shared allocation path for every alloc-shaped API. The host-side
/// [`pocket_kernel::Heap`] tracks the requested size out of band, so
/// `LocalSize` / `_msize` / `do_free` / `do_realloc` can recover it
/// later without trusting guest memory.
fn do_alloc(
    ctx: &mut CallCtx<'_>,
    size: u32,
    zero_init: bool,
) -> Result<DispatchOutcome, KernelError> {
    let user_ptr = match ctx.kernel.heap.alloc(size) {
        Some(p) => p,
        None => {
            log::warn!("heap exhausted; alloc({size}) failed");
            return Ok(DispatchOutcome::ReturnedR0(0));
        }
    };
    if zero_init && size > 0 {
        let zeros = vec![0u8; size as usize];
        ctx.cpu.write_mem(user_ptr, &zeros)?;
    }
    Ok(DispatchOutcome::ReturnedR0(user_ptr))
}

fn do_free(ctx: &mut CallCtx<'_>, user_ptr: u32) {
    ctx.kernel.heap.free(user_ptr);
}

fn do_realloc(
    ctx: &mut CallCtx<'_>,
    p: u32,
    new_size: u32,
) -> Result<DispatchOutcome, KernelError> {
    if p == 0 {
        return do_alloc(ctx, new_size, false);
    }
    let old_size = ctx.kernel.heap.msize(p).unwrap_or(0);
    if new_size == 0 {
        do_free(ctx, p);
        return Ok(DispatchOutcome::ReturnedR0(0));
    }
    let new_p = match ctx.kernel.heap.alloc(new_size) {
        Some(np) => np,
        None => return Ok(DispatchOutcome::ReturnedR0(0)),
    };
    let to_copy = old_size.min(new_size);
    if to_copy > 0 {
        let bytes = ctx.cpu.read_mem(p, to_copy)?;
        ctx.cpu.write_mem(new_p, &bytes)?;
    }
    do_free(ctx, p);
    Ok(DispatchOutcome::ReturnedR0(new_p))
}

// ---------- window / message ----------

fn register_class_w(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // The first argument is `const WNDCLASS *`. On 32-bit Windows
    // the layout is:
    //   UINT      style;          (off 0)
    //   WNDPROC   lpfnWndProc;    (off 4)
    //   int       cbClsExtra;     (off 8)
    //   int       cbWndExtra;     (off 12)
    //   HINSTANCE hInstance;      (off 16)
    //   ...
    // We only care about lpfnWndProc — capture it so DispatchMessageW
    // can trampoline into the guest WndProc.
    let lpwc = ctx.arg_u32(0)?;
    if lpwc != 0 {
        if let Ok(buf) = ctx.cpu.read_mem(lpwc + 4, 4) {
            let proc_va = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            if proc_va != 0 {
                ctx.kernel.wnd_proc = proc_va;
                log::info!(
                    "RegisterClassW captured WndProc=0x{:08x} from WNDCLASS at 0x{:08x}",
                    proc_va,
                    lpwc
                );
            }
        }
    }
    // ATOMs are 16-bit; return a non-zero one.
    Ok(DispatchOutcome::ReturnedR0(0xC001))
}

fn create_window_ex_w(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(FAKE_HWND))
}

fn dispatch_message_w(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // DispatchMessageW(const MSG *lpMsg) — pass the message into the
    // captured WndProc and trampoline guest execution into it. The
    // WndProc's epilogue will return to our LR (the message-loop
    // call site), so the loop continues normally.
    let lp_msg = ctx.arg_u32(0)?;
    let wnd_proc = ctx.kernel.wnd_proc;
    if wnd_proc == 0 || lp_msg == 0 {
        // No registered WndProc / no message → behave like the old
        // stub: return 0, control resumes from LR.
        return Ok(DispatchOutcome::ReturnedR0(0));
    }
    let buf = match ctx.cpu.read_mem(lp_msg, 16) {
        Ok(b) => b,
        Err(_) => return Ok(DispatchOutcome::ReturnedR0(0)),
    };
    let hwnd = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let message = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let wparam = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let lparam = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    log::debug!(
        "DispatchMessageW trampoline -> WndProc(hwnd=0x{:x}, msg=0x{:x}, wp=0x{:x}, lp=0x{:x}) at 0x{:08x}",
        hwnd, message, wparam, lparam, wnd_proc
    );
    use pocket_cpu::regs::ArmReg;
    ctx.cpu.write_reg(ArmReg::R0, hwnd)?;
    ctx.cpu.write_reg(ArmReg::R1, message)?;
    ctx.cpu.write_reg(ArmReg::R2, wparam)?;
    ctx.cpu.write_reg(ArmReg::R3, lparam)?;
    // LR is already the message-loop's return address — leave it.
    Ok(DispatchOutcome::JumpTo(wnd_proc))
}

/// Build a synthetic `MSG` blob (28 bytes on 32-bit Windows) and write
/// it into the guest pointer. `message` selects which window message
/// (e.g. `WM_PAINT = 0x000F` or `WM_QUIT = 0x0012`).
fn write_synthetic_msg(
    cpu: &mut dyn pocket_cpu::Cpu,
    lp_msg: u32,
    message: u32,
) -> Result<(), KernelError> {
    if lp_msg == 0 {
        return Ok(());
    }
    // MSG: HWND hwnd; UINT message; WPARAM wParam; LPARAM lParam;
    //      DWORD time; POINT pt; — 28 bytes total.
    let mut msg = [0u8; 28];
    msg[0..4].copy_from_slice(&FAKE_HWND.to_le_bytes());
    msg[4..8].copy_from_slice(&message.to_le_bytes());
    cpu.write_mem(lp_msg, &msg)?;
    Ok(())
}

/// `BOOL GetMessageW(LPMSG lpMsg, HWND hWnd, UINT wMsgFilterMin, UINT wMsgFilterMax)`
///
/// We have no real OS message queue. To drive an HLE'd Pocket PC game
/// to actually paint, we fabricate a series of `WM_PAINT` messages
/// (up to `synthetic_message_budget`), then signal `WM_QUIT` with a
/// `0` return so the loop tears down cleanly.
fn get_message_w(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let lp_msg = ctx.arg_u32(0)?;
    let count = ctx.kernel.synthetic_message_count;
    let budget = ctx.kernel.synthetic_message_budget;
    let exhausted = budget > 0 && count >= budget;
    if exhausted {
        write_synthetic_msg(ctx.cpu, lp_msg, 0x0012)?; // WM_QUIT
        return Ok(DispatchOutcome::ReturnedR0(0));
    }
    write_synthetic_msg(ctx.cpu, lp_msg, 0x000F)?; // WM_PAINT
    ctx.kernel.synthetic_message_count = count + 1;
    Ok(DispatchOutcome::ReturnedR0(1))
}

/// `BOOL PeekMessageW(LPMSG, HWND, UINT, UINT, UINT removeMode)` —
/// returns 1 with a synthetic `WM_PAINT` until our message budget is
/// exhausted, then 0. This is what most GAPI-based games actually
/// poll on.
fn peek_message_w(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let lp_msg = ctx.arg_u32(0)?;
    let count = ctx.kernel.synthetic_message_count;
    let budget = ctx.kernel.synthetic_message_budget;
    if budget > 0 && count >= budget {
        return Ok(DispatchOutcome::ReturnedR0(0));
    }
    write_synthetic_msg(ctx.cpu, lp_msg, 0x000F)?; // WM_PAINT
    ctx.kernel.synthetic_message_count = count + 1;
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn post_quit_message(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    log::info!("PostQuitMessage called by guest");
    Ok(DispatchOutcome::Halt)
}

fn get_system_metrics(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let n = ctx.arg_u32(0)?;
    // SM_CXSCREEN=0 / SM_CYSCREEN=1 — return Pocket PC defaults so
    // the game's framebuffer math works.
    let v = match n {
        0 => 240,
        1 => 320,
        _ => 0,
    };
    Ok(DispatchOutcome::ReturnedR0(v))
}

// ---------- GDI ----------

fn get_dc(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(GDI_SCREEN_DC))
}

fn begin_paint(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // BeginPaint(hwnd, lpPaint) -> HDC. Fill the PAINTSTRUCT enough
    // for the caller (most games only read .hdc / .rcPaint).
    let _hwnd = ctx.arg_u32(0)?;
    let lp_paint = ctx.arg_u32(1)?;
    if lp_paint != 0 {
        let mut buf = [0u8; PAINTSTRUCT_BYTES as usize];
        // hdc
        buf[0..4].copy_from_slice(&GDI_SCREEN_DC.to_le_bytes());
        // fErase = 1
        buf[4..8].copy_from_slice(&1u32.to_le_bytes());
        // rcPaint = (0,0, FB_WIDTH, FB_HEIGHT)
        buf[8..12].copy_from_slice(&0u32.to_le_bytes());
        buf[12..16].copy_from_slice(&0u32.to_le_bytes());
        buf[16..20].copy_from_slice(&FB_WIDTH.to_le_bytes());
        buf[20..24].copy_from_slice(&FB_HEIGHT.to_le_bytes());
        ctx.cpu.write_mem(lp_paint, &buf)?;
    }
    Ok(DispatchOutcome::ReturnedR0(GDI_SCREEN_DC))
}

fn end_paint(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    ctx.kernel.framebuffer.mark_dirty();
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn create_compatible_dc(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let h = ctx.kernel.gdi.create_memory_dc();
    Ok(DispatchOutcome::ReturnedR0(h))
}

fn create_compatible_bitmap(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let _hdc = ctx.arg_u32(0)?;
    let w = ctx.arg_u32(1)?;
    let h = ctx.arg_u32(2)?;
    let handle = ctx.kernel.gdi.create_compatible_bitmap(w, h);
    Ok(DispatchOutcome::ReturnedR0(handle))
}

fn create_solid_brush(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let color = ctx.arg_u32(0)?;
    let h = ctx.kernel.gdi.create_solid_brush(color);
    Ok(DispatchOutcome::ReturnedR0(h))
}

fn create_pen(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let _style = ctx.arg_u32(0)?;
    let width = ctx.arg_u32(1)?;
    let color = ctx.arg_u32(2)?;
    let h = ctx.kernel.gdi.create_pen(color, width);
    Ok(DispatchOutcome::ReturnedR0(h))
}

fn create_font_indirect(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // We ignore the LOGFONT contents; just allocate a font handle so
    // the caller can SelectObject it. Default height 0 is fine.
    let h = ctx.kernel.gdi.create_font(0);
    Ok(DispatchOutcome::ReturnedR0(h))
}

fn get_stock_object(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // Map stock indices to our pre-registered handles.
    let idx = ctx.arg_u32(0)?;
    let h = match idx {
        0 => STOCK_WHITE_BRUSH, // WHITE_BRUSH
        1 => 0xDEAD_5702,       // LTGRAY_BRUSH (synthetic)
        2 => 0xDEAD_5703,       // GRAY_BRUSH
        4 => STOCK_BLACK_BRUSH, // BLACK_BRUSH
        5 => STOCK_NULL_BRUSH,  // NULL_BRUSH / HOLLOW_BRUSH
        6 => STOCK_WHITE_PEN,   // WHITE_PEN
        7 => STOCK_BLACK_PEN,   // BLACK_PEN
        8 => STOCK_NULL_PEN,    // NULL_PEN
        17 => 0xDEAD_5710,      // DEFAULT_GUI_FONT
        _ => STOCK_WHITE_BRUSH,
    };
    Ok(DispatchOutcome::ReturnedR0(h))
}

fn select_object(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dc = ctx.arg_u32(0)?;
    let obj = ctx.arg_u32(1)?;
    let prev = ctx.kernel.gdi.select_into(dc, obj);
    Ok(DispatchOutcome::ReturnedR0(prev))
}

fn delete_object(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let h = ctx.arg_u32(0)?;
    let _ = ctx.kernel.gdi.delete(h);
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn set_bk_mode(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dc = ctx.arg_u32(0)?;
    let mode = ctx.arg_u32(1)?;
    if let Some(d) = ctx.kernel.gdi.dc_mut(dc) {
        d.bk_transparent = mode == 1; // TRANSPARENT
    }
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn set_bk_color(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dc = ctx.arg_u32(0)?;
    let color = ctx.arg_u32(1)?;
    if let Some(d) = ctx.kernel.gdi.dc_mut(dc) {
        d.bk_color = color;
    }
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn set_text_color(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let dc = ctx.arg_u32(0)?;
    let color = ctx.arg_u32(1)?;
    if let Some(d) = ctx.kernel.gdi.dc_mut(dc) {
        d.text_color = color;
    }
    Ok(DispatchOutcome::ReturnedR0(0))
}

/// Borrow either the framebuffer or a memory bitmap as a writable
/// surface, given a DC handle.
fn surface_for_dc<'a>(state: &'a mut pocket_kernel::KernelState, dc: u32) -> Option<Surface<'a>> {
    let dc_meta = state.gdi.dc(dc)?.clone();
    match dc_meta.surface {
        pocket_kernel::gdi::DcSurface::Screen => Some(Surface::Screen(&mut state.framebuffer)),
        pocket_kernel::gdi::DcSurface::Memory => {
            let bm = dc_meta.selected_bitmap?;
            state.gdi.bitmap_mut(bm).map(Surface::Bitmap)
        }
    }
}

fn fill_rect(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // FillRect(hdc, lprc, hbr): fill rectangle with brush colour.
    let hdc = ctx.arg_u32(0)?;
    let rc_ptr = ctx.arg_u32(1)?;
    let hbr = ctx.arg_u32(2)?;
    let rc = ctx.cpu.read_mem(rc_ptr, 16)?;
    let l = i32::from_le_bytes([rc[0], rc[1], rc[2], rc[3]]);
    let t = i32::from_le_bytes([rc[4], rc[5], rc[6], rc[7]]);
    let r = i32::from_le_bytes([rc[8], rc[9], rc[10], rc[11]]);
    let b = i32::from_le_bytes([rc[12], rc[13], rc[14], rc[15]]);
    let color = ctx
        .kernel
        .gdi
        .brush(hbr)
        .map(|b| b.color)
        .unwrap_or(0x00ff_ffff);
    let rgb = colorref_to_rgb565(color);
    if let Some(mut surf) = surface_for_dc(ctx.kernel, hdc) {
        surf.fill_rect(l, t, r - l, b - t, rgb);
    }
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn rectangle(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // Rectangle(hdc, l, t, r, b)
    let hdc = ctx.arg_u32(0)?;
    let l = ctx.arg_u32(1)? as i32;
    let t = ctx.arg_u32(2)? as i32;
    let r = ctx.arg_u32(3)? as i32;
    let b = ctx.arg_u32(4)? as i32;
    let dc_meta = ctx
        .kernel
        .gdi
        .dc(hdc)
        .cloned()
        .ok_or_else(|| KernelError::Dispatch(format!("Rectangle: bad HDC 0x{hdc:08x}")))?;
    let fill_rgb = colorref_to_rgb565(dc_meta.brush_color);
    let stroke_rgb = colorref_to_rgb565(dc_meta.pen_color);
    if let Some(mut surf) = surface_for_dc(ctx.kernel, hdc) {
        surf.fill_rect(l, t, r - l, b - t, fill_rgb);
        surf.stroke_rect(l, t, r - l, b - t, stroke_rgb);
    }
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn bit_blt(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // BitBlt(hdcDest, x, y, cx, cy, hdcSrc, x1, y1, rop) → BOOL.
    let hdc_dst = ctx.arg_u32(0)?;
    let x = ctx.arg_u32(1)? as i32;
    let y = ctx.arg_u32(2)? as i32;
    let cx = ctx.arg_u32(3)? as i32;
    let cy = ctx.arg_u32(4)? as i32;
    let hdc_src = ctx.arg_u32(5)?;
    let x1 = ctx.arg_u32(6)? as i32;
    let y1 = ctx.arg_u32(7)? as i32;
    let _rop = ctx.arg_u32(8)?;

    // Read the source: either selected bitmap of a memory DC, or a
    // snapshot of the framebuffer if BitBlt-ing from the screen.
    let (src_pixels, src_w, src_h) = match ctx.kernel.gdi.dc(hdc_src).cloned() {
        Some(dc) => match dc.surface {
            pocket_kernel::gdi::DcSurface::Screen => (
                ctx.kernel.framebuffer.pixels.clone(),
                ctx.kernel.framebuffer.width,
                ctx.kernel.framebuffer.height,
            ),
            pocket_kernel::gdi::DcSurface::Memory => match dc.selected_bitmap {
                Some(bh) => match ctx.kernel.gdi.bitmap(bh) {
                    Some(b) => (b.pixels.clone(), b.width, b.height),
                    None => (Vec::new(), 0, 0),
                },
                None => (Vec::new(), 0, 0),
            },
        },
        None => (Vec::new(), 0, 0),
    };

    if src_w == 0 || src_h == 0 {
        return Ok(DispatchOutcome::ReturnedR0(0));
    }
    if let Some(mut dst) = surface_for_dc(ctx.kernel, hdc_dst) {
        dst.blit_from_bytes(x, y, x1, y1, cx, cy, &src_pixels, src_w, src_h);
    }
    Ok(DispatchOutcome::ReturnedR0(1))
}

// ---------- Resources ----------

fn read_wide_resource_key(ctx: &mut CallCtx<'_>, raw: u32) -> Result<ResourceKey, KernelError> {
    if raw < 0x1_0000 {
        // MAKEINTRESOURCE encoding — low 16 bits are an integer ID.
        Ok(ResourceKey::Id(raw))
    } else {
        let mut name = String::new();
        let mut va = raw;
        for _ in 0..256 {
            let b = ctx.cpu.read_mem(va, 2)?;
            let cu = u16::from_le_bytes([b[0], b[1]]);
            if cu == 0 {
                break;
            }
            if let Some(c) = char::from_u32(cu as u32) {
                name.push(c);
            }
            va = va.wrapping_add(2);
        }
        Ok(ResourceKey::Name(name))
    }
}

fn find_resource_w(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // FindResourceW(hModule, lpName, lpType)
    let _hmod = ctx.arg_u32(0)?;
    let name_raw = ctx.arg_u32(1)?;
    let type_raw = ctx.arg_u32(2)?;
    let want_name = read_wide_resource_key(ctx, name_raw)?;
    let want_type = read_wide_resource_key(ctx, type_raw)?;
    if let Some(entry) = ctx
        .kernel
        .resources
        .iter()
        .find(|e| e.ty == want_type && e.name == want_name)
    {
        let va = ctx.kernel.image_base.wrapping_add(entry.data_rva);
        log::trace!(
            "FindResourceW(name={want_name:?}, type={want_type:?}) -> 0x{va:08x} ({} bytes)",
            entry.size
        );
        return Ok(DispatchOutcome::ReturnedR0(va));
    }
    log::trace!("FindResourceW(name={want_name:?}, type={want_type:?}) -> NULL");
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn load_resource(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // LoadResource just returns the same handle on Windows when the
    // resource is in-image. We've already encoded the data VA in the
    // FindResource result.
    let h = ctx.arg_u32(1)?;
    Ok(DispatchOutcome::ReturnedR0(h))
}

fn lock_resource(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let h = ctx.arg_u32(0)?;
    Ok(DispatchOutcome::ReturnedR0(h))
}

fn sizeof_resource(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // SizeofResource(hModule, hResInfo) — hResInfo is the VA we
    // returned from FindResourceW. We look up by data_rva.
    let h = ctx.arg_u32(1)?;
    let rva = h.wrapping_sub(ctx.kernel.image_base);
    if let Some(e) = ctx.kernel.resources.iter().find(|e| e.data_rva == rva) {
        return Ok(DispatchOutcome::ReturnedR0(e.size));
    }
    Ok(DispatchOutcome::ReturnedR0(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pocket_cpu::{regs::ArmReg, stub::StubCpu, Cpu, Prot};
    use pocket_kernel::{vfs::Vfs, Heap, KernelState, Thunk};
    use pocket_pe::ImportBinding;

    fn fresh_kernel() -> KernelState {
        use pocket_kernel::{Framebuffer, GdiState};
        KernelState {
            heap: Heap::new(0x5000_0000, 0x10000),
            vfs: Vfs::new(),
            framebuffer: Framebuffer::default(),
            gdi: GdiState::new(),
            resources: vec![],
            image_base: 0,
            fb_mapped: false,
            synthetic_message_count: 0,
            synthetic_message_budget: 240,
            wnd_proc: 0,
        }
    }

    fn dummy_thunk() -> Thunk {
        Thunk {
            thunk_va: 0x70000000,
            iat_va: 0x20000,
            dll: "coredll.dll".into(),
            binding: ImportBinding::Name("test".into()),
            friendly_name: None,
        }
    }

    #[test]
    fn strlen_walks_until_null() {
        let mut cpu = StubCpu::new();
        let mut kernel = fresh_kernel();
        cpu.map_region(0x1000, 0x1000, Prot::READ | Prot::WRITE)
            .unwrap();
        cpu.write_mem(0x1000, b"hello\0").unwrap();
        cpu.write_reg(ArmReg::R0, 0x1000).unwrap();
        let t = dummy_thunk();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        let r = strlen(&mut c).unwrap();
        match r {
            DispatchOutcome::ReturnedR0(v) => assert_eq!(v, 5),
            _ => panic!(),
        }
    }

    #[test]
    fn setjmp_then_longjmp_restores_state() {
        let mut cpu = StubCpu::new();
        let mut kernel = fresh_kernel();
        cpu.map_region(0x1000, 0x1000, Prot::READ | Prot::WRITE)
            .unwrap();
        let t = dummy_thunk();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        c.cpu.write_reg(ArmReg::R0, 0x1000).unwrap();
        c.cpu.write_reg(ArmReg::R4, 0xCAFE).unwrap();
        c.cpu.write_reg(ArmReg::Lr, 0xBADC0DE).unwrap();
        let _ = setjmp(&mut c).unwrap();
        // Trash registers so we can prove longjmp restores them.
        c.cpu.write_reg(ArmReg::R4, 0).unwrap();
        c.cpu.write_reg(ArmReg::Lr, 0).unwrap();
        c.cpu.write_reg(ArmReg::R0, 0x1000).unwrap();
        c.cpu.write_reg(ArmReg::R1, 42).unwrap();
        let r = longjmp(&mut c).unwrap();
        match r {
            DispatchOutcome::ReturnedR0(v) => assert_eq!(v, 42),
            _ => panic!(),
        }
        assert_eq!(c.cpu.read_reg(ArmReg::R4).unwrap(), 0xCAFE);
        assert_eq!(c.cpu.read_reg(ArmReg::Lr).unwrap(), 0xBADC0DE);
    }

    #[test]
    fn wcslen_counts_until_null() {
        let mut cpu = StubCpu::new();
        let mut kernel = fresh_kernel();
        cpu.map_region(0x1000, 0x1000, Prot::READ | Prot::WRITE)
            .unwrap();
        let s: Vec<u8> = "hi\0"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        cpu.write_mem(0x1000, &s).unwrap();
        cpu.write_reg(ArmReg::R0, 0x1000).unwrap();
        let t = dummy_thunk();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        let r = wcslen(&mut c).unwrap();
        match r {
            DispatchOutcome::ReturnedR0(v) => assert_eq!(v, 2),
            _ => panic!(),
        }
    }

    #[test]
    fn malloc_then_free_round_trips() {
        let mut cpu = StubCpu::new();
        let mut kernel = fresh_kernel();
        cpu.map_region(0x5000_0000, 0x10000, Prot::READ | Prot::WRITE)
            .unwrap();
        let initial_free = kernel.heap.free_bytes();
        cpu.write_reg(ArmReg::R0, 64).unwrap();
        let t = dummy_thunk();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        let p = match malloc(&mut c).unwrap() {
            DispatchOutcome::ReturnedR0(p) => p,
            _ => panic!(),
        };
        assert!(p >= 0x5000_0000);
        c.cpu.write_reg(ArmReg::R0, p).unwrap();
        let _ = free(&mut c).unwrap();
        assert_eq!(c.kernel.heap.free_bytes(), initial_free);
    }

    #[test]
    fn create_file_w_with_no_mount_is_invalid() {
        let mut cpu = StubCpu::new();
        let mut kernel = fresh_kernel();
        cpu.map_region(0x1000, 0x1000, Prot::READ | Prot::WRITE)
            .unwrap();
        cpu.map_region(0x2000, 0x1000, Prot::READ | Prot::WRITE)
            .unwrap();
        // Write a wide-string "\X\foo.txt" at 0x1000.
        let s: Vec<u8> = "\\X\\foo.txt\0"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        cpu.write_mem(0x1000, &s).unwrap();
        cpu.write_reg(ArmReg::R0, 0x1000).unwrap();
        cpu.write_reg(ArmReg::R1, 0x8000_0000).unwrap(); // GENERIC_READ
        cpu.write_reg(ArmReg::Sp, 0x2800).unwrap();
        let t = dummy_thunk();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        let r = create_file_w(&mut c).unwrap();
        assert_eq!(r, DispatchOutcome::ReturnedR0(INVALID_HANDLE_VALUE));
    }

    #[test]
    fn create_file_w_with_mount_returns_real_handle() {
        let mut cpu = StubCpu::new();
        let mut kernel = fresh_kernel();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), b"hi").unwrap();
        kernel.vfs.mount("\\App\\", dir.path());
        cpu.map_region(0x1000, 0x1000, Prot::READ | Prot::WRITE)
            .unwrap();
        cpu.map_region(0x2000, 0x1000, Prot::READ | Prot::WRITE)
            .unwrap();
        let s: Vec<u8> = "\\App\\hello.txt\0"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        cpu.write_mem(0x1000, &s).unwrap();
        cpu.write_reg(ArmReg::R0, 0x1000).unwrap();
        cpu.write_reg(ArmReg::R1, 0x8000_0000).unwrap(); // GENERIC_READ
        cpu.write_reg(ArmReg::Sp, 0x2800).unwrap();
        let t = dummy_thunk();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        let r = create_file_w(&mut c).unwrap();
        match r {
            DispatchOutcome::ReturnedR0(h) => {
                assert_ne!(h, INVALID_HANDLE_VALUE);
                assert!(c.kernel.vfs.is_open(h));
            }
            _ => panic!(),
        }
    }

    // ---- GDI handler tests ----

    #[test]
    fn fill_rect_paints_into_framebuffer() {
        let mut cpu = StubCpu::new();
        let mut kernel = fresh_kernel();
        cpu.map_region(0x1000, 0x1000, Prot::READ | Prot::WRITE)
            .unwrap();
        // RECT { 5, 7, 25, 27 }
        let mut rect = Vec::new();
        rect.extend_from_slice(&5i32.to_le_bytes());
        rect.extend_from_slice(&7i32.to_le_bytes());
        rect.extend_from_slice(&25i32.to_le_bytes());
        rect.extend_from_slice(&27i32.to_le_bytes());
        cpu.write_mem(0x1000, &rect).unwrap();

        // Allocate a brush.
        cpu.write_reg(ArmReg::R0, 0x00ff0000).unwrap(); // COLORREF: red
        let t = dummy_thunk();
        let hbr = {
            let mut c = CallCtx {
                cpu: &mut cpu,
                thunk: &t,
                kernel: &mut kernel,
            };
            match create_solid_brush(&mut c).unwrap() {
                DispatchOutcome::ReturnedR0(h) => h,
                _ => panic!(),
            }
        };
        // FillRect(GDI_SCREEN_DC, 0x1000, hbr).
        cpu.write_reg(ArmReg::R0, GDI_SCREEN_DC).unwrap();
        cpu.write_reg(ArmReg::R1, 0x1000).unwrap();
        cpu.write_reg(ArmReg::R2, hbr).unwrap();
        let pre = kernel.framebuffer.frame_counter;
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        let r = fill_rect(&mut c).unwrap();
        assert_eq!(r, DispatchOutcome::ReturnedR0(1));
        assert!(kernel.framebuffer.frame_counter > pre);
        // Pixel at (5,7) must now be non-zero (red 0xF800 in RGB565,
        // little-endian on the wire).
        let off = (7 * pocket_kernel::framebuffer::FB_WIDTH as usize + 5) * 2;
        assert_ne!(kernel.framebuffer.pixels[off], 0);
    }
}
