# Asphalt 2 3D audio verification

This proof was generated from the supplied Motorola Q9 CAB with the PocketHLE CLI after the audio mixer/backend changes.

Command profile: `--cpu unicorn --screen 320x240 --dump-audio-to ... --dump-frames-to ... --max-frames 900 --message-budget 100000`.

The run verified continuous streaming rather than only the startup burst:

- `waveOutOpen`: PCM, 22050 Hz, mono, 16-bit, `CALLBACK_THREAD`;
- `waveOutPause` / `waveOutRestart` completed successfully;
- repeated `waveOutWrite` calls continued after the initial four buffers, with 2,756-byte buffers and advancing playback cursors;
- the emulator reached `frame_counter=1800` and exited cleanly;
- captured guest audio is a valid 22050 Hz mono PCM WAV, 1.625578 seconds;
- `asphalt-q9-audio-proof.mp4` contains 900 rendered frames and the captured audio track (AAC, 22050 Hz mono).

The verification container has no ALSA output device, so cpal logs the expected graceful fallback and the video is assembled from the guest framebuffer plus the captured PCM stream. This is an audio-path proof; audible speaker playback still needs a host with an available output device or the Android `AudioTrack` frontend.
