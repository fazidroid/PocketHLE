# Zenonia 1.6 Windows Mobile proof

The supplied `Zenonia_1.6-5217fd6b3f10.cab` was run with the ARM Unicorn backend through `tools/ai-tap-sequence.py`.

## Result

- Automated tap sequence: **PASS**
- Taps: `(120,160)`, `(120,220)`, `(120,175)`
- Captured frames: **40**
- Final `frame_counter`: **79
- Emulator exit: clean, status 0
- Final framebuffer: 240x320 RGB565, non-black gameplay scene

The initial failure was not the host rendering loop. Zenonia imports the ordinal-only Windows CE string APIs `StringCbCopyW`, `StringCbLengthW`, `StringCbCatW`, `StringCbCopyNW`, and `_wcslwr`. They were absent from the dispatcher, so startup string/path preparation returned zero values and the game stopped before its real screen. The patch implements bounded UTF-16 copy/concatenation/length and in-place lowercase behavior. It also reports the Windows CE 5.2 / Windows Mobile 6.1 version family expected by this title.

The older baseline run also reached a rendered frame (`frame_counter=458`) but stopped at the game's `Check window mobile version!` dialog; this is why the visible symptom was misleading. The fixed run reaches the rendered scene and continues through the synthetic taps.
