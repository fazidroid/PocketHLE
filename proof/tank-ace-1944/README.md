# Tank Ace 1944 rendering proof

This capture is a headless PocketHLE run of the uploaded Windows Mobile / Pocket PC CAB `Tank_Ace-spaces.im-48c88aa3da65.cab`.

- Target: ARM legacy WinCE executable `tank.exe`, 240×320 portrait.
- Result: the emulator reached the game rendering path; `frame_counter` advanced from 0 to 6 and the final run reported `frame_counter=6`.
- `gameplay.png` is the rendered gameplay surface.
- `blank-before-first-frame.png` is the initial framebuffer and is included only to make the transition explicit.

The run was performed with the release CLI and Unicorn ARM backend. The host environment has no Windows Phone / Windows Mobile emulator, so this is an emulator-side rendering proof, not a native device screenshot.
