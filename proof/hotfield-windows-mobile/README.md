# HotField Windows Mobile proof

The HotField CAB from the task reaches the PocketHLE rendering loop under the ARM Unicorn backend.

- Final run: exit status 0, `frame_counter=16`, clean emulator shutdown.
- `ai-tap-sequence.txt` (captured from `tools/ai-tap-sequence.py`): two synthetic taps queued at `(120,260)` and `(80,180)`; 16 framebuffer snapshots captured; no crash.
- `gameplay.png`: PNG conversion of the last captured 240x320 RGB565 framebuffer.
- `frames/`: selected raw PPM framebuffer snapshots.
