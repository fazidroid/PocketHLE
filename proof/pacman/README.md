# Native ARM Windows Mobile rendering proof

This is a headless PocketHLE run of the native ARM Windows Mobile / Pocket PC Pac-Man CAB used as a regression target for the message-pump and framebuffer path.

- Target: legacy ARM PE32 executable, 240×320 portrait.
- Result: the emulator reached the rendering path and advanced `frame_counter` from 0 to 5359 before exiting cleanly.
- `pacman-gameplay.png` is a PNG conversion of the eighth captured RGB565 framebuffer snapshot.
- The run used the release CLI and Unicorn ARM backend with an extended synthetic message budget (`5000`) and wrote eight changed-frame snapshots.

The screenshot is emulator-side proof. A native Windows Mobile SDK/device emulator was not available in the host environment, so this is not a screenshot from an actual handset.
