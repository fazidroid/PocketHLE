# Toy Golf (Fathammer): boot to main menu

Toy Golf is a GDI/GAPI title — no GL at all — that drives its audio from
a dedicated worker thread and its display from a `WM_PAINT`-only idle
loop. It needed four fixes, all in
`crates/pocket-winceapi/src/coredll.rs`, and each one hid the next.

## Run

```sh
./target/release/pockethle run /tmp/tg-install/toygolf_qvga.exe \
  --rom-dir /tmp/tg-install --rom-prefix '\Program Files\Game\' \
  --module-path '\Program Files\Game\toygolf_qvga.exe' \
  --screen 320x240 --message-budget 0 \
  --key 10:enter --tap 20:160,120 --key 30:enter \
  --dump-frames-to /tmp/tg-key --dump-frame-stride 20 \
  --max-frames 30 --max-slices 80000000
```

`exit=0`, `frame_counter=1164`, 26 of the 30 captured frames distinct,
zero "unimplemented call" warnings. 231 assets load out of
`minigolf_dell.cfl` — all nine levels plus every sound.

Note `--rom-prefix`: `--module-path` has to sit *inside* the mounted
prefix or every relative `fopen` the game does misses.

## Frames

| File | What it shows |
| --- | --- |
| `01-fathammer-splash.png` | Fathammer publisher splash |
| `02-main-menu.png` | Main menu — logo, ball, SINGLE PLAYER / MULTIPLAYER / OPTIONS / QUIT |
| `03-main-menu-later.png` | The same menu 11 captures later, still live |

## The four fixes

### `CreateThread` must return to its creator

The crash was `READ_UNMAPPED` at `0x00000038` — `ldr r1,[r5,#0x38]` with
`r5 = 0` at `pc=0x000d9d04`. `create_thread` was jumping straight into
the new thread's entry point, so the audio thread ran before its creator
had stored the mixer pointer it immediately dereferences.

Every new thread now parks at its entry point exactly the way
`CREATE_SUSPENDED` ones always did, and `CreateThread` returns the handle
to the creator. `CREATE_SUSPENDED` survives as the `started` flag, which
is the only difference left between the two paths.

### `CALLBACK_EVENT` has to actually signal

With the crash gone the game hung: 1,036,104 `waveOutWrite` calls and
exactly one "scheduling worker" line. `retire_wave_buffer` set
`WHDR_DONE` for every callback kind but only *delivered* Window, Thread
and Function callbacks — `WaveCallbackKind::Event` did nothing.

That event is the mixer thread's only back-pressure. Never setting it
makes the thread's wait fall through, so it refills as fast as the CPU
allows, never yields, and the main thread never draws again.

### An infinite `WaitForSingleObject` is a scheduling point

The other half of the same hang. A worker blocking forever has to yield;
`park_worker_and_reevaluate` re-parks it at *its own thunk* so the call
re-runs with its arguments intact when it resumes. Rewriting `r0` with a
return value the way `park_worker_at` does would make the resumed wait
look at the wrong handle. When the *main* thread is the waiter nothing
can signal the object from there, so the permissive `WAIT_OBJECT_0`
stays — honouring that wait deadlocks the process.

### `InvalidateRect` is not a frame

This was the black-frame root cause, and it was never a graphics bug.
Toy Golf calls `InvalidateRect` 99,490 times from its idle loop, and the
handler bumped `frame_counter`. The CLI's dump hook fires on that
counter, so the entire `--max-frames` budget went to identical black
startup frames and the run ended before the game drew anything. The
framebuffer had 153,600 non-zero pixels the whole time — content was
present, capture was pointed at the wrong moments.

`frame_counter` is the host's "new pixels are ready" signal. Anything
that moves it without changing pixels spends a frontend's frame budget.

## Also cleaned up

`SystemIdleTimerReset` (79,240 calls), `ImmGetContext`,
`ImmReleaseContext` and `ImmSetCompositionWindow` were unimplemented —
each call logged a warning and cost a host round-trip. They are constants
now, so the run is silent.

## Regression tests

In `coredll.rs`'s `mod tests`, each with a distinct `thunk_va` because
`resolve_handler` memoizes negative results by that address:

* `create_thread_returns_to_its_creator_instead_of_entering_the_thread`
* `a_suspended_thread_is_parked_but_not_yet_runnable`
* `a_drained_buffer_signals_its_callback_event`
* `an_infinite_wait_from_a_worker_yields_and_keeps_its_handle`
* `invalidate_rect_does_not_count_as_a_rendered_frame`

## Status

Boots, loads every asset, reaches and holds the main menu, exits
cleanly. Menu navigation and gameplay are not verified.
