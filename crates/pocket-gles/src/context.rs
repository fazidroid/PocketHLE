//! The OpenGL ES 1.1 context: state machine, vertex assembly, drawing.
//!
//! This is the layer the DLL entry points call into. It owns every piece
//! of GL state (matrix stacks, enables, texture objects, array
//! pointers), assembles primitives, and hands triangles to
//! [`crate::raster`].
//!
//! Vertex and texture data live in *guest* memory, which this crate has
//! no way to read directly. Callers therefore supply a [`GuestMemory`]
//! implementation; the emulator backs it with the CPU's address space
//! and the tests back it with a plain byte vector.

use crate::consts::*;
use crate::fixed;
use crate::matrix::{self, Matrix4, MatrixMode, MatrixStack};
use crate::raster::{
    self, BlendFactor, CompareFunc, CullMode, FogMode, FrontFace, PipelineState, RenderTarget,
    TexEnvMode, Vertex,
};
use crate::texture::{self, Texture};
use std::collections::HashMap;

/// Read access to the guest's address space.
pub trait GuestMemory {
    /// Copy `len` bytes starting at `addr` into `out`. Returns `false`
    /// if any part of the range is unmapped, in which case the caller
    /// skips the draw rather than rendering garbage.
    fn read(&mut self, addr: u32, len: usize, out: &mut Vec<u8>) -> bool;
}

/// A `gl*Pointer` client array binding.
#[derive(Debug, Clone, Copy)]
pub struct ArrayPointer {
    pub enabled: bool,
    /// Components per element (2, 3 or 4).
    pub size: u32,
    /// One of `GL_BYTE`, `GL_SHORT`, `GL_FIXED`, `GL_FLOAT`,
    /// `GL_UNSIGNED_BYTE`.
    pub ty: u32,
    /// Byte stride between elements. Zero means tightly packed.
    pub stride: u32,
    /// Guest virtual address of the first element.
    pub pointer: u32,
}

impl Default for ArrayPointer {
    fn default() -> Self {
        Self {
            enabled: false,
            size: 4,
            ty: GL_FLOAT,
            stride: 0,
            pointer: 0,
        }
    }
}

impl ArrayPointer {
    /// Bytes one component occupies, or `None` for an unsupported type.
    fn component_bytes(&self) -> Option<u32> {
        Some(match self.ty {
            GL_BYTE | GL_UNSIGNED_BYTE => 1,
            GL_SHORT | GL_UNSIGNED_SHORT => 2,
            GL_FLOAT | GL_FIXED => 4,
            _ => return None,
        })
    }

    /// Distance in bytes between consecutive elements.
    fn element_stride(&self) -> Option<u32> {
        let packed = self.component_bytes()?.checked_mul(self.size)?;
        Some(if self.stride == 0 {
            packed
        } else {
            self.stride
        })
    }
}

/// Decode one component out of a raw little-endian buffer.
///
/// `normalize` maps integer types into `[0, 1]`, which is what GL does
/// for colours but *not* for positions or texture coordinates.
fn decode_component(bytes: &[u8], ty: u32, normalize: bool) -> f32 {
    match ty {
        GL_FLOAT => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        GL_FIXED => fixed::to_f32(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        GL_BYTE => {
            let v = bytes[0] as i8 as f32;
            if normalize {
                (v / 127.0).clamp(-1.0, 1.0)
            } else {
                v
            }
        }
        GL_UNSIGNED_BYTE => {
            let v = bytes[0] as f32;
            if normalize {
                v / 255.0
            } else {
                v
            }
        }
        GL_SHORT => {
            let v = i16::from_le_bytes([bytes[0], bytes[1]]) as f32;
            if normalize {
                (v / 32767.0).clamp(-1.0, 1.0)
            } else {
                v
            }
        }
        GL_UNSIGNED_SHORT => {
            let v = u16::from_le_bytes([bytes[0], bytes[1]]) as f32;
            if normalize {
                v / 65535.0
            } else {
                v
            }
        }
        _ => 0.0,
    }
}

/// The full GL ES 1.1 context.
pub struct Context {
    // ---- transform state ----
    pub matrix_mode: MatrixMode,
    pub modelview: MatrixStack,
    pub projection: MatrixStack,
    pub texture_matrix: MatrixStack,

    // ---- per-vertex defaults, used when the array is disabled ----
    pub current_color: [f32; 4],
    pub current_texcoord: [f32; 2],
    pub current_normal: [f32; 3],

    // ---- client arrays ----
    pub vertex_array: ArrayPointer,
    pub color_array: ArrayPointer,
    pub texcoord_array: ArrayPointer,
    pub normal_array: ArrayPointer,
    /// Texture unit `glTexCoordPointer` / `glEnableClientState` affect.
    pub client_active_texture: u32,
    /// Texture unit `glTexEnv` / `glBindTexture` affect.
    pub active_texture: u32,

    // ---- textures ----
    pub textures: HashMap<u32, Texture>,
    pub bound_texture: u32,
    next_texture_name: u32,

    // ---- fragment pipeline ----
    pub state: PipelineState,
    pub clear_color: [f32; 4],
    pub clear_depth: f32,
    /// `GL_CULL_FACE` enable, tracked separately from the face mode so
    /// `glCullFace` before `glEnable` is not lost.
    cull_enabled: bool,
    /// The face `glCullFace` selected; GL's initial value is `GL_BACK`.
    cull_mode: CullMode,

    // ---- framebuffer ----
    pub target: RenderTarget,

    /// Sticky error code, returned and reset by `glGetError`.
    error: u32,

    /// Scratch buffer for guest array reads, reused across draws so a
    /// 60 fps render loop does not allocate per call.
    scratch: Vec<u8>,
    /// Scratch for index buffer reads.
    index_scratch: Vec<u8>,
}

impl Context {
    pub fn new(width: u32, height: u32) -> Self {
        let mut state = PipelineState {
            viewport: (0, 0, width as i32, height as i32),
            ..Default::default()
        };
        // GL's initial depth func is LESS, but the test is off until
        // the app enables it.
        state.depth_func = CompareFunc::Less;
        Self {
            matrix_mode: MatrixMode::Modelview,
            modelview: MatrixStack::new(16),
            projection: MatrixStack::new(16),
            texture_matrix: MatrixStack::new(16),
            current_color: [1.0, 1.0, 1.0, 1.0],
            current_texcoord: [0.0, 0.0],
            current_normal: [0.0, 0.0, 1.0],
            vertex_array: ArrayPointer::default(),
            color_array: ArrayPointer::default(),
            texcoord_array: ArrayPointer::default(),
            normal_array: ArrayPointer::default(),
            client_active_texture: 0,
            active_texture: 0,
            textures: HashMap::new(),
            bound_texture: 0,
            next_texture_name: 1,
            state,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear_depth: 1.0,
            cull_enabled: false,
            cull_mode: CullMode::Back,
            target: RenderTarget::new(width, height),
            error: GL_NO_ERROR,
            scratch: Vec::new(),
            index_scratch: Vec::new(),
        }
    }

    // ---- error handling -------------------------------------------------

    /// Record an error. GL keeps the *first* error until it is read, so
    /// a later failure does not mask an earlier one.
    pub fn set_error(&mut self, code: u32) {
        if self.error == GL_NO_ERROR {
            self.error = code;
        }
    }

    /// `glGetError`: return and clear the sticky error code.
    pub fn take_error(&mut self) -> u32 {
        std::mem::replace(&mut self.error, GL_NO_ERROR)
    }

    // ---- matrix stacks --------------------------------------------------

    fn stack_mut(&mut self) -> &mut MatrixStack {
        match self.matrix_mode {
            MatrixMode::Modelview => &mut self.modelview,
            MatrixMode::Projection => &mut self.projection,
            MatrixMode::Texture => &mut self.texture_matrix,
        }
    }

    pub fn set_matrix_mode(&mut self, mode: u32) {
        match MatrixMode::from_enum(mode) {
            Some(m) => self.matrix_mode = m,
            None => self.set_error(GL_INVALID_ENUM),
        }
    }

    pub fn load_identity(&mut self) {
        self.stack_mut().load_identity();
    }

    pub fn load_matrix(&mut self, m: Matrix4) {
        self.stack_mut().load(m);
    }

    pub fn mult_matrix(&mut self, m: Matrix4) {
        self.stack_mut().multiply_by(&m);
    }

    pub fn push_matrix(&mut self) {
        if !self.stack_mut().push() {
            self.set_error(GL_STACK_OVERFLOW);
        }
    }

    pub fn pop_matrix(&mut self) {
        if !self.stack_mut().pop() {
            self.set_error(GL_STACK_UNDERFLOW);
        }
    }

    pub fn translate(&mut self, x: f32, y: f32, z: f32) {
        let m = matrix::translate(x, y, z);
        self.stack_mut().multiply_by(&m);
    }

    pub fn scale(&mut self, x: f32, y: f32, z: f32) {
        let m = matrix::scale(x, y, z);
        self.stack_mut().multiply_by(&m);
    }

    pub fn rotate(&mut self, angle: f32, x: f32, y: f32, z: f32) {
        let m = matrix::rotate(angle, x, y, z);
        self.stack_mut().multiply_by(&m);
    }

    pub fn frustum(&mut self, l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) {
        // GL rejects a degenerate frustum rather than producing NaNs.
        if n <= 0.0 || f <= 0.0 || l == r || b == t || n == f {
            self.set_error(GL_INVALID_VALUE);
            return;
        }
        let m = matrix::frustum(l, r, b, t, n, f);
        self.stack_mut().multiply_by(&m);
    }

    pub fn ortho(&mut self, l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) {
        if l == r || b == t || n == f {
            self.set_error(GL_INVALID_VALUE);
            return;
        }
        let m = matrix::ortho(l, r, b, t, n, f);
        self.stack_mut().multiply_by(&m);
    }

    // ---- enables --------------------------------------------------------

    pub fn set_capability(&mut self, cap: u32, on: bool) {
        match cap {
            GL_DEPTH_TEST => self.state.depth_test = on,
            GL_ALPHA_TEST => self.state.alpha_test = on,
            GL_BLEND => self.state.blend = on,
            GL_CULL_FACE => {
                self.cull_enabled = on;
                self.sync_cull();
            }
            GL_SCISSOR_TEST => {
                if !on {
                    self.state.scissor = None;
                }
                // Enabling without a box set leaves the full-window
                // default, which matches GL's initial scissor state.
            }
            GL_TEXTURE_2D => self.state.texture_enabled = on,
            GL_FOG => self.state.fog = on,
            // Lighting, dither, normalize, stencil and the rest are
            // accepted and ignored: the fixed-function lighting model
            // is not implemented, and silently ignoring is far better
            // than raising GL_INVALID_ENUM, which some engines treat as
            // fatal.
            GL_LIGHTING
            | GL_DITHER
            | GL_NORMALIZE
            | GL_STENCIL_TEST
            | GL_COLOR_MATERIAL
            | GL_POLYGON_OFFSET_FILL => {}
            _ => {
                if !(0x4000..0x4010).contains(&cap) {
                    // GL_LIGHT0..7 live at 0x4000; anything else
                    // unknown is a genuine enum error.
                    self.set_error(GL_INVALID_ENUM);
                }
            }
        }
    }

    pub fn set_client_state(&mut self, array: u32, on: bool) {
        match array {
            GL_VERTEX_ARRAY => self.vertex_array.enabled = on,
            GL_COLOR_ARRAY => self.color_array.enabled = on,
            GL_TEXTURE_COORD_ARRAY => self.texcoord_array.enabled = on,
            GL_NORMAL_ARRAY => self.normal_array.enabled = on,
            _ => self.set_error(GL_INVALID_ENUM),
        }
    }

    // ---- textures -------------------------------------------------------

    /// `glGenTextures`: allocate `n` unused names.
    pub fn gen_textures(&mut self, n: usize) -> Vec<u32> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let name = self.next_texture_name;
            self.next_texture_name += 1;
            self.textures.insert(name, Texture::default());
            out.push(name);
        }
        out
    }

    pub fn bind_texture(&mut self, target: u32, name: u32) {
        if target != GL_TEXTURE_2D {
            self.set_error(GL_INVALID_ENUM);
            return;
        }
        // Binding an unseen name creates the object, as GL requires.
        if name != 0 {
            self.textures.entry(name).or_default();
            self.next_texture_name = self.next_texture_name.max(name + 1);
        }
        self.bound_texture = name;
    }

    pub fn delete_textures(&mut self, names: &[u32]) {
        for &name in names {
            if name == 0 {
                continue;
            }
            self.textures.remove(&name);
            if self.bound_texture == name {
                self.bound_texture = 0;
            }
        }
    }

    /// `glTexImage2D`. `data` is the already-copied guest buffer; an
    /// empty slice means a null pointer, which allocates storage
    /// without initialising it.
    // The parameter list mirrors the GL entry point one-for-one.
    // Bundling them into a struct would only move the unpacking to
    // every call site in the dispatch layer.
    #[allow(clippy::too_many_arguments)]
    pub fn tex_image_2d(
        &mut self,
        target: u32,
        level: i32,
        width: u32,
        height: u32,
        format: u32,
        ty: u32,
        data: &[u8],
    ) {
        if target != GL_TEXTURE_2D {
            self.set_error(GL_INVALID_ENUM);
            return;
        }
        // Only the base level is stored: we do not mipmap, and the
        // min-filters collapse to their base filter, so uploading level
        // 1+ would just overwrite level 0 with a half-size image.
        if level != 0 {
            return;
        }
        if self.bound_texture == 0 {
            self.set_error(GL_INVALID_OPERATION);
            return;
        }
        let rgba = if data.is_empty() {
            vec![0u8; (width as usize) * (height as usize) * 4]
        } else {
            match texture::decode_to_rgba(data, width, height, format, ty) {
                Some(v) => v,
                None => {
                    self.set_error(GL_INVALID_ENUM);
                    return;
                }
            }
        };
        let name = self.bound_texture;
        let tex = self.textures.entry(name).or_default();
        tex.width = width;
        tex.height = height;
        tex.rgba = rgba;
    }

    /// `glTexSubImage2D`: patch a rectangle of the bound texture.
    // The parameter list mirrors the GL entry point one-for-one.
    #[allow(clippy::too_many_arguments)]
    pub fn tex_sub_image_2d(
        &mut self,
        target: u32,
        level: i32,
        xoffset: u32,
        yoffset: u32,
        width: u32,
        height: u32,
        format: u32,
        ty: u32,
        data: &[u8],
    ) {
        if target != GL_TEXTURE_2D {
            self.set_error(GL_INVALID_ENUM);
            return;
        }
        if level != 0 {
            return;
        }
        let Some(patch) = texture::decode_to_rgba(data, width, height, format, ty) else {
            self.set_error(GL_INVALID_ENUM);
            return;
        };
        let name = self.bound_texture;
        let Some(tex) = self.textures.get_mut(&name) else {
            self.set_error(GL_INVALID_OPERATION);
            return;
        };
        if xoffset + width > tex.width || yoffset + height > tex.height {
            self.set_error(GL_INVALID_VALUE);
            return;
        }
        for row in 0..height {
            let src = (row * width * 4) as usize;
            let dst = (((yoffset + row) * tex.width + xoffset) * 4) as usize;
            let len = (width * 4) as usize;
            tex.rgba[dst..dst + len].copy_from_slice(&patch[src..src + len]);
        }
    }

    pub fn tex_parameter(&mut self, target: u32, pname: u32, value: u32) {
        if target != GL_TEXTURE_2D {
            self.set_error(GL_INVALID_ENUM);
            return;
        }
        let name = self.bound_texture;
        let Some(tex) = self.textures.get_mut(&name) else {
            return;
        };
        if !tex.set_parameter(pname, value) {
            self.set_error(GL_INVALID_ENUM);
        }
    }

    pub fn tex_env(&mut self, pname: u32, value: u32) {
        if pname != GL_TEXTURE_ENV_MODE {
            // TEXTURE_ENV_COLOR and the COMBINE parameters are accepted
            // and ignored rather than erroring.
            return;
        }
        self.state.tex_env = match value {
            GL_MODULATE => TexEnvMode::Modulate,
            GL_REPLACE => TexEnvMode::Replace,
            GL_DECAL => TexEnvMode::Decal,
            GL_ADD => TexEnvMode::Add,
            GL_COMBINE => TexEnvMode::Modulate,
            // GL_BLEND shares its value with the blend enable enum.
            0x0BE2 => TexEnvMode::Blend,
            _ => {
                self.set_error(GL_INVALID_ENUM);
                return;
            }
        };
    }

    // ---- fragment state -------------------------------------------------

    pub fn set_depth_func(&mut self, func: u32) {
        match CompareFunc::from_enum(func) {
            Some(f) => self.state.depth_func = f,
            None => self.set_error(GL_INVALID_ENUM),
        }
    }

    pub fn set_alpha_func(&mut self, func: u32, reference: f32) {
        match CompareFunc::from_enum(func) {
            Some(f) => {
                self.state.alpha_func = f;
                self.state.alpha_ref = reference.clamp(0.0, 1.0);
            }
            None => self.set_error(GL_INVALID_ENUM),
        }
    }

    pub fn set_blend_func(&mut self, src: u32, dst: u32) {
        match (BlendFactor::from_enum(src), BlendFactor::from_enum(dst)) {
            (Some(s), Some(d)) => {
                self.state.blend_src = s;
                self.state.blend_dst = d;
            }
            _ => self.set_error(GL_INVALID_ENUM),
        }
    }

    pub fn set_cull_face(&mut self, mode: u32) {
        let cull = match mode {
            GL_FRONT => CullMode::Front,
            GL_BACK => CullMode::Back,
            GL_FRONT_AND_BACK => CullMode::FrontAndBack,
            _ => {
                self.set_error(GL_INVALID_ENUM);
                return;
            }
        };
        self.cull_mode = cull;
        self.sync_cull();
    }

    /// `GL_CULL_FACE` and `glCullFace` are independent pieces of state;
    /// the rasterizer only sees the combination, so recompute it
    /// whenever either changes.
    fn sync_cull(&mut self) {
        self.state.cull = if self.cull_enabled {
            Some(self.cull_mode)
        } else {
            None
        };
    }

    pub fn set_front_face(&mut self, mode: u32) {
        match mode {
            GL_CW => self.state.front_face = FrontFace::Cw,
            GL_CCW => self.state.front_face = FrontFace::Ccw,
            _ => self.set_error(GL_INVALID_ENUM),
        }
    }

    pub fn set_shade_model(&mut self, mode: u32) {
        match mode {
            GL_FLAT => self.state.smooth_shading = false,
            GL_SMOOTH => self.state.smooth_shading = true,
            _ => self.set_error(GL_INVALID_ENUM),
        }
    }

    pub fn set_fog(&mut self, pname: u32, value: f32) {
        match pname {
            GL_FOG_MODE => {
                // GL_LINEAR and GL_EXP share numeric space with other
                // enums, so compare against the fog values directly.
                self.state.fog_mode = match value as u32 {
                    GL_EXP => FogMode::Exp,
                    GL_EXP2 => FogMode::Exp2,
                    GL_LINEAR => FogMode::Linear,
                    _ => {
                        self.set_error(GL_INVALID_ENUM);
                        return;
                    }
                };
            }
            GL_FOG_DENSITY => self.state.fog_density = value,
            GL_FOG_START => self.state.fog_start = value,
            GL_FOG_END => self.state.fog_end = value,
            _ => self.set_error(GL_INVALID_ENUM),
        }
    }

    pub fn set_viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if w < 0 || h < 0 {
            self.set_error(GL_INVALID_VALUE);
            return;
        }
        self.state.viewport = (x, y, w, h);
    }

    pub fn set_scissor(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if w < 0 || h < 0 {
            self.set_error(GL_INVALID_VALUE);
            return;
        }
        self.state.scissor = Some((x, y, w, h));
    }

    pub fn set_depth_range(&mut self, near: f32, far: f32) {
        self.state.depth_range = (near.clamp(0.0, 1.0), far.clamp(0.0, 1.0));
    }

    // ---- clearing -------------------------------------------------------

    pub fn clear(&mut self, mask: u32) {
        if mask & GL_COLOR_BUFFER_BIT != 0 {
            let c = [
                (self.clear_color[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                (self.clear_color[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                (self.clear_color[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                (self.clear_color[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            ];
            self.target.clear_color(c, self.state.scissor);
        }
        if mask & GL_DEPTH_BUFFER_BIT != 0 {
            let d = self.clear_depth;
            self.target.clear_depth(d, self.state.scissor);
        }
    }

    // ---- drawing --------------------------------------------------------

    /// The combined projection × modelview transform applied to every
    /// incoming position.
    fn mvp(&self) -> Matrix4 {
        matrix::multiply(self.projection.current(), self.modelview.current())
    }

    /// Fetch element `index` of `array`, writing up to 4 components.
    /// Missing components take their GL defaults: `(0, 0, 0, 1)`.
    fn fetch(
        &mut self,
        mem: &mut dyn GuestMemory,
        array: ArrayPointer,
        index: u32,
        normalize: bool,
    ) -> Option<[f32; 4]> {
        let comp = array.component_bytes()?;
        let stride = array.element_stride()?;
        let addr = array.pointer.checked_add(stride.checked_mul(index)?)?;
        let len = (comp * array.size) as usize;
        let mut buf = std::mem::take(&mut self.scratch);
        let ok = mem.read(addr, len, &mut buf);
        let mut out = [0.0, 0.0, 0.0, 1.0];
        if ok {
            for (c, slot) in out.iter_mut().enumerate().take(array.size as usize) {
                let off = c * comp as usize;
                *slot = decode_component(&buf[off..], array.ty, normalize);
            }
        }
        self.scratch = buf;
        if ok {
            Some(out)
        } else {
            None
        }
    }

    /// Assemble one vertex from the enabled client arrays.
    fn assemble(&mut self, mem: &mut dyn GuestMemory, index: u32, mvp: &Matrix4) -> Option<Vertex> {
        let obj = self.fetch(mem, self.vertex_array, index, false)?;
        let clip = matrix::transform(mvp, obj);

        let color = if self.color_array.enabled {
            // Colours from integer arrays are normalized to [0, 1];
            // this is the one place GL treats a UNSIGNED_BYTE array as
            // a fraction rather than a raw value.
            self.fetch(mem, self.color_array, index, true)
                .unwrap_or(self.current_color)
        } else {
            self.current_color
        };

        let texcoord = if self.texcoord_array.enabled {
            let t = self
                .fetch(mem, self.texcoord_array, index, false)
                .unwrap_or([0.0, 0.0, 0.0, 1.0]);
            // The texture matrix applies to incoming coordinates.
            let m = matrix::transform(self.texture_matrix.current(), [t[0], t[1], 0.0, 1.0]);
            [m[0], m[1]]
        } else {
            self.current_texcoord
        };

        // Eye-space Z drives fog. The modelview alone takes us to eye
        // space; GL fogs on |z_eye|.
        let eye = matrix::transform(self.modelview.current(), obj);
        Some(Vertex {
            pos: clip,
            color,
            texcoord,
            fog_depth: eye[2].abs(),
        })
    }

    /// `glDrawArrays`.
    pub fn draw_arrays(&mut self, mem: &mut dyn GuestMemory, mode: u32, first: u32, count: u32) {
        if !self.vertex_array.enabled || self.vertex_array.pointer == 0 {
            return;
        }
        let indices: Vec<u32> = (first..first.saturating_add(count)).collect();
        self.draw_indexed(mem, mode, &indices);
    }

    /// `glDrawElements`. `indices` has already been read out of guest
    /// memory and widened to `u32`.
    pub fn draw_elements(&mut self, mem: &mut dyn GuestMemory, mode: u32, indices: &[u32]) {
        if !self.vertex_array.enabled || self.vertex_array.pointer == 0 {
            return;
        }
        self.draw_indexed(mem, mode, indices);
    }

    /// Read an index buffer out of guest memory and draw with it.
    pub fn draw_elements_from_guest(
        &mut self,
        mem: &mut dyn GuestMemory,
        mode: u32,
        count: u32,
        ty: u32,
        pointer: u32,
    ) {
        let width = match ty {
            GL_UNSIGNED_BYTE => 1usize,
            GL_UNSIGNED_SHORT => 2usize,
            _ => {
                self.set_error(GL_INVALID_ENUM);
                return;
            }
        };
        let Some(len) = (count as usize).checked_mul(width) else {
            self.set_error(GL_INVALID_VALUE);
            return;
        };
        let mut buf = std::mem::take(&mut self.index_scratch);
        let ok = mem.read(pointer, len, &mut buf);
        let indices: Vec<u32> = if !ok {
            Vec::new()
        } else if width == 1 {
            buf[..len].iter().map(|&b| b as u32).collect()
        } else {
            buf[..len]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
                .collect()
        };
        self.index_scratch = buf;
        if indices.is_empty() {
            return;
        }
        self.draw_elements(mem, mode, &indices);
    }

    fn draw_indexed(&mut self, mem: &mut dyn GuestMemory, mode: u32, indices: &[u32]) {
        let mvp = self.mvp();
        // Assemble every referenced vertex once. A cache keyed on the
        // index means an indexed mesh transforms each shared vertex a
        // single time instead of once per triangle that uses it.
        let mut cache: HashMap<u32, Vertex> = HashMap::new();
        let mut verts: Vec<Vertex> = Vec::with_capacity(indices.len());
        for &i in indices {
            let v = match cache.get(&i) {
                Some(v) => *v,
                None => {
                    let Some(v) = self.assemble(mem, i, &mvp) else {
                        // An unmapped array pointer aborts the draw
                        // rather than rendering garbage geometry.
                        return;
                    };
                    cache.insert(i, v);
                    v
                }
            };
            verts.push(v);
        }

        let tris: Vec<[Vertex; 3]> = match mode {
            GL_TRIANGLES => verts.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
            GL_TRIANGLE_STRIP => (2..verts.len())
                .map(|i| {
                    // Every other triangle in a strip has reversed
                    // winding; swap two vertices so face culling sees a
                    // consistent orientation.
                    if i % 2 == 0 {
                        [verts[i - 2], verts[i - 1], verts[i]]
                    } else {
                        [verts[i - 1], verts[i - 2], verts[i]]
                    }
                })
                .collect(),
            GL_TRIANGLE_FAN => (2..verts.len())
                .map(|i| [verts[0], verts[i - 1], verts[i]])
                .collect(),
            // Points and lines are not rasterized; accepting them
            // silently keeps a game's HUD code from erroring out.
            GL_POINTS | GL_LINES | GL_LINE_STRIP | GL_LINE_LOOP => return,
            _ => {
                self.set_error(GL_INVALID_ENUM);
                return;
            }
        };

        // Resolve the bound texture once per draw, not per fragment.
        let tex = if self.state.texture_enabled {
            self.textures.get(&self.bound_texture)
        } else {
            None
        };
        let sample = |s: f32, t: f32| tex.map(|tx| tx.sample_nearest(s, t));
        for tri in tris {
            raster::draw_triangle(&mut self.target, &self.state, &sample, tri);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guest memory backed by a flat buffer based at `base`.
    struct FakeMem {
        base: u32,
        bytes: Vec<u8>,
    }

    impl GuestMemory for FakeMem {
        fn read(&mut self, addr: u32, len: usize, out: &mut Vec<u8>) -> bool {
            let Some(off) = addr.checked_sub(self.base) else {
                return false;
            };
            let off = off as usize;
            if off + len > self.bytes.len() {
                return false;
            }
            out.clear();
            out.extend_from_slice(&self.bytes[off..off + len]);
            true
        }
    }

    const BASE: u32 = 0x1000;

    fn floats(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    fn fixeds(v: &[f32]) -> Vec<u8> {
        v.iter()
            .flat_map(|f| fixed::from_f32(*f).to_le_bytes())
            .collect()
    }

    fn ctx() -> Context {
        let mut c = Context::new(64, 64);
        c.state.viewport = (0, 0, 64, 64);
        c
    }

    fn pixel(c: &Context, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * c.target.width + x) * 4) as usize;
        [
            c.target.color[i],
            c.target.color[i + 1],
            c.target.color[i + 2],
            c.target.color[i + 3],
        ]
    }

    fn covered(c: &Context) -> usize {
        c.target
            .color
            .chunks_exact(4)
            .filter(|p| p[..3] != [0, 0, 0])
            .count()
    }

    /// A full-viewport quad in NDC, as two triangles worth of vertices.
    fn quad_floats() -> Vec<u8> {
        floats(&[
            -1.0, -1.0, 0.0, //
            1.0, -1.0, 0.0, //
            1.0, 1.0, 0.0, //
            -1.0, 1.0, 0.0,
        ])
    }

    #[test]
    fn draw_arrays_renders_float_positions() {
        let mut c = ctx();
        let mut m = FakeMem {
            base: BASE,
            bytes: quad_floats(),
        };
        c.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };
        c.current_color = [1.0, 0.0, 0.0, 1.0];
        c.draw_arrays(&mut m, GL_TRIANGLE_FAN, 0, 4);
        assert!(covered(&c) > 3000, "covered only {}", covered(&c));
        assert_eq!(pixel(&c, 32, 32), [255, 0, 0, 255]);
    }

    #[test]
    fn fixed_point_positions_render_identically_to_float() {
        // This is the whole reason the crate carries a fixed-point
        // module: Call of Duty 2 feeds GL_FIXED vertex arrays, which
        // desktop GL 2.1 cannot consume at all.
        let mut a = ctx();
        let mut ma = FakeMem {
            base: BASE,
            bytes: quad_floats(),
        };
        a.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };
        a.draw_arrays(&mut ma, GL_TRIANGLE_FAN, 0, 4);

        let mut b = ctx();
        let mut mb = FakeMem {
            base: BASE,
            bytes: fixeds(&[
                -1.0, -1.0, 0.0, //
                1.0, -1.0, 0.0, //
                1.0, 1.0, 0.0, //
                -1.0, 1.0, 0.0,
            ]),
        };
        b.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FIXED,
            stride: 0,
            pointer: BASE,
        };
        b.draw_arrays(&mut mb, GL_TRIANGLE_FAN, 0, 4);

        assert_eq!(a.target.color, b.target.color);
        assert!(covered(&a) > 3000);
    }

    #[test]
    fn interleaved_stride_skips_padding() {
        // Position at offset 0 of a 20-byte struct, the classic
        // interleaved layout. A stride bug renders scrambled geometry.
        let mut bytes = Vec::new();
        for pos in [
            [-1.0f32, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ] {
            bytes.extend(floats(&pos));
            bytes.extend([0u8; 8]); // padding: uv, colour, whatever
        }
        let mut c = ctx();
        let mut m = FakeMem { base: BASE, bytes };
        c.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 20,
            pointer: BASE,
        };
        c.draw_arrays(&mut m, GL_TRIANGLE_FAN, 0, 4);
        assert!(covered(&c) > 3000, "stride handling dropped geometry");
    }

    #[test]
    fn unmapped_array_pointer_aborts_the_draw() {
        let mut c = ctx();
        let mut m = FakeMem {
            base: BASE,
            bytes: vec![0u8; 8],
        };
        c.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            // Far outside the fake mapping.
            pointer: 0xDEAD_0000,
        };
        c.draw_arrays(&mut m, GL_TRIANGLE_FAN, 0, 4);
        assert_eq!(covered(&c), 0);
    }

    #[test]
    fn disabled_vertex_array_draws_nothing() {
        let mut c = ctx();
        let mut m = FakeMem {
            base: BASE,
            bytes: quad_floats(),
        };
        c.vertex_array = ArrayPointer {
            enabled: false,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };
        c.draw_arrays(&mut m, GL_TRIANGLE_FAN, 0, 4);
        assert_eq!(covered(&c), 0);
    }

    #[test]
    fn draw_elements_reads_ushort_indices_from_guest_memory() {
        let mut bytes = quad_floats();
        let index_off = bytes.len() as u32;
        for i in [0u16, 1, 2, 0, 2, 3] {
            bytes.extend(i.to_le_bytes());
        }
        let mut c = ctx();
        let mut m = FakeMem { base: BASE, bytes };
        c.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };
        c.draw_elements_from_guest(&mut m, GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, BASE + index_off);
        assert!(covered(&c) > 3000, "indexed quad covered {}", covered(&c));
    }

    #[test]
    fn draw_elements_reads_ubyte_indices() {
        let mut bytes = quad_floats();
        let index_off = bytes.len() as u32;
        bytes.extend([0u8, 1, 2, 0, 2, 3]);
        let mut c = ctx();
        let mut m = FakeMem { base: BASE, bytes };
        c.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };
        c.draw_elements_from_guest(&mut m, GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, BASE + index_off);
        assert!(covered(&c) > 3000);
    }

    #[test]
    fn color_array_bytes_are_normalized() {
        // A UNSIGNED_BYTE colour array holds 0..255 meaning 0.0..1.0.
        // Treating it as a raw value would saturate every vertex white.
        let mut bytes = quad_floats();
        let color_off = bytes.len() as u32;
        for _ in 0..4 {
            bytes.extend([0u8, 0, 255, 255]); // pure blue
        }
        let mut c = ctx();
        let mut m = FakeMem { base: BASE, bytes };
        c.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };
        c.color_array = ArrayPointer {
            enabled: true,
            size: 4,
            ty: GL_UNSIGNED_BYTE,
            stride: 0,
            pointer: BASE + color_off,
        };
        c.draw_arrays(&mut m, GL_TRIANGLE_FAN, 0, 4);
        assert_eq!(pixel(&c, 32, 32), [0, 0, 255, 255]);
    }

    #[test]
    fn texture_coords_sample_the_bound_texture() {
        let mut bytes = quad_floats();
        let uv_off = bytes.len() as u32;
        bytes.extend(floats(&[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]));

        let mut c = ctx();
        let mut m = FakeMem { base: BASE, bytes };
        let names = c.gen_textures(1);
        c.bind_texture(GL_TEXTURE_2D, names[0]);
        // 1x1 green texture.
        c.tex_image_2d(
            GL_TEXTURE_2D,
            0,
            1,
            1,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            &[0, 255, 0, 255],
        );
        c.set_capability(GL_TEXTURE_2D, true);
        c.state.tex_env = TexEnvMode::Replace;
        c.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };
        c.texcoord_array = ArrayPointer {
            enabled: true,
            size: 2,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE + uv_off,
        };
        c.draw_arrays(&mut m, GL_TRIANGLE_FAN, 0, 4);
        assert_eq!(pixel(&c, 32, 32), [0, 255, 0, 255]);
    }

    #[test]
    fn projection_and_modelview_both_apply() {
        // Halving via the modelview and halving again via the
        // projection must compound: coverage should drop to ~1/16.
        let mut full = ctx();
        let mut m1 = FakeMem {
            base: BASE,
            bytes: quad_floats(),
        };
        let ptr = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };
        full.vertex_array = ptr;
        full.draw_arrays(&mut m1, GL_TRIANGLE_FAN, 0, 4);
        let full_area = covered(&full);

        let mut half = ctx();
        let mut m2 = FakeMem {
            base: BASE,
            bytes: quad_floats(),
        };
        half.vertex_array = ptr;
        half.set_matrix_mode(GL_MODELVIEW);
        half.scale(0.5, 0.5, 1.0);
        half.set_matrix_mode(GL_PROJECTION);
        half.scale(0.5, 0.5, 1.0);
        half.draw_arrays(&mut m2, GL_TRIANGLE_FAN, 0, 4);
        let quarter_area = covered(&half);

        let ratio = full_area as f32 / quarter_area.max(1) as f32;
        assert!(
            (12.0..20.0).contains(&ratio),
            "expected ~16x area ratio, got {ratio}"
        );
    }

    #[test]
    fn matrix_stack_push_pop_isolates_transforms() {
        let mut c = ctx();
        c.set_matrix_mode(GL_MODELVIEW);
        c.load_identity();
        c.push_matrix();
        c.scale(0.25, 0.25, 1.0);
        c.pop_matrix();
        assert_eq!(*c.modelview.current(), matrix::IDENTITY);
        assert_eq!(c.take_error(), GL_NO_ERROR);
    }

    #[test]
    fn stack_underflow_is_reported() {
        let mut c = ctx();
        c.set_matrix_mode(GL_MODELVIEW);
        c.pop_matrix();
        assert_eq!(c.take_error(), GL_STACK_UNDERFLOW);
    }

    #[test]
    fn first_error_survives_until_read() {
        let mut c = ctx();
        c.set_matrix_mode(0x9999); // GL_INVALID_ENUM
        c.set_viewport(0, 0, -1, -1); // GL_INVALID_VALUE
        assert_eq!(c.take_error(), GL_INVALID_ENUM);
        assert_eq!(c.take_error(), GL_NO_ERROR);
    }

    #[test]
    fn clear_uses_the_clear_color() {
        let mut c = ctx();
        c.clear_color = [0.0, 0.0, 1.0, 1.0];
        c.clear(GL_COLOR_BUFFER_BIT);
        assert_eq!(pixel(&c, 0, 0), [0, 0, 255, 255]);
    }

    #[test]
    fn gen_textures_returns_distinct_names() {
        let mut c = ctx();
        let a = c.gen_textures(3);
        let b = c.gen_textures(2);
        assert_eq!(a.len(), 3);
        assert!(a.iter().all(|n| *n != 0), "0 is the reserved default name");
        assert!(b.iter().all(|n| !a.contains(n)), "names were reused");
    }

    #[test]
    fn deleting_the_bound_texture_unbinds_it() {
        let mut c = ctx();
        let n = c.gen_textures(1);
        c.bind_texture(GL_TEXTURE_2D, n[0]);
        c.delete_textures(&n);
        assert_eq!(c.bound_texture, 0);
        assert!(!c.textures.contains_key(&n[0]));
    }

    #[test]
    fn tex_image_without_a_bound_texture_is_an_error() {
        let mut c = ctx();
        c.bind_texture(GL_TEXTURE_2D, 0);
        c.tex_image_2d(
            GL_TEXTURE_2D,
            0,
            1,
            1,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            &[1, 2, 3, 4],
        );
        assert_eq!(c.take_error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn tex_sub_image_patches_a_rectangle() {
        let mut c = ctx();
        let n = c.gen_textures(1);
        c.bind_texture(GL_TEXTURE_2D, n[0]);
        c.tex_image_2d(
            GL_TEXTURE_2D,
            0,
            2,
            2,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            &[0u8; 16],
        );
        c.tex_sub_image_2d(
            GL_TEXTURE_2D,
            0,
            1,
            1,
            1,
            1,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            &[9, 9, 9, 9],
        );
        let t = &c.textures[&n[0]];
        assert_eq!(&t.rgba[12..16], &[9, 9, 9, 9]);
        assert_eq!(&t.rgba[0..4], &[0, 0, 0, 0]);
        assert_eq!(c.take_error(), GL_NO_ERROR);
    }

    #[test]
    fn tex_sub_image_out_of_bounds_is_rejected() {
        let mut c = ctx();
        let n = c.gen_textures(1);
        c.bind_texture(GL_TEXTURE_2D, n[0]);
        c.tex_image_2d(
            GL_TEXTURE_2D,
            0,
            2,
            2,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            &[0u8; 16],
        );
        c.tex_sub_image_2d(
            GL_TEXTURE_2D,
            0,
            2,
            2,
            2,
            2,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            &[0u8; 16],
        );
        assert_eq!(c.take_error(), GL_INVALID_VALUE);
    }

    #[test]
    fn triangle_strip_winding_stays_consistent_under_culling() {
        // Every second triangle of a strip has reversed winding. If the
        // assembler doesn't compensate, back-face culling erases half
        // the mesh — the classic "striped" rendering bug.
        //
        // Vertices run top, bottom, top, bottom left-to-right, which
        // makes the first triangle counter-clockwise in GL's frame and
        // therefore front-facing.
        let strip = || {
            let mut b = Vec::new();
            for p in [
                [-1.0f32, 1.0, 0.0],
                [-1.0, -1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, -1.0, 0.0],
                [1.0, 1.0, 0.0],
                [1.0, -1.0, 0.0],
            ] {
                b.extend(floats(&p));
            }
            b
        };
        let ptr = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };

        let mut plain = ctx();
        let mut m1 = FakeMem {
            base: BASE,
            bytes: strip(),
        };
        plain.vertex_array = ptr;
        plain.draw_arrays(&mut m1, GL_TRIANGLE_STRIP, 0, 6);
        let no_cull = covered(&plain);

        let mut culled = ctx();
        let mut m2 = FakeMem {
            base: BASE,
            bytes: strip(),
        };
        culled.vertex_array = ptr;
        culled.set_cull_face(GL_BACK);
        culled.set_capability(GL_CULL_FACE, true);
        culled.draw_arrays(&mut m2, GL_TRIANGLE_STRIP, 0, 6);
        let with_cull = covered(&culled);

        assert!(no_cull > 3000, "unculled strip covered {no_cull}");
        assert_eq!(
            with_cull, no_cull,
            "culling removed part of a consistently-wound strip"
        );

        // And the converse: flipping the front face must erase the
        // whole strip, proving the strip really was uniformly wound
        // rather than the cull test being a no-op.
        let mut flipped = ctx();
        let mut m3 = FakeMem {
            base: BASE,
            bytes: strip(),
        };
        flipped.vertex_array = ptr;
        flipped.set_front_face(GL_CW);
        flipped.set_cull_face(GL_BACK);
        flipped.set_capability(GL_CULL_FACE, true);
        flipped.draw_arrays(&mut m3, GL_TRIANGLE_STRIP, 0, 6);
        assert_eq!(covered(&flipped), 0, "strip was not uniformly wound");
    }

    #[test]
    fn points_and_lines_are_accepted_without_error() {
        let mut c = ctx();
        let mut m = FakeMem {
            base: BASE,
            bytes: quad_floats(),
        };
        c.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };
        c.draw_arrays(&mut m, GL_LINES, 0, 4);
        c.draw_arrays(&mut m, GL_POINTS, 0, 4);
        assert_eq!(c.take_error(), GL_NO_ERROR);
    }

    #[test]
    fn unknown_primitive_mode_is_an_error() {
        let mut c = ctx();
        let mut m = FakeMem {
            base: BASE,
            bytes: quad_floats(),
        };
        c.vertex_array = ArrayPointer {
            enabled: true,
            size: 3,
            ty: GL_FLOAT,
            stride: 0,
            pointer: BASE,
        };
        c.draw_arrays(&mut m, 0x99, 0, 4);
        assert_eq!(c.take_error(), GL_INVALID_ENUM);
    }

    #[test]
    fn lighting_enable_is_ignored_not_rejected() {
        // COD2 enables lighting; erroring here would make the game
        // think the driver is broken.
        let mut c = ctx();
        c.set_capability(GL_LIGHTING, true);
        c.set_capability(0x4000, true); // GL_LIGHT0
        assert_eq!(c.take_error(), GL_NO_ERROR);
    }

    #[test]
    fn cull_face_before_enable_is_not_lost() {
        // Engines commonly call glCullFace once at init and toggle
        // GL_CULL_FACE per material. If the mode were only stored while
        // culling happened to be enabled, the toggle would silently
        // revert to GL_BACK.
        let mut c = ctx();
        c.set_cull_face(GL_FRONT);
        assert!(c.state.cull.is_none(), "mode set should not enable culling");
        c.set_capability(GL_CULL_FACE, true);
        assert_eq!(c.state.cull, Some(CullMode::Front));
        c.set_capability(GL_CULL_FACE, false);
        assert!(c.state.cull.is_none());
        c.set_capability(GL_CULL_FACE, true);
        assert_eq!(c.state.cull, Some(CullMode::Front), "mode was forgotten");
    }

    #[test]
    fn degenerate_frustum_is_rejected_without_nans() {
        let mut c = ctx();
        c.set_matrix_mode(GL_PROJECTION);
        c.frustum(-1.0, 1.0, -1.0, 1.0, 0.0, 100.0);
        assert_eq!(c.take_error(), GL_INVALID_VALUE);
        assert!(c.projection.current().iter().all(|v| v.is_finite()));
    }
}
