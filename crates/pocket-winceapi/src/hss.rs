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
    let stubs = [
        // hssSound
        "??0hssSound@@QAA@XZ",
        "??1hssSound@@UAA@XZ",
        "?volume@hssSound@@QAAXI@Z",
        "?loop@hssSound@@QAAX_N@Z",
        "?load@hssSound@@QAAHPBG@Z",
        // hssMusic
        "??0hssMusic@@QAA@XZ",
        "??1hssMusic@@UAA@XZ",
        "?volume@hssMusic@@QAAXI@Z",
        "?loop@hssMusic@@QAAX_N@Z",
        "?load@hssMusic@@QAAHPBG@Z",
        // hssSpeaker
        "??0hssSpeaker@@QAA@XZ",
        "??1hssSpeaker@@UAA@XZ",
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
    for f in stubs {
        d.register_handler(dll, f, ok);
    }
}

fn ok(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}
