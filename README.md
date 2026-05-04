# PocketHLE

> High-level emulator for Windows Mobile / Pocket PC games.
> Inspired by [touchHLE](https://github.com/touchHLE/touchHLE) (iPhone OS) and
> [EKA2L1](https://github.com/EKA2L1/EKA2L1) (Symbian).

PocketHLE focuses on running individual Pocket PC 2002 / 2003 / Windows Mobile
5/6 games without emulating an entire Windows CE kernel. Like touchHLE, it
loads a real game executable, runs the ARM code in a CPU emulator, and
implements the system DLLs (`coredll`, `aygshell`, `gx`, `hss`, ...) on the
host side so the game thinks it's running on a real Pocket PC.

The first target ROM is a small physics game called **JumpyBall** (provided
by the user, ARM PE32, Windows CE 5 GUI). All work is driven from the public
APIs that this game uses.

> **Status:** early — the ARM CPU runs, the loader works, system-DLL imports
> are intercepted and traced, and a handful of `coredll` functions are
> implemented (`memcpy`, `memset`, `GetTickCount`, ...). There is no rendering
> backend yet. See [Roadmap](#roadmap).

[Russian (Русский) version → `README.ru.md`](README.ru.md)

---

## Table of contents

- [Features](#features)
- [Architecture](#architecture)
- [Building on Linux](#building-on-linux)
- [Trying it on the JumpyBall test ROM](#trying-it-on-the-jumpyball-test-rom)
- [Building for Android](#building-for-android)
- [Roadmap](#roadmap)
- [Project layout](#project-layout)
- [Comparison to other emulators](#comparison-to-other-emulators)
- [License](#license)

## Features

- **CAB extractor** — unpacks Windows Mobile `.CAB` archives. Auto-detects the
  largest ARM `.exe` inside.
- **PE32 loader** — supports the WinCE flavour of `IMAGE_FILE_MACHINE_ARM`
  (`0x01c0`), parses imports by ordinal *and* name, lays out sections in the
  emulated address space.
- **Pluggable CPU backend** — a tiny `Cpu` trait with two implementations:
  - `stub` — software-only, no real instruction decoding, used for tests.
  - `unicorn` — wraps [Unicorn Engine 2](https://www.unicorn-engine.org/);
    runs real ARM machine code at native-ish speed.
- **HLE thunk dispatcher** — every imported symbol is rewritten in the IAT
  to point at a synthetic 4-byte page. The CPU gets a code hook on each
  thunk; on hit, the host's [`WinCeDispatcher`] looks up a Rust handler and
  resumes execution at `LR`.
- **Ordinal → name resolver** — partial maps for `coredll.dll` and
  `aygshell.dll` shipped as JSON data files. The resolver kicks in
  automatically so logs say `PeekMessageW` instead of `ord 266`.
- **Linux CLI frontend** with `pe-info`, `unpack-cab`, `inspect-cab` and
  `run` subcommands.
- **Android skeleton** — Gradle project that consumes the emulator core via
  cargo-ndk + JNI (`SurfaceView`-based UI is a TODO).

## Architecture

```
+-----------+    +-----------+    +---------+    +----------+
| pocket-cli|    |pocket-     |    | pocket- |    |pocket-   |
| (CLI)     |    | android    |    | core    |--->| pocket-pe|
+-----+-----+    | (Gradle)   |    +----+----+    +----------+
      |          +-----+------+         |
      v                |                v
+-----+----------------+----------------+----+
|             pocket-kernel                  |   process model, IAT thunks
|  +--------------+   +--------------------+ |
|  |pocket-cpu    |   |pocket-winceapi     | |   coredll / aygshell /
|  | (Unicorn)    |   |  (HLE handlers)    | |   gx / hss handlers
|  +--------------+   +--------------------+ |
+--------------------------------------------+
```

When `Emulator::run()` is invoked:

1. `pocket-cab` unpacks the user's `.CAB` and identifies the game `.exe`.
2. `pocket-pe` parses the PE, builds a list of `LoadedSection`s and the
   `Vec<ImportSymbol>` describing every imported function.
3. `pocket-kernel::Process::map_into` creates the address space:
   - maps each section,
   - allocates a `THUNK_REGION_BASE` pool of 4-byte stubs (`bx lr`),
   - patches the IAT so every imported symbol points at one of those
     stubs,
   - registers a code hook on every stub.
4. `pocket-kernel::run_main_loop` enters Unicorn at the entry point. As soon
   as the guest jumps through any IAT entry, Unicorn returns control;
   `WinCeDispatcher::dispatch` looks up the symbol and invokes the matching
   Rust handler, then resumes the guest at `LR`.

This is exactly the design touchHLE uses for Objective-C runtime calls and
Foundation, just adapted to Win32-style import tables.

## Building on Linux

```bash
# 1. Toolchain
rustup default stable               # 1.85+ recommended

# 2. Native deps (Ubuntu / Debian)
sudo apt install -y cmake build-essential pkg-config libclang-dev

# 3. Build everything (stub CPU only — fast, ~30 s)
cargo build --release --workspace

# 4. Build the CLI with the real ARM CPU backend (~3 minutes first time;
#    Unicorn Engine is built from source).
cargo build --release -p pocket-cli --features unicorn

# 5. Run tests
cargo test --workspace
```

The resulting binary lives at `target/release/pockethle`.

## Trying it on the JumpyBall test ROM

```bash
# Inspect a CAB without unpacking permanently
pockethle inspect-cab ~/JumpyBallPPC.cab

# Or unpack manually first:
pockethle unpack-cab ~/JumpyBallPPC.cab /tmp/jumpy
pockethle pe-info     /tmp/jumpy/JUMPYB~1.002

# Run the game (CPU = unicorn). Without the feature flag, the run
# subcommand still loads the PE and prints the import table, but does
# not interpret any instructions.
pockethle -v run /tmp/jumpy/JUMPYB~1.002 \
    --cpu unicorn \
    --max-slices 200 --instructions-per-slice 100000
```

The first time you run this, expect to see lines like:

```
[INFO  pocket_kernel] entering emulated main: entry=0x000247c8, stack_top=0x60000000
[WARN  pocket_winceapi] unimplemented call -> COREDLL.dll!__chkstk
[WARN  pocket_winceapi] unimplemented call -> COREDLL.dll!CreateDirectoryW
[WARN  pocket_winceapi] unimplemented call -> COREDLL.dll!Rectangle
...
```

Each `unimplemented call` line is a clue: that API needs a real handler in
`crates/pocket-winceapi/src/coredll.rs` (or wherever appropriate) before the
game can progress. See [Roadmap](#roadmap).

## Building for Android

The Android frontend lives in [`frontends/pocket-android`](frontends/pocket-android).
It depends on:

- Android Studio Iguana (or any AGP 8.4+ install)
- Android NDK r26 or newer
- [`cargo-ndk`](https://github.com/bbqsrc/cargo-ndk) (`cargo install cargo-ndk`)

Build:

```bash
# From the repo root, cross-compile the core for the four
# Android ABIs we ship (arm64-v8a is the realistic target).
cargo ndk -t arm64-v8a -t armeabi-v7a -o frontends/pocket-android/app/src/main/jniLibs \
    build --release -p pocket-cli

# Then open frontends/pocket-android in Android Studio and run on a
# device or emulator.
```

> The Android UI is currently a stub `MainActivity` that just calls into
> JNI to print the emulator's banner. A real `SurfaceView`-backed render
> loop is on the roadmap.

## Roadmap

The order below roughly matches what JumpyBall calls into during its first
hundred milliseconds:

1. **CRT prologue** — `__chkstk`, `_setjmp`, `longjmp`, `_except_handler3`.
2. **Window setup** — `RegisterClassW`, `CreateWindowExW`, `ShowWindow`,
   `SHFullScreen`.
3. **Resource loading** — `FindResourceW`, `LoadResource`, `LockResource`,
   `CreateFileW`, `ReadFile`.
4. **GDI** — `BeginPaint`, `EndPaint`, `BitBlt`, `Rectangle`, `FillRect`,
   `CreateCompatibleDC/Bitmap`, `SelectObject`, `DeleteObject`. We will
   implement this as a software-rasterised framebuffer that gets blitted
   straight to the host's window.
5. **GAPI** — already stubbed in `pocket-winceapi/src/gx.rs`. Hook up the
   `GXBeginDraw` framebuffer to a host-side window (SDL2 on Linux,
   `SurfaceView` on Android).
6. **Audio** — replace the HSS stubs with real SDL2 / OpenSL ES playback.
7. **Input** — translate host keyboard / touch events to Pocket PC
   `WM_KEYDOWN` / `WM_LBUTTONDOWN` and pump them into the message queue.

Each milestone is intended to land as a separate small PR.

## Project layout

```
crates/
  pocket-cab/        Microsoft Cabinet extractor
  pocket-pe/         WinCE PE32 loader (ARM / x86 / Thumb)
  pocket-cpu/        Cpu trait + stub + unicorn backends
  pocket-kernel/     Address space, IAT thunks, dispatcher loop
  pocket-winceapi/   coredll / aygshell / gx (GAPI) / hss handlers
  pocket-core/       Top-level Emulator that frontends drive
frontends/
  pocket-cli/        Linux command-line tool (`pockethle`)
  pocket-android/    Gradle project that wraps the core via JNI
data/
  ordinals/          JSON ordinal -> name maps for coredll, aygshell
docs/
  architecture/      Design notes
  api-stubs/         Per-API specification of what stubs need to do
```

## Comparison to other emulators

| Project              | Target OS                       | Approach                  | Status                                  |
|----------------------|---------------------------------|---------------------------|-----------------------------------------|
| `touchHLE`           | iPhone OS 2.x / 3.x             | High-level (HLE)          | Plays a handful of OpenGL ES games      |
| `EKA2L1`             | Symbian S60v3 / S60v5           | Mostly HLE, partial LLE   | Plays many Symbian games                |
| Microsoft Device Emu | Windows CE 5 / 6                | Full LLE (closed source)  | Discontinued                            |
| **PocketHLE** (this) | Windows Mobile 5 / 6 / Pocket PC| HLE                       | Loads + boots CRT for JumpyBall         |

PocketHLE deliberately copies touchHLE's layout — small Rust workspace,
HLE-first — because it scales well: every new game adds a few more API
implementations rather than years of low-level reverse engineering.

## License

Dual licensed under [Apache-2.0](LICENSE-APACHE) **OR** [MIT](LICENSE-MIT) at
your option.

PocketHLE itself does not contain or distribute any copyrighted Microsoft
code, Pocket PC system DLL, or game asset. Users supply their own legally
obtained `.CAB` files. The system-DLL stubs are clean-room reimplementations
based on publicly documented APIs and ordinal numbers.
