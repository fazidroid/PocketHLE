//! Microsoft GAPI (Game API) — `gx.dll`.
//!
//! GAPI exposes nine functions that give a Pocket PC game direct
//! access to the framebuffer. The interface is small and very stable
//! across devices. Implemented here as logged stubs that return
//! "success" plus a synthetic `GXDisplayProperties` record so games
//! see a 240x320, 16 bits-per-pixel landscape device.

use pocket_kernel::{DispatchOutcome, KernelError};

use crate::{CallCtx, WinCeDispatcher};

/// Synthetic framebuffer base address. Mapped lazily by the
/// dispatcher on `GXBeginDraw`. The value is chosen well above the
/// thunk pool so it cannot collide with normal allocations.
pub const SYNTHETIC_FB_BASE: u32 = 0x7800_0000;
pub const SCREEN_WIDTH: u32 = 240;
pub const SCREEN_HEIGHT: u32 = 320;
/// 16 bpp framebuffer, default Pocket PC depth.
pub const SCREEN_BPP: u32 = 16;

pub fn register(d: &mut WinCeDispatcher) {
    let dll = "gx.dll";
    // The names below are the *demangled* C++ names used in the
    // import directory of the JumpyBall PE. We strip mangling for
    // dispatch lookup.
    d.register_handler(dll, "?GXOpenDisplay@@YAHPAUHWND__@@K@Z", gx_open_display);
    d.register_handler(dll, "?GXCloseDisplay@@YAHXZ", gx_close_display);
    d.register_handler(dll, "?GXBeginDraw@@YAPAXXZ", gx_begin_draw);
    d.register_handler(dll, "?GXEndDraw@@YAHXZ", gx_end_draw);
    d.register_handler(dll, "?GXSuspend@@YAHXZ", gx_suspend);
    d.register_handler(dll, "?GXOpenInput@@YAHXZ", gx_open_input);
    d.register_handler(dll, "?GXCloseInput@@YAHXZ", gx_close_input);
    d.register_handler(
        dll,
        "?GXGetDefaultKeys@@YA?AUGXKeyList@@H@Z",
        gx_get_default_keys,
    );
    d.register_handler(
        dll,
        "?GXGetDisplayProperties@@YA?AUGXDisplayProperties@@XZ",
        gx_get_display_properties,
    );
}

fn gx_open_display(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    log::info!("GXOpenDisplay() -> 1");
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn gx_close_display(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn gx_begin_draw(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    log::trace!("GXBeginDraw() -> 0x{:08x}", SYNTHETIC_FB_BASE);
    Ok(DispatchOutcome::ReturnedR0(SYNTHETIC_FB_BASE))
}

fn gx_end_draw(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn gx_suspend(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn gx_open_input(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn gx_close_input(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}

fn gx_get_default_keys(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // The function returns a `GXKeyList` value via a hidden pointer
    // passed in r0 (sret on ARM AAPCS). We zero-fill it for now.
    let sret = ctx.arg_u32(0)?;
    let zero = vec![0u8; 0x80];
    ctx.cpu.write_mem(sret, &zero)?;
    Ok(DispatchOutcome::ReturnedR0(sret))
}

fn gx_get_display_properties(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // Returns GXDisplayProperties { cxWidth, cyHeight, cbxPitch, cbyPitch, cBPP, ffFormat }.
    let sret = ctx.arg_u32(0)?;
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&SCREEN_WIDTH.to_le_bytes());
    buf.extend_from_slice(&SCREEN_HEIGHT.to_le_bytes());
    buf.extend_from_slice(&(SCREEN_BPP / 8).to_le_bytes());
    buf.extend_from_slice(&(SCREEN_WIDTH * SCREEN_BPP / 8).to_le_bytes());
    buf.extend_from_slice(&SCREEN_BPP.to_le_bytes());
    // ffFormat = kfDirect | kfDirect565
    buf.extend_from_slice(&0x40_0010u32.to_le_bytes());
    ctx.cpu.write_mem(sret, &buf)?;
    Ok(DispatchOutcome::ReturnedR0(sret))
}
