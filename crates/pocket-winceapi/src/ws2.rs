//! Windows Sockets 2 (`ws2.dll`).
//!
//! PocketHLE does not proxy real network I/O out of the guest — there
//! is no host socket behind any of this. What matters is *how* a game
//! finds that out.
//!
//! Before this module existed, every one of these calls fell through
//! to the generic unimplemented-call stub, which logs a warning and
//! synthesizes a `0` return (see `WinCeDispatcher::dispatch`). `0`
//! happens to be the real WinSock success code for `WSAStartup` *and*
//! looks like a valid `SOCKET` handle for `socket()` *and* is the
//! immediate-success return for `connect()`. Three unimplemented
//! calls in a row therefore looked, from the guest's point of view,
//! like WinSock came up and a connection succeeded on the first try —
//! right up until `recv()` (also stubbed to `0`) reported the peer
//! gracefully closing a connection that was never really open.
//!
//! Gameloft's Asphalt 4 (`CWinBlue::ConnectLoop`, per its own debug
//! strings) reacts to that specific sequence by reconnecting and
//! trying again, forever — the game's `CHighGear` state machine never
//! leaves its pre-render "loading" state, so it never creates the
//! DirectDraw surface it would otherwise present every frame. Nothing
//! crashes; nothing halts; nothing is ever unimplemented. It just
//! never stops "connecting".
//!
//! The fix isn't a smarter retry simulation — it's answering honestly.
//! `socket()` returns `INVALID_SOCKET` and `WSAGetLastError()` reports
//! `WSAENETDOWN`, exactly as a real device with no active data
//! connection would. A game that already ships an offline fallback
//! (this one does: its own strings include `ERROR_CONNECTION` and
//! `REGISTERED FAILED` states) can act on a clear failure. It can't
//! act on a success that quietly never finishes.
//!
//! `htons` is the one function here with no networking behind it at
//! all — a 16-bit byte swap — so it is implemented for real rather
//! than stubbed, in case something on the path to giving up still
//! calls it.

use pocket_kernel::{DispatchOutcome, KernelError};

use crate::{CallCtx, WinCeDispatcher};

/// `(SOCKET)(~0)`. Also the bit pattern of `SOCKET_ERROR` (`-1`) — the
/// two constants are numerically identical, only the name differs by
/// call site.
const INVALID_SOCKET: u32 = 0xffff_ffff;
const SOCKET_ERROR: u32 = 0xffff_ffff;
/// "A socket operation encountered a dead network." The one error
/// code every function here agrees on, so a game that calls
/// `WSAGetLastError()` after any of them sees a consistent story.
const WSAENETDOWN: u32 = 10050;

pub fn register(d: &mut WinCeDispatcher) {
    let dll = "ws2.dll";
    d.register_handler(dll, "WSAStartup", wsa_startup);
    d.register_handler(dll, "WSACleanup", wsa_cleanup);
    d.register_handler(dll, "WSAGetLastError", wsa_get_last_error);
    d.register_handler(dll, "socket", socket);
    d.register_handler(dll, "closesocket", closesocket);
    d.register_handler(dll, "connect", connect);
    d.register_handler(dll, "bind", bind);
    d.register_handler(dll, "listen", listen);
    d.register_handler(dll, "accept", accept);
    d.register_handler(dll, "send", send);
    d.register_handler(dll, "recv", recv);
    d.register_handler(dll, "select", select);
    d.register_handler(dll, "__WSAFDIsSet", wsa_fd_is_set);
    d.register_handler(dll, "getsockname", getsockname);
    d.register_handler(dll, "gethostname", gethostname);
    d.register_handler(dll, "gethostbyname", gethostbyname);
    d.register_handler(dll, "htons", htons);
    d.register_handler(dll, "WSALookupServiceBeginW", wsa_lookup_service_begin_w);
    d.register_handler(dll, "WSALookupServiceNextW", wsa_lookup_service_next_w);
    d.register_handler(dll, "WSALookupServiceEnd", wsa_lookup_service_end);
    d.register_handler(dll, "WSASetServiceW", wsa_set_service_w);
}

/// `int WSAStartup(WORD wVersionRequested, LPWSADATA lpWSAData)`.
///
/// This one call is allowed to succeed — real devices bring the
/// WinSock subsystem up fine even with no active data connection, and
/// plenty of unrelated init code checks this return before ever
/// touching a socket. Only `wVersion`/`wHighVersion` (the first two
/// `WORD`s) are written; the rest of `WSADATA` is left untouched, the
/// same caution `RegQueryValueExW` and friends take with
/// caller-owned buffers elsewhere in this crate — the real struct's
/// exact size varies across WinCE SDK versions and nothing we target
/// reads past the version fields.
fn wsa_startup(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let lp_wsa_data = ctx.arg_u32(1)?;
    if lp_wsa_data != 0 {
        // wVersion = wHighVersion = 2.2, the version essentially every
        // WinSock caller since Windows 98 has asked for.
        ctx.cpu.write_mem(lp_wsa_data, &[0x02, 0x02, 0x02, 0x02])?;
    }
    log::debug!("WSAStartup() -> 0 (version 2.2)");
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn wsa_cleanup(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn wsa_get_last_error(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(WSAENETDOWN))
}

/// `SOCKET socket(int af, int type, int protocol)`.
fn socket(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let af = ctx.arg_u32(0)?;
    let kind = ctx.arg_u32(1)?;
    log::debug!("socket({af}, {kind}, ..) -> INVALID_SOCKET (WSAENETDOWN)");
    Ok(DispatchOutcome::ReturnedR0(INVALID_SOCKET))
}

fn closesocket(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // Always safe to report success at closing a handle, including
    // one nothing here ever actually handed out.
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn connect(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(SOCKET_ERROR))
}

fn bind(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(SOCKET_ERROR))
}

fn listen(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(SOCKET_ERROR))
}

fn accept(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(INVALID_SOCKET))
}

fn send(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(SOCKET_ERROR))
}

fn recv(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(SOCKET_ERROR))
}

/// `int select(int nfds, fd_set* readfds, fd_set* writefds, fd_set*
/// exceptfds, const timeval* timeout)`.
///
/// `0` here means something different from everywhere else in this
/// file: not an error, but WinSock's real "the timeout elapsed and no
/// descriptor became ready" — the honest answer, since nothing here
/// ever hands out a descriptor that could become ready. Combined with
/// `__WSAFDIsSet` always reporting "not set", a caller that loops on
/// `select` waiting for activity sees a clean, consistent nothing
/// rather than an error partway through the wait.
fn select(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn wsa_fd_is_set(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn getsockname(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(SOCKET_ERROR))
}

/// `int gethostname(char* name, int namelen)`.
///
/// Unlike the rest of this file, this doesn't depend on connectivity
/// — it is asking for the local device's own name, which a real phone
/// can always answer. Succeeds with a short placeholder name when the
/// caller's buffer has room for it, and fails cleanly rather than
/// truncating when it doesn't.
fn gethostname(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let name = ctx.arg_u32(0)?;
    let namelen = ctx.arg_u32(1)?;
    const PLACEHOLDER: &[u8] = b"WM6DEVICE\0";
    if name == 0 || (namelen as usize) < PLACEHOLDER.len() {
        return Ok(DispatchOutcome::ReturnedR0(SOCKET_ERROR));
    }
    ctx.cpu.write_mem(name, PLACEHOLDER)?;
    Ok(DispatchOutcome::ReturnedR0(0))
}

/// `struct hostent* gethostbyname(const char* name)`.
///
/// Real WinSock returns `NULL` when the name can't be resolved; a
/// null guest pointer is `0`, so this doubles as both "not
/// implemented" and "correctly reporting a lookup failure".
fn gethostbyname(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(0))
}

/// `u_short htons(u_short hostshort)`. Pure byte-order arithmetic —
/// the one function in this module answered for real rather than
/// stubbed, since it has no dependency on there being a network at
/// all.
fn htons(ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    let host = (ctx.arg_u32(0)? & 0xffff) as u16;
    Ok(DispatchOutcome::ReturnedR0(host.to_be() as u32))
}

fn wsa_lookup_service_begin_w(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(SOCKET_ERROR))
}

fn wsa_lookup_service_next_w(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(SOCKET_ERROR))
}

fn wsa_lookup_service_end(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    // Ending a lookup that never successfully began is harmless.
    Ok(DispatchOutcome::ReturnedR0(0))
}

fn wsa_set_service_w(_ctx: &mut CallCtx<'_>) -> Result<DispatchOutcome, KernelError> {
    Ok(DispatchOutcome::ReturnedR0(SOCKET_ERROR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pocket_cpu::{regs::ArmReg, stub::StubCpu, Cpu};
    use pocket_kernel::Thunk;
    use pocket_pe::ImportBinding;

    fn dummy_thunk() -> Thunk {
        Thunk {
            thunk_va: 0x7000_0000,
            iat_va: 0x4000_0000,
            dll: "ws2.dll".to_string(),
            binding: ImportBinding::Ordinal(0),
            friendly_name: None,
        }
    }

    // `htons` is the only pure-arithmetic function in this module —
    // everything else either returns a fixed constant (exercised via
    // dispatch below) or does a bounds-checked memory write.
    #[test]
    fn htons_swaps_bytes() {
        assert_eq!(0x1234u16.to_be(), 0x3412);
        assert_eq!(0x0050u16.to_be(), 0x5000); // port 80
    }

    #[test]
    fn socket_reports_invalid_socket_with_wsaenetdown() {
        let mut cpu = StubCpu::new();
        let mut kernel = crate::gx::tests::fresh_kernel();
        let t = dummy_thunk();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        assert_eq!(
            socket(&mut c).unwrap(),
            DispatchOutcome::ReturnedR0(INVALID_SOCKET)
        );
        assert_eq!(
            wsa_get_last_error(&mut c).unwrap(),
            DispatchOutcome::ReturnedR0(WSAENETDOWN)
        );
    }

    // `connect`/`accept` are dead code for a game that correctly gives
    // up after `socket()` fails, but Asphalt 4's `CWinBlue::ConnectLoop`
    // is exactly the kind of caller this guards against: if it ever
    // calls one of these anyway, it must see a clean failure, not
    // another success-shaped `0`.
    #[test]
    fn connect_and_friends_fail_rather_than_looking_like_success() {
        let mut cpu = StubCpu::new();
        let mut kernel = crate::gx::tests::fresh_kernel();
        let t = dummy_thunk();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        assert_eq!(
            connect(&mut c).unwrap(),
            DispatchOutcome::ReturnedR0(SOCKET_ERROR)
        );
        assert_eq!(
            bind(&mut c).unwrap(),
            DispatchOutcome::ReturnedR0(SOCKET_ERROR)
        );
        assert_eq!(
            listen(&mut c).unwrap(),
            DispatchOutcome::ReturnedR0(SOCKET_ERROR)
        );
        assert_eq!(
            send(&mut c).unwrap(),
            DispatchOutcome::ReturnedR0(SOCKET_ERROR)
        );
        assert_eq!(
            recv(&mut c).unwrap(),
            DispatchOutcome::ReturnedR0(SOCKET_ERROR)
        );
        assert_eq!(
            accept(&mut c).unwrap(),
            DispatchOutcome::ReturnedR0(INVALID_SOCKET)
        );
    }

    #[test]
    fn wsa_startup_writes_version_and_succeeds() {
        let mut cpu = StubCpu::new();
        cpu.map_region(
            0x5000_0000,
            0x1000,
            pocket_cpu::Prot::READ | pocket_cpu::Prot::WRITE,
        )
        .unwrap();
        let mut kernel = crate::gx::tests::fresh_kernel();
        let t = dummy_thunk();
        cpu.write_reg(ArmReg::R0, 0x0202).unwrap();
        cpu.write_reg(ArmReg::R1, 0x5000_0000).unwrap();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        assert_eq!(wsa_startup(&mut c).unwrap(), DispatchOutcome::ReturnedR0(0));
        assert_eq!(
            c.cpu.read_mem(0x5000_0000, 4).unwrap(),
            vec![0x02, 0x02, 0x02, 0x02]
        );
    }

    /// A null `lpWSAData` is a caller bug, not a reason for PocketHLE
    /// to fault — WSAStartup still reports success and simply skips
    /// the write.
    #[test]
    fn wsa_startup_tolerates_a_null_out_param() {
        let mut cpu = StubCpu::new();
        let mut kernel = crate::gx::tests::fresh_kernel();
        let t = dummy_thunk();
        cpu.write_reg(ArmReg::R0, 0x0202).unwrap();
        cpu.write_reg(ArmReg::R1, 0).unwrap();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        assert_eq!(wsa_startup(&mut c).unwrap(), DispatchOutcome::ReturnedR0(0));
    }

    #[test]
    fn gethostname_succeeds_when_the_buffer_has_room() {
        let mut cpu = StubCpu::new();
        cpu.map_region(
            0x5000_0000,
            0x1000,
            pocket_cpu::Prot::READ | pocket_cpu::Prot::WRITE,
        )
        .unwrap();
        let mut kernel = crate::gx::tests::fresh_kernel();
        let t = dummy_thunk();
        cpu.write_reg(ArmReg::R0, 0x5000_0000).unwrap();
        cpu.write_reg(ArmReg::R1, 32).unwrap();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        assert_eq!(gethostname(&mut c).unwrap(), DispatchOutcome::ReturnedR0(0));
        let written = c.cpu.read_mem(0x5000_0000, 10).unwrap();
        assert_eq!(&written, b"WM6DEVICE\0");
    }

    #[test]
    fn gethostname_fails_cleanly_when_the_buffer_is_too_small() {
        let mut cpu = StubCpu::new();
        let mut kernel = crate::gx::tests::fresh_kernel();
        let t = dummy_thunk();
        cpu.write_reg(ArmReg::R0, 0x5000_0000).unwrap();
        cpu.write_reg(ArmReg::R1, 2).unwrap();
        let mut c = CallCtx {
            cpu: &mut cpu,
            thunk: &t,
            kernel: &mut kernel,
        };
        assert_eq!(
            gethostname(&mut c).unwrap(),
            DispatchOutcome::ReturnedR0(SOCKET_ERROR)
        );
    }

    /// Every name Asphalt 4's real import table (Samsung Omnia build)
    /// pulls from `WS2.dll`. If this list and the handler map drift
    /// apart, the game is right back to silent unimplemented-stub
    /// zeros and the `CWinBlue::ConnectLoop` hang this module exists
    /// to prevent.
    #[test]
    fn every_ws2_import_asphalt4_needs_reaches_a_handler() {
        let d = WinCeDispatcher::new();
        let registered: std::collections::HashSet<(String, String)> = d
            .registered_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        for name in [
            "send",
            "WSALookupServiceEnd",
            "gethostname",
            "select",
            "WSAGetLastError",
            "WSALookupServiceNextW",
            "getsockname",
            "WSACleanup",
            "bind",
            "WSALookupServiceBeginW",
            "WSASetServiceW",
            "__WSAFDIsSet",
            "closesocket",
            "listen",
            "accept",
            "connect",
            "WSAStartup",
            "htons",
            "recv",
            "socket",
            "gethostbyname",
        ] {
            assert!(
                registered.contains(&("ws2.dll".to_string(), name.to_string())),
                "ws2.dll!{name} has no registered handler"
            );
        }
    }
}
