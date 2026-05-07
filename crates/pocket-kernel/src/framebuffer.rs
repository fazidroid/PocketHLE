//! Software framebuffer that backs both GDI (`BitBlt`, `FillRect`,
//! ...) and GAPI (`GXBeginDraw`/`GXEndDraw`) rendering.
//!
//! The Pocket PC reference target is a 240×320 portrait LCD with a
//! 16-bit-per-pixel RGB565 framebuffer. We keep that as the canonical
//! pixel format because:
//!
//! * GAPI is documented to expose RGB565 on the vast majority of
//!   Pocket PC devices, including the JumpyBall test ROM.
//! * GDI bitmaps created via `CreateCompatibleBitmap` are also 16 bpp
//!   on a 16 bpp screen, so memory-DC-to-screen `BitBlt` becomes a
//!   straight memcpy.
//!
//! All host-facing operations work on top of [`Framebuffer::pixels`],
//! a row-major byte buffer. `width * 2` is the row stride.

/// Width of the emulated Pocket PC LCD.
pub const FB_WIDTH: u32 = 240;
/// Height of the emulated Pocket PC LCD.
pub const FB_HEIGHT: u32 = 320;
/// Bits per pixel of the canonical framebuffer (RGB565).
pub const FB_BPP: u32 = 16;
/// Total framebuffer size in bytes.
pub const FB_BYTES: u32 = FB_WIDTH * FB_HEIGHT * (FB_BPP / 8);

/// A simple RGB565 framebuffer.
#[derive(Debug, Clone)]
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    /// Row-major RGB565 little-endian pixels. Length is
    /// `width * height * (bpp/8)`.
    pub pixels: Vec<u8>,
    /// Incremented every time the framebuffer is mutated. Hosts use
    /// it to decide whether they need to re-upload the surface.
    pub frame_counter: u64,
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new(FB_WIDTH, FB_HEIGHT)
    }
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bpp: FB_BPP,
            pixels: vec![0u8; (width * height * 2) as usize],
            frame_counter: 0,
        }
    }

    pub fn byte_size(&self) -> u32 {
        self.width * self.height * (self.bpp / 8)
    }

    pub fn stride_bytes(&self) -> u32 {
        self.width * (self.bpp / 8)
    }

    pub fn mark_dirty(&mut self) {
        self.frame_counter = self.frame_counter.wrapping_add(1);
    }

    /// Fill the entire framebuffer with one RGB565 value.
    pub fn fill(&mut self, color: u16) {
        let bytes = color.to_le_bytes();
        for chunk in self.pixels.chunks_exact_mut(2) {
            chunk[0] = bytes[0];
            chunk[1] = bytes[1];
        }
        self.mark_dirty();
    }

    /// Plot one pixel. Out-of-bounds is silently ignored to match
    /// GDI's clipping semantics.
    pub fn put_pixel(&mut self, x: i32, y: i32, color: u16) {
        if x < 0 || y < 0 || (x as u32) >= self.width || (y as u32) >= self.height {
            return;
        }
        let off = ((y as u32) * self.width + x as u32) as usize * 2;
        let bytes = color.to_le_bytes();
        self.pixels[off] = bytes[0];
        self.pixels[off + 1] = bytes[1];
        // Caller is expected to call mark_dirty() once per logical
        // operation; we don't bump it per pixel for performance.
    }

    /// Fill an axis-aligned rectangle. `(x, y, w, h)` is clipped to
    /// the framebuffer.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u16) {
        if w <= 0 || h <= 0 {
            return;
        }
        let (cx0, cy0, cx1, cy1) = clip_rect(x, y, w, h, self.width, self.height);
        if cx0 >= cx1 || cy0 >= cy1 {
            return;
        }
        let bytes = color.to_le_bytes();
        for row in cy0..cy1 {
            let off = (row * self.width + cx0) as usize * 2;
            let row_len = (cx1 - cx0) as usize;
            for i in 0..row_len {
                self.pixels[off + i * 2] = bytes[0];
                self.pixels[off + i * 2 + 1] = bytes[1];
            }
        }
        self.mark_dirty();
    }

    /// Draw a one-pixel-thick rectangle outline. Used by `Rectangle`
    /// after the interior is already filled.
    pub fn stroke_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u16) {
        if w <= 0 || h <= 0 {
            return;
        }
        for i in 0..w {
            self.put_pixel(x + i, y, color);
            self.put_pixel(x + i, y + h - 1, color);
        }
        for i in 0..h {
            self.put_pixel(x, y + i, color);
            self.put_pixel(x + w - 1, y + i, color);
        }
        self.mark_dirty();
    }

    /// Copy a sub-region of `src` into `(dx, dy)`. Clipping is applied
    /// independently on both source and destination axes.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_from(
        &mut self,
        dx: i32,
        dy: i32,
        sx: i32,
        sy: i32,
        w: i32,
        h: i32,
        src: &[u8],
        src_w: u32,
        src_h: u32,
    ) {
        if w <= 0 || h <= 0 || src_w == 0 || src_h == 0 {
            return;
        }
        let (sx0, sy0, sx1, sy1) = clip_rect(sx, sy, w, h, src_w, src_h);
        if sx0 >= sx1 || sy0 >= sy1 {
            return;
        }
        let dst_x0 = dx + (sx0 as i32 - sx);
        let dst_y0 = dy + (sy0 as i32 - sy);
        let copy_w = (sx1 - sx0) as i32;
        let copy_h = (sy1 - sy0) as i32;
        let (dx0, dy0, dx1, dy1) =
            clip_rect(dst_x0, dst_y0, copy_w, copy_h, self.width, self.height);
        if dx0 >= dx1 || dy0 >= dy1 {
            return;
        }
        let row_bytes = (dx1 - dx0) as usize * 2;
        let src_skip_x = (dx0 as i32 - dst_x0) as u32;
        let src_skip_y = (dy0 as i32 - dst_y0) as u32;
        let src_x0 = sx0 + src_skip_x;
        let src_y0 = sy0 + src_skip_y;
        for row in 0..(dy1 - dy0) {
            let dst_off = ((dy0 + row) * self.width + dx0) as usize * 2;
            let src_off = ((src_y0 + row) * src_w + src_x0) as usize * 2;
            if src_off + row_bytes > src.len() || dst_off + row_bytes > self.pixels.len() {
                continue;
            }
            self.pixels[dst_off..dst_off + row_bytes]
                .copy_from_slice(&src[src_off..src_off + row_bytes]);
        }
        self.mark_dirty();
    }

    /// Convert the framebuffer to a flat 0xRR,GG,BB,AA buffer for
    /// host-side display. Each pixel is decoded from RGB565 to
    /// 8-bit-per-channel and given full opacity.
    pub fn snapshot_rgba8888(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
        for chunk in self.pixels.chunks_exact(2) {
            let p = u16::from_le_bytes([chunk[0], chunk[1]]);
            let r5 = ((p >> 11) & 0x1f) as u8;
            let g6 = ((p >> 5) & 0x3f) as u8;
            let b5 = (p & 0x1f) as u8;
            let r = (r5 << 3) | (r5 >> 2);
            let g = (g6 << 2) | (g6 >> 4);
            let b = (b5 << 3) | (b5 >> 2);
            out.push(r);
            out.push(g);
            out.push(b);
            out.push(0xff);
        }
        out
    }

    /// Pack the framebuffer as a P6 PPM image. P6 is binary, one
    /// triplet per pixel, no compression — perfect for headless
    /// proof-of-rendering captures without pulling in `png`.
    pub fn snapshot_ppm(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + (self.width * self.height * 3) as usize);
        out.extend_from_slice(format!("P6\n{} {}\n255\n", self.width, self.height).as_bytes());
        for chunk in self.pixels.chunks_exact(2) {
            let p = u16::from_le_bytes([chunk[0], chunk[1]]);
            let r5 = ((p >> 11) & 0x1f) as u8;
            let g6 = ((p >> 5) & 0x3f) as u8;
            let b5 = (p & 0x1f) as u8;
            out.push((r5 << 3) | (r5 >> 2));
            out.push((g6 << 2) | (g6 >> 4));
            out.push((b5 << 3) | (b5 >> 2));
        }
        out
    }
}

/// Pack an `(R, G, B)` triple into RGB565.
pub fn pack_rgb565(r: u8, g: u8, b: u8) -> u16 {
    let r5 = (r as u16 >> 3) & 0x1f;
    let g6 = (g as u16 >> 2) & 0x3f;
    let b5 = (b as u16 >> 3) & 0x1f;
    (r5 << 11) | (g6 << 5) | b5
}

/// Convert a Win32 `COLORREF` (`0x00BBGGRR`) to RGB565.
pub fn colorref_to_rgb565(colorref: u32) -> u16 {
    let r = (colorref & 0xff) as u8;
    let g = ((colorref >> 8) & 0xff) as u8;
    let b = ((colorref >> 16) & 0xff) as u8;
    pack_rgb565(r, g, b)
}

/// Intersect `[x, x+w) × [y, y+h)` with `[0, max_w) × [0, max_h)`.
/// Returns `(x0, y0, x1, y1)` in `u32`.
fn clip_rect(x: i32, y: i32, w: i32, h: i32, max_w: u32, max_h: u32) -> (u32, u32, u32, u32) {
    let x0 = x.max(0).min(max_w as i32) as u32;
    let y0 = y.max(0).min(max_h as i32) as u32;
    let x1 = (x + w).max(0).min(max_w as i32) as u32;
    let y1 = (y + h).max(0).min(max_h as i32) as u32;
    (x0, y0, x1, y1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_then_count_changes() {
        let mut fb = Framebuffer::new(4, 4);
        let c0 = fb.frame_counter;
        fb.fill(pack_rgb565(255, 0, 0));
        assert_ne!(fb.frame_counter, c0);
        let snap = fb.snapshot_rgba8888();
        assert_eq!(snap[0], 0xff);
        assert_eq!(snap[1], 0x00);
        assert_eq!(snap[2], 0x00);
        assert_eq!(snap[3], 0xff);
    }

    #[test]
    fn fill_rect_clips_to_bounds() {
        let mut fb = Framebuffer::new(4, 4);
        let red = pack_rgb565(255, 0, 0);
        fb.fill_rect(2, 2, 100, 100, red);
        // Only the bottom-right 2x2 should be red.
        let rgba = fb.snapshot_rgba8888();
        let pix = |x: u32, y: u32| -> [u8; 4] {
            let off = ((y * 4 + x) * 4) as usize;
            [rgba[off], rgba[off + 1], rgba[off + 2], rgba[off + 3]]
        };
        assert_eq!(pix(0, 0), [0, 0, 0, 0xff]);
        assert_eq!(pix(2, 2)[0], 0xff);
        assert_eq!(pix(3, 3)[0], 0xff);
    }

    #[test]
    fn blit_copies_subregion() {
        let mut fb = Framebuffer::new(4, 4);
        let blue = pack_rgb565(0, 0, 255);
        let src: Vec<u8> = (0..(2 * 2)).flat_map(|_| blue.to_le_bytes()).collect();
        fb.blit_from(1, 1, 0, 0, 2, 2, &src, 2, 2);
        let rgba = fb.snapshot_rgba8888();
        let pix = |x: u32, y: u32| -> [u8; 4] {
            let off = ((y * 4 + x) * 4) as usize;
            [rgba[off], rgba[off + 1], rgba[off + 2], rgba[off + 3]]
        };
        assert_eq!(pix(0, 0)[2], 0x00);
        assert_eq!(pix(1, 1)[2], 0xff);
        assert_eq!(pix(2, 2)[2], 0xff);
    }

    #[test]
    fn ppm_snapshot_starts_with_header() {
        let fb = Framebuffer::new(2, 2);
        let ppm = fb.snapshot_ppm();
        assert!(ppm.starts_with(b"P6\n2 2\n255\n"));
        assert_eq!(ppm.len(), 11 + 12);
    }
}
