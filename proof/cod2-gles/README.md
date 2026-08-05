# Call of Duty 2 (Windows Mobile): OpenGL ES proof

First game booted through PocketHLE's OpenGL ES 1.x layer. COD2 imports
`libgles_cl.dll` — 40 symbols, every one by ordinal, no names in the PE
import directory — so the whole run goes through the Common-Lite
ordinal table in `pocket-gles`.

## Run

```sh
cargo run --release -p pocket-cli -- \
  run /tmp/cod2-install/cod2_gles.exe \
  --rom-dir /tmp/cod2-install \
  --module-path '\Program Files\COD2\cod2_gles.exe' \
  --key 1:enter --key 2:enter --key 3:enter \
  --dump-frames-to /tmp/cod2-frames --max-frames 5 \
  --max-slices 8000000
```

The game ships its own software `libGLES_CM.dll`; PocketHLE shadows it
so the guest binds to our implementation instead. `GetProcAddress` on
`libGLES_CL.dll` / `libGLES_CM.dll` resolves against per-DLL export
tables keyed by fake HMODULEs, which is what makes the shadowing work
for guests that resolve dynamically.

## Frames

| File | What it shows |
| --- | --- |
| `cod2-activision-splash.png` | Activision logo |
| `cod2-aspyr-splash.png` | Aspyr logo |
| `cod2-ionfx-splash.png` | ionfx v1.0 "gold engine" logo |
| `cod2-title-screen.png` | Call of Duty 2 title screen, as rendered into the 240×320 portrait surface |
| `cod2-title-screen-landscape.png` | The same frame rotated upright for review |

COD2 renders landscape content into the portrait EGL surface and relies
on the device being held sideways, so the raw frame is rotated 90°.
The `-landscape` file is a convenience rotation of the same pixels, not
a separate capture.

### Splash screens are modal input gates

The function at `0x421c0` is not the main loop — it is a splash gate.
`r11` is set to 1 at `0x421c8`, `0x42218: ands r11, r11, #2` clears it,
and `0x42568: cmp r11,#0 / bne 0x4220c` spins while it is non-zero. The
body accepts WM_KEYDOWN/WM_MOUSEMOVE/WM_QUIT and rejects virtual keys
below VK_RETURN (`0x424b0: cmp r11,#13 / blt`). Each splash therefore
needs exactly one input to advance, which is why the run feeds three
Enters and no more — extra presses reach the title menu and eventually
select its exit item, and the game shuts down cleanly (33
`glDeleteTextures`, `eglDestroyContext`, `eglTerminate`,
`waveOutClose`) instead of staying on screen.

## What the GLES layer actually served

From `--trace-json` over the five captured frames: 1,498,199 dispatched
API calls total, of which 598 are GL/EGL across 30 distinct entry
points.

```
  244  glTexImage2D        8  glTexCoordPointer     1  eglGetDisplay
  165  glTexParameterx     8  glColorPointer        1  eglInitialize
   41  glBindTexture       8  glDrawElements        1  eglGetConfigs
   34  glGenTextures       7  glLoadIdentity        1  eglChooseConfig
   19  glMatrixMode        6  glFrustumx            1  eglCreateContext
   10  glEnable            5  glClearColorx         1  eglCreateWindowSurface
    9  glDisable           5  glClear               1  eglMakeCurrent
                           5  eglSwapBuffers        1  glViewport
                           5  glLoadMatrixx         1  glHint
                           4  glVertexPointer       1  glTexEnvx
                           3  glEnableClientState   1  glShadeModel
                                                    1  glCullFace
```

The 244 `glTexImage2D` calls are 34 textures plus their mipmap levels.
Everything is fixed-point (`glFrustumx`, `glClearColorx`,
`glLoadMatrixx`) — the Common-Lite profile has no float entry points,
so `GLfixed` 16.16 conversion is on the critical path for every matrix
and colour the game sets.

`eglSwapBuffers` is the only point where GL output becomes visible in
the RGB565 framebuffer; the five swaps correspond one-to-one with the
five captured frames.

## Status

Boot, EGL setup, texture upload and indexed draws work end to end.
This is the title screen, not gameplay — the in-game renderer is the
next target.
