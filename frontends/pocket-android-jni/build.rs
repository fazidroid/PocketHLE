// Build glue for `pocket-android-jni` cross-compiles.
//
// `unicorn-engine-sys`'s `build.rs` unconditionally emits
// `cargo:rustc-link-lib=pthread` and `cargo:rustc-link-lib=m` for any
// non-MSVC target, but Android's bionic libc provides the pthread API
// directly — there is no separate `libpthread.so` shipped with the NDK.
// The linker therefore fails with `unable to find library -lpthread`.
//
// To satisfy the `-lpthread` flag without touching the upstream crate,
// drop an empty `libpthread.a` next to a `cargo:rustc-link-search`
// path. The link succeeds and the actual `pthread_*` symbols are
// resolved against `libc.so` (bionic) at load time.
//
// `libm.a` is shipped by NDK r26 so it does not need a stub.
//
// On non-Android targets this build script is a no-op.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("-linux-android") {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let stub_dir = out_dir.join("android-link-stubs");
    fs::create_dir_all(&stub_dir).expect("create android-link-stubs dir");

    // Empty `ar` archive: just the 8-byte magic. `ld.lld` accepts this as
    // a valid archive that contributes zero symbols, so the `-lpthread`
    // flag is satisfied and pthread symbols come from `libc.so` at load.
    let empty_archive: &[u8] = b"!<arch>\n";
    let pthread_path = stub_dir.join("libpthread.a");
    fs::write(&pthread_path, empty_archive).expect("write libpthread.a stub");

    println!("cargo:rustc-link-search=native={}", stub_dir.display());
}
