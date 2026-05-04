//! Skeleton handlers for `coredll.dll`.
//!
//! Only the most boring functions are implemented — the rest are
//! left as `Unimplemented` so the trace log can show exactly which
//! APIs the game relies on. Each commit can chip away at this list.

use pocket_kernel::{DispatchOutcome, KernelError};

use crate::{CallCtx, WinCeDispatcher};

pub fn register(d: &mut WinCeDispatcher) {
    let dll = "coredll.dll";
    d.register_handler(dll, "GetTickCount", get_tick_count);
    d.register_handler(dll, "Sleep", sleep);
    d.register_handler(dll, "ExitProcess", exit_process);
    d.register_handler(dll, "TerminateProcess", exit_process);
    d.register_handler(dll, "GetLastError", zero_returning);
    d.register_handler(dll, "GetCommandLineW", null_returning);
    d.register_handler(dll, "GetModuleHandleW", get_module_handle_w);
    d.register_handler(dll, "GetModuleFileNameW", zero_returning);
    d.register_handler(dll, "GetProcAddress", null_returning);
    d.register_handler(dll, "LoadLibraryW", load_library_w);
    d.register_handler(dll, "FreeLibrary", one_returning);
    // Memory / string CRT.
    d.register_handler(dll, "memset", memset);
    d.register_handler(dll, "memcpy", memcpy);
    d.register_handler(dll, "memmove", memcpy);
    d.register_handler(dll, "memcmp", memcmp);
    d.register_handler(dll, "strlen", strlen);
    d.register_handler(dll, "wcslen", wcslen);
    // Window / message stubs (always return 0 / FALSE for now).
    for f in [
        "RegisterClassW",
        "CreateWindowExW",
        "ShowWindow",
        "UpdateWindow",
        "DefWindowProcW",
        "DispatchMessageW",
        "GetMessageW",
        "TranslateMessage",
        "PostQuitMessage",
        "PostMessageW",
        "PeekMessageW",
        "SendMessageW",
        "InvalidateRect",
        "GetSystemMetrics",
        "FindResourceW",
        "LoadResource",
        "LockResource",
        "BeginPaint",
        "EndPaint",
        "GetDC",
        "ReleaseDC",
    ] {
        d.register_handler(dll, f, zero_returning);
    }
}

fn zero_returning(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn one_returning(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn null_returning(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(0))
}

/// `DWORD GetTickCount(void)` — milliseconds since boot. We use a
/// monotonic counter starting at 0 when the emulator launches.
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
    // Return a fake non-null handle so `if (h != NULL)` checks succeed.
    Ok(DispatchOutcome::ReturnedR0(0x1000_0000))
}

fn load_library_w(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // Same — pretend the DLL was already loaded.
    Ok(DispatchOutcome::ReturnedR0(0x1000_0000))
}

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

fn strlen(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let s = ctx.arg_u32(0)?;
    let mut len = 0u32;
    for _ in 0..0x10000 {
        let b = ctx.cpu.read_mem(s + len, 1)?;
        if b[0] == 0 {
            break;
        }
        len += 1;
    }
    Ok(DispatchOutcome::ReturnedR0(len))
}

fn wcslen(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let s = ctx.arg_u32(0)?;
    let mut chars = 0u32;
    for _ in 0..0x10000 {
        let b = ctx.cpu.read_mem(s + chars * 2, 2)?;
        if b[0] == 0 && b[1] == 0 {
            break;
        }
        chars += 1;
    }
    Ok(DispatchOutcome::ReturnedR0(chars))
}
