# Xtrakt (Xperia X1): OpenGL ES proof

Second game booted through PocketHLE's OpenGL ES 1.x layer, and the
first that needed compressed textures. Xtrakt shipped with the Xperia
X1, whose Adreno GPU makes ATC the natural texture format — the game
uploads its entire atlas set as `GL_ATC_RGBA_EXPLICIT_ALPHA_AMD`.

## Run

```sh
cargo run -p pocket-cli -- \
  run /tmp/xtrakt-rom/Xtrakt.exe \
  --module-path '\Application\Xtrakt.exe' \
  --rom-dir /tmp/xtrakt-rom \
  --screen 480x800 --max-frames 40 \
  --dump-frames-to /tmp/xs-final --dump-frame-stride 39
```

## What was blocking it

The game exited after a single frame. Its outer run loop at `0x0b8b6c`
opens by testing a global at `0x00129f5c` and short-circuits the whole
loop when it is set; the pump call at `0x0b8bc0` was therefore never
reached. That global is latched by the startup path at `0x02bf84`,
immediately after a `glGetError` at `0x02bf78` that follows the
`glCompressedTexImage2D` at `0x02bf68`. Rejecting the ATC formats was
enough to make Xtrakt decide the device could not run it and quit.

With ATC decoding in place `frame_counter` goes from 1 to ~470 over a
40-frame run: the game reaches its real main loop and keeps rendering.

## Two fixes behind these frames

**ATC decoding** (`pocket-gles/src/texture.rs`). `ATC_RGB` is 8 bytes
per 4x4 block, the two `ATC_RGBA_*` variants 16, with the alpha block
first as in DXT3/DXT5. Bit 15 of the first endpoint selects between
ATC's two interpolation modes rather than acting as DXT1's punch-through
alpha flag. The formats are now advertised through
`GL_NUM_COMPRESSED_TEXTURE_FORMATS`, `GL_COMPRESSED_TEXTURE_FORMATS`
and the `GL_EXTENSIONS` string, because Xtrakt checks before uploading.

**Bilinear filtering** (`pocket-gles/src/texture.rs`). `Filter::Linear`
was parsed, stored and never used — the rasterizer only ever called
`sample_nearest`, so every `GL_LINEAR` texture in every game was being
point-sampled. `sample_linear` and the `sample` dispatcher fix that.
This is what the text in these frames gained.

## Frames

| File | What it shows |
| --- | --- |
| `xtrakt-warning-dialog.png` | The startup warning dialog as rendered into the 480x800 surface |
| `xtrakt-warning-dialog-upright.png` | The same frame rotated 90° for review |
| `font-atlas-atc-decoded.png` | Top-left quarter of the game's 1024x1024 font atlas, alpha channel, decoded from `ATC_RGBA_EXPLICIT_ALPHA` |

Like COD2, Xtrakt draws landscape content into a portrait surface and
expects the handset held sideways, so the `-upright` file is a rotation
of the same pixels rather than a separate capture.

## On the look of the text

The glyph edges are chunky, and that is the asset rather than the
decoder. Dumping the atlas with `POCKETHLE_DUMP_TEXTURES=<dir>` shows
its alpha channel uses only nine distinct levels — zero, then 136
through 255 in steps of 17. The 4-bit explicit-alpha nibbles 1 to 7
never occur anywhere in the 1 MiB block stream, so the atlas simply
carries no coverage below 53% and the font has hard edges by
construction. The glyph shapes in the frames match the atlas exactly.

The same dump also shows Xtrakt passes an `imageSize` of twice the
block-stream length for every explicit-alpha upload; the tail is all
zeroes and is ignored.

## Debug hooks added along the way

`POCKETHLE_DUMP_TEXTURES=<dir>` writes each uploaded texture as a PPM
plus a greyscale PPM of its alpha, and the undecoded compressed blocks
as `.raw`. A wrong compressed-format decode shows up on screen only as
art that is slightly off, which is near-impossible to judge from a
composited frame; the atlas on its own is decisive.
