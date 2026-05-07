//! Hekkus Sound System (`hss.dll`).
//!
//! HSS is a freeware C++ audio mixer commonly bundled with Pocket PC
//! games. The JumpyBall test ROM uses the C++ classes
//! `hssSpeaker`, `hssSound`, `hssMusic` directly. We register stubs
//! by their MSVC-mangled names so dispatch matches without us doing
//! demangling at runtime. Each stub returns success and discards
//! audio data.

use pocket_kernel::{DispatchOutcome, KernelError};

use crate::{CallCtx, WinCeDispatcher};

pub fn register(d: &mut WinCeDispatcher) {
    let dll = "hss.dll";
    // C++ ctors/dtors and member functions on the ARM ABI receive
    // `this` in R0 and (for ctors) must return it. Returning a fixed
    // `1` makes the caller treat `1` as a valid object pointer and
    // it then dereferences it on the next instruction.
    let identity_stubs = [
        "??0hssSound@@QAA@XZ",
        "??1hssSound@@UAA@XZ",
        "??0hssMusic@@QAA@XZ",
        "??1hssMusic@@UAA@XZ",
        "??0hssSpeaker@@QAA@XZ",
        "??1hssSpeaker@@UAA@XZ",
    ];
    for f in identity_stubs {
        d.register_handler(dll, f, this_returning);
    }
    let success_stubs = [
        "?volume@hssSound@@QAAXI@Z",
        "?loop@hssSound@@QAAX_N@Z",
        "?load@hssSound@@QAAHPBG@Z",
        "?volume@hssMusic@@QAAXI@Z",
        "?loop@hssMusic@@QAAX_N@Z",
        "?load@hssMusic@@QAAHPBG@Z",
        "?open@hssSpeaker@@QAAHII_NII@Z",
        "?volumeSounds@hssSpeaker@@QAAXI@Z",
        "?volumeSounds@hssSpeaker@@QAAIXZ",
        "?volumeMusics@hssSpeaker@@QAAXI@Z",
        "?volumeMusics@hssSpeaker@@QAAIXZ",
        "?stopSounds@hssSpeaker@@QAAXXZ",
        "?stopMusics@hssSpeaker@@QAAXXZ",
        "?playSound@hssSpeaker@@QAAHPAVhssSound@@I@Z",
        "?playMusic@hssSpeaker@@QAAHPAVhssMusic@@I@Z",
    ];
    for f in success_stubs {
        d.register_handler(dll, f, ok);
    }
}

fn ok(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}

/// Constructor stub: zeroes out a small block at `this` (so the
/// caller's object isn't full of stack garbage) and returns `this`.
fn this_returning(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let this_ptr = ctx.arg_u32(0)?;
    if this_ptr != 0 {
        let zeroes = [0u8; 256];
        // Best-effort: if `this` lands in unmapped territory we just
        // skip — the constructor is a no-op anyway in that case.
        let _ = ctx.cpu.write_mem(this_ptr, &zeroes);
    }
    Ok(DispatchOutcome::ReturnedR0(this_ptr))
}
