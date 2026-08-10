# JumpyBall (PocketNew): sound

JumpyBall ships music and sound effects and played neither. The game is
a GAPI title that drives all of its audio through `hss.dll` — the Hekkus
Sound System — and `crates/pocket-winceapi/src/hss.rs` was registration
without implementation: every entry point returned success and no PCM
ever reached the audio engine. The game had no way to tell.

## Run

```sh
RUST_LOG=pocket_winceapi::hss=debug ./target/release/pockethle -v run \
  /tmp/jb-lib/games/jumpyballppc/extracted/JumpyBall.exe \
  --rom-dir /tmp/jb-lib/games/jumpyballppc/extracted \
  --rom-prefix '\Program Files\PocketNew\JumpyBall\' \
  --module-path '\Program Files\PocketNew\JumpyBall\JumpyBall.exe' \
  --message-budget 0 --max-frames 8 --max-slices 6000000 \
  --key enter --key 1:enter --tap 1:120,200 --tap 2:120,240 --tap 3:120,160 \
  --dump-frames-to /tmp/jb-final --dump-audio-to /tmp/jb-final.wav \
  --trace-json /tmp/jb-final.jsonl
```

Set up the guest tree with
`POCKETHLE_LIBRARY=/tmp/jb-lib ./target/release/pockethle import JumpyBallPPC.cab`,
which restores the CAB's 8.3 names (`00music1.006`) to real ones
(`Musics/music1.tkm`). Note `unpack-cab` takes its output directory
positionally, not as `--out`.

`exit=0`, `frame_counter=11`, 8 of 8 captured frames distinct, **zero
"unimplemented call" warnings** — down from 7 at baseline, which were
exactly the `?load@hssSound@@QAAHPBG@Z` × 5 and
`?load@hssMusic@@QAAHPBG@Z` × 2 the game could not do without.

## Frames

| File | What it shows |
| --- | --- |
| `01-pocketnew-splash.png` | PocketNew publisher splash |
| `02-gameplay.png` | In-game: the ball on the track, HUD score and timer |
| `03-level-select.png` | Level select — Level 1 unlocked, "Are you ready? Good luck!" |

The frames are here to show the run reaches real game states while audio
plays. Sound is not visible in a screenshot; the evidence for it is the
capture and the trace below.

## Root cause

Four names in the game's import table had no handler at all:

* `?load@hssSound@@QAAHPBG@Z` and `?load@hssMusic@@QAAHPBG@Z` — the
  `const wchar_t*` overloads. The file registered the `PAX_N`
  (`void*, bool`) overloads instead. Overloads mangle differently;
  registering one is not registering the other.
* `?volumeSounds@hssSpeaker@@QAAIXZ` and
  `?volumeMusics@hssSpeaker@@QAAIXZ` — the *getters*. Only the `XI@Z`
  setters were registered. The run below never calls either getter, so
  these two cost no warnings and fixing them changed nothing I can show;
  the game imports them, and an import with no handler only warns when
  it is reached, so leaving them out would strand whichever path does
  read a volume back.

Everything else was a stub returning success, which is the worse
failure: the game proceeds as though it has audio, and nothing in the
trace says otherwise.

## What it took

**A decoder.** HSS takes a *filename*, not PCM — unlike `waveOut`, where
the guest has already decoded. So `hss.rs` now decodes both formats HSS
accepts. `.wav` reuses `coredll`'s `decode_pcm_wave` rather than growing
a second set of rounding bugs. `.tkm` is a renamed Protracker module, so
`decode_clip` tries both decoders against the *content*; the extension is
not load-bearing on a device and it isn't here either.

**A Protracker renderer** — `crates/pocket-kernel/src/tracker.rs`, a
parser and mixer for `M.K.` modules with no guest-memory awareness, so it
is testable on its own. Two bugs worth recording:

* The sample number is split across two nibbles — high in cell byte 0,
  low in the *high* nibble of byte 2. My own test fixture got this wrong
  and selected sample 16, an empty slot, which renders silence.
* Source samples carry a large DC offset: means of +65 to +99 out of
  ±128 across these three tracks. An Amiga's AC-coupled output discarded
  it; a modern DAC does not. Rendering music1 with the offset intact
  pinned that render's mean at +6515 out of ±32767 and left only 74
  zero-crossings/sec — a signal that barely crosses zero is one a DAC
  reproduces as a thump rather than a note. A DC blocker on the mix bus
  fixed it: the capture below means +0.1 over its first 40 s, with 2,649
  zero-crossings/sec. The two rates are measured on different signals —
  one render versus the whole capture — so read them as "pinned off zero"
  against "crossing freely", not as a ratio.

**Voice groups**, so `stopMusics` stops the music without cutting the
effects still playing. JumpyBall calls it on every level change; it
fires 4 times in this 8-frame run.

**A decode cache.** The game re-loads `mainmenu.tkm` into a fresh object
three times before the first level. Rendering is the expensive half of
`load`, so decodes are cached by path behind an `Arc`: 4 music loads in
this run, 2 renders. The `Arc` also removes a 13 MB copy per `play`.

## Trace

All 16 HSS entry points the game calls now dispatch — 48 calls, none
unimplemented:

```
 6  ??0hssSound@@QAA@XZ              4  ?loop@hssMusic@@QAAX_N@Z
 5  ?load@hssSound@@QAAHPBG@Z        4  ?volume@hssMusic@@QAAXI@Z
 5  ?loop@hssSound@@QAAX_N@Z         4  ?playMusic@hssSpeaker@@...
 4  ?stopMusics@hssSpeaker@@QAAXXZ   3  ?volume@hssSound@@QAAXI@Z
 4  ?load@hssMusic@@QAAHPBG@Z        2  ?volumeSounds@hssSpeaker@@QAAXI@Z
 2  ?stopSounds@hssSpeaker@@QAAXXZ   1  ?playSound@hssSpeaker@@...
 1  ??0hssSpeaker / ??0hssMusic / ?open@hssSpeaker / ?volumeMusics
```

Decoded assets, from the debug log:

```
hssSpeaker::open(22050 Hz, 16 bit, mono)
hssSound::load("...\Sounds\sblam.wav")   -> 3844 samples @ 11025 Hz x1
hssSound::load("...\Sounds\smenu.wav")   ->  849 samples @ 11025 Hz x1
hss: rendered module "delicate 0ooz!" -> 148.5s
hssMusic::load("...\Musics\mainmenu.tkm") -> 6547968 samples @ 22050 Hz x2
hss: play "...\Musics\mainmenu.tkm" (6547968 samples, loop=true, gain=0.31)
hss: rendered module "pip 8.0" -> 53.8s
hss: play "...\Musics\music1.tkm" (2370816 samples, loop=true, gain=0.62)
hss: play "...\Sounds\smenu.wav" (849 samples, loop=false, gain=0.50)
```

`souspont.wav`, `sexpl.wav` and `smiss.wav` report "file not found" and
that is correct: the CAB ships 9 files and those three are not among
them. The game asks for effects it does not carry, and `load` returning
0 is the honest answer.

## Capture

`/tmp/jb-final.wav`: 2 ch, 22050 Hz, 499.2 s, peak 20228, 100% of
samples non-zero over the first 40 s, mean +0.1.

**A `--dump-audio-to` capture records at submission time, not as a
real-time mixdown.** It shows what the guest submitted, in submission
order, so the four `playMusic` calls appear as four full-length tracks
laid end to end — 499 s of WAV out of an 8-frame run. It answers "did
PCM reach the engine", not "what would a user hear".

## Tests

`crates/pocket-kernel/src/tracker.rs`:

* `a_protracker_module_parses_its_header`
* `a_file_without_the_magic_is_not_a_module`
* `rendering_produces_non_silent_stereo_audio`
* `a_looping_order_table_still_terminates`
* `detuning_moves_pitch_by_the_expected_amount`
* `a_dc_offset_sample_renders_centred`

`crates/pocket-winceapi/src/hss.rs`:

* `a_wav_and_a_module_both_decode`
* `an_unrecognised_file_decodes_to_nothing`
* `loading_the_same_file_twice_decodes_it_once` — deletes the file
  between the two loads, so a second success can only come from the
  cache

## Status

Music and effects both decode, play, loop and stop through the real HSS
API surface; the run exits cleanly with no unimplemented calls. Verified
at the API and PCM level — the trace shows the calls and the capture
shows the samples. Nobody has listened to it on this machine; there is
no audio device here.
