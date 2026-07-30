# Asphalt 2 3D audio verification

The audio path is exercised by the Motorola Q9 CAB supplied for this repository. The run reached the game loop and produced these successful guest calls:

- `waveOutOpen`: PCM, 22050 Hz, mono, 16-bit; `CALLBACK_THREAD`, thread 2;
- four `waveOutWrite` calls, 8 bytes each, return code `0`;
- `waveOutPause` and `waveOutRestart`, both return code `0`;
- the run did not crash or hit an unimplemented API.

The detailed capture log is `asphalt-q9-audio.log`.

The verification host is headless and has no available ALSA output device, so cpal reports `device.default_output_config() failed`. Therefore this artifact proves that PocketHLE accepts the game's PCM stream and routes it into the audio engine, but it is not a claim of audible playback from this container. Audible playback still needs to be checked on a machine with an output device or on the Android frontend, whose `AudioTrack` path drains the same PCM tap.
