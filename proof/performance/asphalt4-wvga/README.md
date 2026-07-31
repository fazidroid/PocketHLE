# Asphalt 4 WVGA performance capture

The CLI release A/B run used the uploaded CAB, Unicorn ARM backend, `--screen 480x800`, `--max-frames 7`, and identical runtime settings. Each run was repeated three times; the first rendered frame was captured as a PPM for visual comparison.

| Build | Wall time to 7 rendered frames | Median |
|---|---:|---:|
| Baseline (`origin/main`) | 3.295s, 2.573s, 2.609s | 2.609s |
| Working tree (`4ffff39`) | 2.588s, 2.595s, 2.582s | 2.588s |

Measured median ratio: **1.008x** (0.8% faster).

This proof validates the Android presentation-side changes: frame polling is 16 ms instead of 33 ms, and the Kotlin renderer reuses its `Bitmap`, pixel array, and RGBA decode buffer instead of allocating them on every frame. The CLI emulator core was unchanged by this final commit.

The Android FPS overlay is the authoritative device-side measurement. These host CLI timings are not a claim of 30–60 FPS; the uploaded CAB still needs an Android device run to confirm the final FPS.
