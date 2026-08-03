//! Texture objects and pixel-format conversion.
//!
//! OpenGL ES 1.1 accepts textures in five base formats crossed with
//! three packed 16-bit types. Windows Mobile games lean on the packed
//! types heavily because a 16-bit texel halves both the file size and
//! the memory bandwidth — Call of Duty 2 uploads 85 textures, most of
//! them `GL_UNSIGNED_SHORT_5_6_5` or `GL_UNSIGNED_SHORT_4_4_4_4`.
//!
//! We decode everything to straight RGBA8888 at upload time so the
//! rasterizer has a single format to sample, and so a future host-GL
//! backend can hand the data to `glTexImage2D` unchanged.

use crate::consts::*;

/// Texture filtering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Linear,
}

/// Texture coordinate wrap mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    Repeat,
    ClampToEdge,
}

/// A single texture object, as created by `glGenTextures`.
#[derive(Debug, Clone)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    /// Decoded RGBA8888 texels, row-major, `width * height * 4` bytes.
    /// Empty until the first successful `glTexImage2D`.
    pub rgba: Vec<u8>,
    pub min_filter: Filter,
    pub mag_filter: Filter,
    pub wrap_s: Wrap,
    pub wrap_t: Wrap,
}

impl Default for Texture {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            rgba: Vec::new(),
            // GL defaults: min is mipmap-linear, mag is linear. We
            // collapse the mipmap modes to their base filter.
            min_filter: Filter::Linear,
            mag_filter: Filter::Linear,
            wrap_s: Wrap::Repeat,
            wrap_t: Wrap::Repeat,
        }
    }
}

impl Texture {
    /// Is this texture complete enough to sample?
    pub fn is_complete(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.rgba.len() >= (self.width * self.height * 4) as usize
    }

    /// Sample a texel with nearest-neighbour filtering.
    ///
    /// Returns premultiplied-by-nothing straight RGBA. Out-of-range
    /// coordinates are resolved through the wrap modes.
    pub fn sample_nearest(&self, s: f32, t: f32) -> [u8; 4] {
        if !self.is_complete() {
            // An incomplete texture samples as opaque white in our
            // pipeline, which makes `GL_MODULATE` a no-op and leaves
            // the underlying vertex colour visible instead of turning
            // the surface black.
            return [255, 255, 255, 255];
        }
        let x = Self::wrap_coord(s, self.width, self.wrap_s);
        let y = Self::wrap_coord(t, self.height, self.wrap_t);
        let idx = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[idx],
            self.rgba[idx + 1],
            self.rgba[idx + 2],
            self.rgba[idx + 3],
        ]
    }

    fn wrap_coord(v: f32, size: u32, wrap: Wrap) -> u32 {
        if size == 0 {
            return 0;
        }
        let scaled = v * size as f32;
        match wrap {
            Wrap::Repeat => {
                let m = (scaled.floor() as i64).rem_euclid(size as i64);
                m as u32
            }
            Wrap::ClampToEdge => {
                let c = scaled.floor();
                if c < 0.0 {
                    0
                } else if c >= size as f32 {
                    size - 1
                } else {
                    c as u32
                }
            }
        }
    }

    /// Apply a `glTexParameter*` value. Returns `false` if the
    /// enumerant is not one we recognise, which the caller reports as
    /// `GL_INVALID_ENUM`.
    pub fn set_parameter(&mut self, pname: u32, value: u32) -> bool {
        match pname {
            GL_TEXTURE_MIN_FILTER => {
                self.min_filter = match value {
                    GL_NEAREST | GL_NEAREST_MIPMAP_NEAREST | GL_NEAREST_MIPMAP_LINEAR => {
                        Filter::Nearest
                    }
                    GL_LINEAR | GL_LINEAR_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_LINEAR => {
                        Filter::Linear
                    }
                    _ => return false,
                };
                true
            }
            GL_TEXTURE_MAG_FILTER => {
                self.mag_filter = match value {
                    GL_NEAREST => Filter::Nearest,
                    GL_LINEAR => Filter::Linear,
                    _ => return false,
                };
                true
            }
            GL_TEXTURE_WRAP_S => match wrap_from_enum(value) {
                Some(w) => {
                    self.wrap_s = w;
                    true
                }
                None => false,
            },
            GL_TEXTURE_WRAP_T => match wrap_from_enum(value) {
                Some(w) => {
                    self.wrap_t = w;
                    true
                }
                None => false,
            },
            _ => false,
        }
    }
}

fn wrap_from_enum(value: u32) -> Option<Wrap> {
    match value {
        GL_REPEAT => Some(Wrap::Repeat),
        GL_CLAMP_TO_EDGE => Some(Wrap::ClampToEdge),
        _ => None,
    }
}

/// Number of bytes one texel occupies in the guest's buffer.
pub fn bytes_per_texel(format: u32, ty: u32) -> Option<usize> {
    match ty {
        GL_UNSIGNED_SHORT_5_6_5 | GL_UNSIGNED_SHORT_4_4_4_4 | GL_UNSIGNED_SHORT_5_5_5_1 => Some(2),
        GL_UNSIGNED_BYTE => match format {
            GL_ALPHA | GL_LUMINANCE => Some(1),
            GL_LUMINANCE_ALPHA => Some(2),
            GL_RGB => Some(3),
            GL_RGBA => Some(4),
            _ => None,
        },
        _ => None,
    }
}

/// Expand a 5-bit channel to 8 bits, replicating the high bits so that
/// `0b11111` maps to `255` rather than `248`.
#[inline]
fn expand5(v: u16) -> u8 {
    let v = (v & 0x1F) as u8;
    (v << 3) | (v >> 2)
}

#[inline]
fn expand6(v: u16) -> u8 {
    let v = (v & 0x3F) as u8;
    (v << 2) | (v >> 4)
}

#[inline]
fn expand4(v: u16) -> u8 {
    let v = (v & 0x0F) as u8;
    (v << 4) | v
}

/// Decode a guest texel buffer into RGBA8888.
///
/// Returns `None` if the format/type combination is not one OpenGL ES
/// 1.1 permits, which the caller reports as `GL_INVALID_ENUM`.
pub fn decode_to_rgba(
    data: &[u8],
    width: u32,
    height: u32,
    format: u32,
    ty: u32,
) -> Option<Vec<u8>> {
    let texel_bytes = bytes_per_texel(format, ty)?;
    let count = (width as usize).checked_mul(height as usize)?;
    let needed = count.checked_mul(texel_bytes)?;
    if data.len() < needed {
        return None;
    }
    let mut out = Vec::with_capacity(count * 4);
    match ty {
        GL_UNSIGNED_SHORT_5_6_5 => {
            for i in 0..count {
                let p = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                out.extend_from_slice(&[expand5(p >> 11), expand6(p >> 5), expand5(p), 255]);
            }
        }
        GL_UNSIGNED_SHORT_4_4_4_4 => {
            for i in 0..count {
                let p = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                out.extend_from_slice(&[
                    expand4(p >> 12),
                    expand4(p >> 8),
                    expand4(p >> 4),
                    expand4(p),
                ]);
            }
        }
        GL_UNSIGNED_SHORT_5_5_5_1 => {
            for i in 0..count {
                let p = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                let a = if p & 1 != 0 { 255 } else { 0 };
                out.extend_from_slice(&[expand5(p >> 11), expand5(p >> 6), expand5(p >> 1), a]);
            }
        }
        GL_UNSIGNED_BYTE => match format {
            GL_RGBA => out.extend_from_slice(&data[..needed]),
            GL_RGB => {
                for i in 0..count {
                    let s = i * 3;
                    out.extend_from_slice(&[data[s], data[s + 1], data[s + 2], 255]);
                }
            }
            GL_LUMINANCE => {
                for &l in data.iter().take(count) {
                    out.extend_from_slice(&[l, l, l, 255]);
                }
            }
            GL_LUMINANCE_ALPHA => {
                for i in 0..count {
                    let l = data[i * 2];
                    out.extend_from_slice(&[l, l, l, data[i * 2 + 1]]);
                }
            }
            GL_ALPHA => {
                for &a in data.iter().take(count) {
                    out.extend_from_slice(&[255, 255, 255, a]);
                }
            }
            _ => return None,
        },
        _ => return None,
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_expands_to_full_range() {
        // Pure white in 5:6:5 must decode to 255,255,255 — not
        // 248,252,248, which is what a naive `<< 3` shift produces and
        // which makes every bright surface visibly dingy.
        let white = 0xFFFFu16.to_le_bytes();
        let out = decode_to_rgba(&white, 1, 1, GL_RGB, GL_UNSIGNED_SHORT_5_6_5).unwrap();
        assert_eq!(out, vec![255, 255, 255, 255]);
    }

    #[test]
    fn rgb565_channel_order_is_r_g_b() {
        // 0xF800 = red only.
        let red = 0xF800u16.to_le_bytes();
        let out = decode_to_rgba(&red, 1, 1, GL_RGB, GL_UNSIGNED_SHORT_5_6_5).unwrap();
        assert_eq!(out, vec![255, 0, 0, 255]);
        // 0x001F = blue only.
        let blue = 0x001Fu16.to_le_bytes();
        let out = decode_to_rgba(&blue, 1, 1, GL_RGB, GL_UNSIGNED_SHORT_5_6_5).unwrap();
        assert_eq!(out, vec![0, 0, 255, 255]);
    }

    #[test]
    fn rgba4444_decodes_alpha() {
        // 0xF00F: red=F, g=0, b=0, a=F
        let px = 0xF00Fu16.to_le_bytes();
        let out = decode_to_rgba(&px, 1, 1, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4).unwrap();
        assert_eq!(out, vec![255, 0, 0, 255]);
    }

    #[test]
    fn rgba5551_alpha_is_one_bit() {
        let opaque = 0x0001u16.to_le_bytes();
        let out = decode_to_rgba(&opaque, 1, 1, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1).unwrap();
        assert_eq!(out[3], 255);
        let transparent = 0x0000u16.to_le_bytes();
        let out = decode_to_rgba(&transparent, 1, 1, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1).unwrap();
        assert_eq!(out[3], 0);
    }

    #[test]
    fn rgb_ubyte_gains_opaque_alpha() {
        let out = decode_to_rgba(&[10, 20, 30], 1, 1, GL_RGB, GL_UNSIGNED_BYTE).unwrap();
        assert_eq!(out, vec![10, 20, 30, 255]);
    }

    #[test]
    fn truncated_buffer_is_rejected() {
        // A 2×2 RGBA texture needs 16 bytes; 8 must not be read past.
        assert!(decode_to_rgba(&[0u8; 8], 2, 2, GL_RGBA, GL_UNSIGNED_BYTE).is_none());
    }

    #[test]
    fn unsupported_format_is_rejected() {
        assert!(decode_to_rgba(&[0u8; 4], 1, 1, 0x9999, GL_UNSIGNED_BYTE).is_none());
    }

    #[test]
    fn repeat_wraps_negative_coordinates() {
        let mut t = Texture {
            width: 4,
            height: 1,
            rgba: vec![0; 16],
            ..Default::default()
        };
        t.wrap_s = Wrap::Repeat;
        // -0.25 in a 4-texel row is texel 3, not a clamp to 0.
        assert_eq!(Texture::wrap_coord(-0.25, 4, Wrap::Repeat), 3);
        assert_eq!(Texture::wrap_coord(1.25, 4, Wrap::Repeat), 1);
    }

    #[test]
    fn clamp_to_edge_saturates() {
        assert_eq!(Texture::wrap_coord(-5.0, 4, Wrap::ClampToEdge), 0);
        assert_eq!(Texture::wrap_coord(99.0, 4, Wrap::ClampToEdge), 3);
    }

    #[test]
    fn incomplete_texture_samples_white() {
        let t = Texture::default();
        assert_eq!(t.sample_nearest(0.5, 0.5), [255, 255, 255, 255]);
    }

    #[test]
    fn mipmap_min_filters_collapse_to_base_filter() {
        let mut t = Texture::default();
        assert!(t.set_parameter(GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR));
        assert_eq!(t.min_filter, Filter::Linear);
        assert!(t.set_parameter(GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST));
        assert_eq!(t.min_filter, Filter::Nearest);
    }

    #[test]
    fn unknown_parameter_is_rejected() {
        let mut t = Texture::default();
        assert!(!t.set_parameter(0x9999, GL_LINEAR));
        assert!(!t.set_parameter(GL_TEXTURE_MAG_FILTER, 0x9999));
    }
}
