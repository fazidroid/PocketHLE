//! Pocket PC shell extensions (`aygshell.dll`).

use pocket_kernel::{DispatchOutcome, KernelError};

use crate::{CallCtx, WinCeDispatcher};

pub fn register(d: &mut WinCeDispatcher) {
    let dll = "aygshell.dll";
    for f in [
        "SHFullScreen",
        "SHCreateMenuBar",
        "SHHandleWMActivate",
        "SHHandleWMSettingChange",
        "SHInitDialog",
        "SHSipPreference",
    ] {
        d.register_handler(dll, f, ok);
    }
}

fn ok(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(1))
}
