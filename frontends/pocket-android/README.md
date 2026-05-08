# PocketHLE — Android frontend (skeleton)

A minimal Android Gradle project that wraps the PocketHLE core through JNI.
At this stage it does **not** render the game; it only proves that the core
crate cross-compiles to `aarch64-linux-android` via
[`cargo-ndk`](https://github.com/bbqsrc/cargo-ndk) and can be loaded by an
Android `Activity`.

## Prerequisites

- Android Studio Iguana (AGP 8.4+) **or** standalone Gradle 8.x with
  `local.properties` pointing at an Android SDK.
- Android NDK r26+.
- `cargo install cargo-ndk` (the cross-compile helper).

## Building the native library

```bash
# From the repo root:
cargo ndk -t arm64-v8a -t armeabi-v7a -o frontends/pocket-android/app/src/main/jniLibs \
    build --release -p pocket-android-jni
```

This drops `libpockethle_jni.so` under
`frontends/pocket-android/app/src/main/jniLibs/<abi>/`.

> **CPU backend on Android.** The Android crate currently builds with
> the trace-only stub CPU only. The `unicorn` feature is *not* in the
> default set because `unicorn-engine-sys 2.1.5` ships QEMU's
> autoconf-style `qemu/configure`, which detects the build host's CPU
> instead of the cross-target's CPU and tries to compile the i386 TCG
> JIT backend with NDK clang (which then chokes on QEMU's x86 cpuid
> intrinsics). The desktop/CLI frontends keep `unicorn` on by default
> because they target the same CPU as the build host. Turning real
> ARM emulation back on for Android is tracked separately and will
> require either a forked unicorn build script or a newer
> unicorn-engine release that exposes `--cpu` to the QEMU
> auto-detection.

## Building the APK

Inside `frontends/pocket-android`:

```bash
./gradlew assembleDebug
```

> **Note:** No Gradle wrapper jar is committed yet — run
> `gradle wrapper --gradle-version 8.7` once locally to generate it before
> the first build.
