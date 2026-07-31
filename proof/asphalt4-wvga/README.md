# Asphalt 4 WVGA proof

This directory records the headless Unicorn run of the uploaded Windows Mobile CAB `WVGA_A4-1601624865f3.cab`.

- CAB import selected `Asphalt4.exe`, restored the install paths, and ran the game at `480x800`.
- The guest opened two simultaneous `waveOut` devices: one for music and one for sound effects. The emulator now preserves the first device when the second opens instead of flushing its audio queue.
- The trace records two `waveOutOpen` calls, two `waveOutWrite` calls, then `waveOutReset` and `waveOutClose`.
- `asphalt4-audio-capture.wav` is 17.5 seconds of captured PCM at 22050 Hz mono. Verified amplitude: mean `-10.4 dB`, peak `-0.2 dB`; it is not silent.
- `asphalt4-audio-proof.mp4` contains the emulator video plus AAC audio. Verified amplitude: mean `-10.5 dB`, peak `-0.3 dB`.
- `asphalt4-audio-run.log` contains the exact run output and `asphalt4-audio-trace.jsonl` contains the API trace.
- `asphalt4-gameplay-proof.png` and the numbered PNG frames are the earlier graphics-only proof.

The video is an emulator proof on Linux, not a native Windows Mobile device run. A physical host audio device was unavailable in this environment, so the PCM capture and video audio track are the authoritative audio verification.
