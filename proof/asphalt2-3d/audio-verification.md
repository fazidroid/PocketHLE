# Asphalt 2 3D audio verification

The attached trace and WAV were produced from the SPV C600 CAB supplied for this repository.

Command:

```sh
pockethle -v run Asphalt_2_3D__SPV_C600_-49eeec247f64.cab \
  --cpu unicorn --screen 320x240 \
  --dump-audio-to proof/asphalt2-3d/asphalt-spv-audio.wav \
  --trace-json proof/asphalt2-3d/asphalt-spv-audio-trace.jsonl \
  --max-slices 100000 --message-budget 500
```

The run completed cleanly and rendered the game framebuffer. The trace contains the dispatched WinCE API calls; the WAV is the host-side PCM capture emitted by the emulator's audio pipeline. The current SPV startup path did not submit a `waveOutWrite` buffer before the deterministic frame stop, so this artifact is a regression fixture for the actual `PlaySound` WAV decoder and capture path, not a claim that this one short startup run proves continuous Asphalt music.
