//! opengl32.dll: WGL and OpenGL 3.3 (plus compatibility profile), backed by `gpu_api::gl::Gl` on the machine's GPU.
//!
//! Each process that creates an OpenGL context gets a [`GlState`] holding one `gpu_api::gl::Gl` instance, per-context
//! client state (pixel-store, active client texture unit, mapped buffer allocations, and immediate-mode batching),
//! per-thread current `(hdc, hglrc)` bindings, and a guest string pool for `glGetString`/`wglGetExtensionsStringARB`.
//! `wglGetProcAddress` resolves OpenGL 1.2–3.3, compatibility, and WGL extension entry points to their exports in this
//! DLL image.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use exec::{FpRegs, Isa};
use gpu_api::gl::Gl;
use gpu_api::vulkan::Guest;
use gpu_wire::gl as wire;
use gpu_wire::records::{self, Reader};
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "OPENGL32.dll";
    // Core GL 1.1 exports (statically imported by Windows programs).
    host glAlphaFunc(2) => gl_alpha_func;
    host glBegin(1) => gl_begin;
    host glBindTexture(2) => gl_bind_texture;
    host glBlendFunc(2) => gl_blend_func;
    host glClear(1) => gl_clear;
    host glClearColor(4) => gl_clear_color;
    host glClearDepth(1) => gl_clear_depth;
    host glClearStencil(1) => gl_clear_stencil;
    host glColor3f(3) => gl_color3f;
    host glColor3d(3) => gl_color3d;
    host glColor3ub(3) => gl_color3ub;
    host glColor3fv(1) => gl_color3fv;
    host glColor4f(4) => gl_color4f;
    host glColor4d(4) => gl_color4d;
    host glColor4fv(1) => gl_color4fv;
    host glColor4ub(4) => gl_color4ub;
    host glColor4ubv(1) => gl_color4ubv;
    host glColorMask(4) => gl_color_mask;
    host glColorPointer(4) => gl_color_pointer;
    host glCopyTexSubImage2D(8) => gl_noop;
    host glCullFace(1) => gl_cull_face;
    host glDeleteTextures(2) => gl_delete_textures;
    host glDepthFunc(1) => gl_depth_func;
    host glDepthMask(1) => gl_depth_mask;
    host glDepthRange(2) => gl_depth_range;
    host glDisable(1) => gl_disable;
    host glDisableClientState(1) => gl_disable_client_state;
    host glDrawArrays(3) => gl_draw_arrays;
    host glDrawBuffer(1) => gl_draw_buffer;
    host glDrawElements(4) => gl_draw_elements;
    host glEnable(1) => gl_enable;
    host glEnableClientState(1) => gl_enable_client_state;
    host glEnd(0) => gl_end;
    host glFinish(0) => gl_finish;
    host glFlush(0) => gl_flush;
    host glFogf(2) => gl_fogf;
    host glFogfv(2) => gl_fogfv;
    host glFogi(2) => gl_fogi;
    host glFogiv(2) => gl_fogiv;
    host glFrontFace(1) => gl_front_face;
    host glFrustum(6) => gl_frustum;
    host glGenTextures(2) => gl_gen_textures;
    host glGetBooleanv(2) => gl_get_booleanv;
    host glGetDoublev(2) => gl_get_doublev;
    host glGetError(0) => gl_get_error;
    host glGetFloatv(2) => gl_get_floatv;
    host glGetIntegerv(2) => gl_get_integerv;
    host glGetString(1) => gl_get_string;
    host glHint(2) => gl_noop;
    host glIsEnabled(1) => gl_is_enabled;
    host glIsTexture(1) => gl_is_name;
    host glLightf(3) => gl_noop;
    host glLightfv(3) => gl_noop;
    host glLightModeli(2) => gl_noop;
    host glLightModelfv(2) => gl_noop;
    host glLineWidth(1) => gl_noop;
    host glLoadIdentity(0) => gl_load_identity;
    host glLoadMatrixd(1) => gl_load_matrixd;
    host glLoadMatrixf(1) => gl_load_matrixf;
    host glMaterialf(3) => gl_noop;
    host glMaterialfv(3) => gl_noop;
    host glColorMaterial(2) => gl_noop;
    host glMatrixMode(1) => gl_matrix_mode;
    host glMultMatrixd(1) => gl_mult_matrixd;
    host glMultMatrixf(1) => gl_mult_matrixf;
    host glNormal3f(3) => gl_normal3f;
    host glNormal3fv(1) => gl_normal3fv;
    host glNormalPointer(3) => gl_normal_pointer;
    host glOrtho(6) => gl_ortho;
    host glPixelStoref(2) => gl_pixel_storef;
    host glPixelStorei(2) => gl_pixel_storei;
    host glPixelZoom(2) => gl_noop;
    host glPointSize(1) => gl_noop;
    host glPolygonMode(2) => gl_noop;
    host glPolygonOffset(2) => gl_noop;
    host glPopAttrib(0) => gl_noop;
    host glPopClientAttrib(0) => gl_noop;
    host glPopMatrix(0) => gl_pop_matrix;
    host glPushAttrib(1) => gl_noop;
    host glPushClientAttrib(1) => gl_noop;
    host glPushMatrix(0) => gl_push_matrix;
    host glReadBuffer(1) => gl_read_buffer;
    host glReadPixels(7) => gl_read_pixels;
    host glRectf(4) => gl_rectf;
    host glRenderMode(1) => gl_zero;
    host glRotated(4) => gl_rotated;
    host glRotatef(4) => gl_rotatef;
    host glScaled(3) => gl_scaled;
    host glScalef(3) => gl_scalef;
    host glScissor(4) => gl_scissor;
    host glShadeModel(1) => gl_shade_model;
    host glStencilFunc(3) => gl_noop;
    host glStencilMask(1) => gl_noop;
    host glStencilOp(3) => gl_noop;
    host glTexCoord2d(2) => gl_tex_coord2d;
    host glTexCoord2f(2) => gl_tex_coord2f;
    host glTexCoord2fv(1) => gl_tex_coord2fv;
    host glTexCoord2i(2) => gl_tex_coord2i;
    host glTexCoordPointer(4) => gl_tex_coord_pointer;
    host glTexEnvf(3) => gl_tex_envf;
    host glTexEnvfv(3) => gl_tex_envfv;
    host glTexEnvi(3) => gl_tex_envi;
    host glTexEnviv(3) => gl_tex_enviv;
    host glTexGeni(3) => gl_noop;
    host glTexGenfv(3) => gl_noop;
    host glTexImage2D(9) => gl_tex_image_2d;
    host glTexParameterf(3) => gl_tex_parameterf;
    host glTexParameterfv(3) => gl_tex_parameterfv;
    host glTexParameteri(3) => gl_tex_parameteri;
    host glTexParameteriv(3) => gl_tex_parameteriv;
    host glTexSubImage2D(9) => gl_tex_sub_image_2d;
    host glTranslated(3) => gl_translated;
    host glTranslatef(3) => gl_translatef;
    host glVertex2f(2) => gl_vertex2f;
    host glVertex2i(2) => gl_vertex2i;
    host glVertex3f(3) => gl_vertex3f;
    host glVertex3d(3) => gl_vertex3d;
    host glVertexPointer(4) => gl_vertex_pointer;
    host glViewport(4) => gl_viewport;
    host glClipPlane(2) => gl_noop;

    // GL 1.2–3.3 exports (resolved via wglGetProcAddress).
    host glActiveTexture(1) => gl_active_texture;
    host glClientActiveTexture(1) => gl_client_active_texture;
    host glMultiTexCoord2f(3) => gl_multi_tex_coord2f;
    host glMultiTexCoord4f(5) => gl_multi_tex_coord4f;
    host glSecondaryColor3f(3) => gl_secondary_color3f;
    host glSecondaryColorPointer(4) => gl_secondary_color_pointer;
    host glFogCoordf(1) => gl_fog_coordf;
    host glFogCoordPointer(3) => gl_fog_coord_pointer;
    host glLoadTransposeMatrixf(1) => gl_load_transpose_matrixf;
    host glMultTransposeMatrixf(1) => gl_mult_transpose_matrixf;
    host glBlendFuncSeparate(4) => gl_blend_func_separate;
    host glBlendEquation(1) => gl_blend_equation;
    host glBlendEquationSeparate(2) => gl_blend_equation_separate;
    host glStencilFuncSeparate(4) => gl_noop;
    host glStencilOpSeparate(4) => gl_noop;
    host glStencilMaskSeparate(2) => gl_noop;
    host glClearDepthf(1) => gl_clear_depthf;
    host glDepthRangef(2) => gl_depth_rangef;
    host glClearBufferfv(3) => gl_clear_bufferfv;
    host glClearBufferiv(3) => gl_clear_bufferiv;
    host glClearBufferuiv(3) => gl_clear_bufferuiv;
    host glClearBufferfi(4) => gl_clear_bufferfi;
    host glGetInteger64v(2) => gl_get_integer64v;
    host glGetStringi(2) => gl_get_stringi;
    host glCreateShader(1) => gl_create_shader;
    host glShaderSource(4) => gl_shader_source;
    host glCompileShader(1) => gl_compile_shader;
    host glDeleteShader(1) => gl_delete_shader;
    host glGetShaderiv(3) => gl_get_shaderiv;
    host glGetShaderInfoLog(4) => gl_get_shader_info_log;
    host glGetShaderPrecisionFormat(4) => gl_get_shader_precision_format;
    host glReleaseShaderCompiler(0) => gl_noop;
    host glShaderBinary(5) => gl_shader_binary;
    host glCreateProgram(0) => gl_create_program;
    host glAttachShader(2) => gl_attach_shader;
    host glDetachShader(2) => gl_detach_shader;
    host glBindAttribLocation(3) => gl_bind_attrib_location;
    host glLinkProgram(1) => gl_link_program;
    host glValidateProgram(1) => gl_validate_program;
    host glUseProgram(1) => gl_use_program;
    host glDeleteProgram(1) => gl_delete_program;
    host glGetProgramiv(3) => gl_get_programiv;
    host glGetProgramInfoLog(4) => gl_get_program_info_log;
    host glGetUniformLocation(2) => gl_get_uniform_location;
    host glGetAttribLocation(2) => gl_get_attrib_location;
    host glGetActiveUniform(7) => gl_get_active_uniform;
    host glGetActiveAttrib(7) => gl_get_active_attrib;
    host glGetAttachedShaders(4) => gl_get_attached_shaders;
    host glProgramParameteri(3) => gl_noop;
    host glUniform1f(2) => gl_uniform1f;
    host glUniform2f(3) => gl_uniform2f;
    host glUniform3f(4) => gl_uniform3f;
    host glUniform4f(5) => gl_uniform4f;
    host glUniform1i(2) => gl_uniform1i;
    host glUniform2i(3) => gl_uniform2i;
    host glUniform3i(4) => gl_uniform3i;
    host glUniform4i(5) => gl_uniform4i;
    host glUniform1ui(2) => gl_uniform1ui;
    host glUniform2ui(3) => gl_uniform2ui;
    host glUniform3ui(4) => gl_uniform3ui;
    host glUniform4ui(5) => gl_uniform4ui;
    host glUniform1fv(3) => gl_uniform1fv;
    host glUniform2fv(3) => gl_uniform2fv;
    host glUniform3fv(3) => gl_uniform3fv;
    host glUniform4fv(3) => gl_uniform4fv;
    host glUniform1iv(3) => gl_uniform1iv;
    host glUniform2iv(3) => gl_uniform2iv;
    host glUniform3iv(3) => gl_uniform3iv;
    host glUniform4iv(3) => gl_uniform4iv;
    host glUniform1uiv(3) => gl_uniform1uiv;
    host glUniform2uiv(3) => gl_uniform2uiv;
    host glUniform3uiv(3) => gl_uniform3uiv;
    host glUniform4uiv(3) => gl_uniform4uiv;
    host glUniformMatrix2fv(4) => gl_uniform_matrix2fv;
    host glUniformMatrix3fv(4) => gl_uniform_matrix3fv;
    host glUniformMatrix4fv(4) => gl_uniform_matrix4fv;
    host glUniformMatrix2x3fv(4) => gl_uniform_matrix2x3fv;
    host glUniformMatrix3x2fv(4) => gl_uniform_matrix3x2fv;
    host glUniformMatrix2x4fv(4) => gl_uniform_matrix2x4fv;
    host glUniformMatrix4x2fv(4) => gl_uniform_matrix4x2fv;
    host glUniformMatrix3x4fv(4) => gl_uniform_matrix3x4fv;
    host glUniformMatrix4x3fv(4) => gl_uniform_matrix4x3fv;
    host glGenBuffers(2) => gl_gen_names;
    host glDeleteBuffers(2) => gl_delete_buffers;
    host glBindBuffer(2) => gl_bind_buffer;
    host glBufferData(4) => gl_buffer_data;
    host glBufferSubData(4) => gl_buffer_sub_data;
    host glMapBuffer(2) => gl_map_buffer;
    host glMapBufferRange(4) => gl_map_buffer_range;
    host glFlushMappedBufferRange(3) => gl_flush_mapped_buffer_range;
    host glUnmapBuffer(1) => gl_unmap_buffer;
    host glGetBufferPointerv(3) => gl_get_buffer_pointerv;
    host glGetBufferParameteriv(3) => gl_get_buffer_parameteriv;
    host glGetBufferParameteri64v(3) => gl_get_buffer_parameteri64v;
    host glBindBufferBase(3) => gl_bind_buffer_base;
    host glBindBufferRange(5) => gl_bind_buffer_range;
    host glGenVertexArrays(2) => gl_gen_names;
    host glDeleteVertexArrays(2) => gl_delete_vertex_arrays;
    host glBindVertexArray(1) => gl_bind_vertex_array;
    host glVertexAttribPointer(6) => gl_vertex_attrib_pointer;
    host glVertexAttribIPointer(5) => gl_vertex_attrib_i_pointer;
    host glEnableVertexAttribArray(1) => gl_enable_vertex_attrib_array;
    host glDisableVertexAttribArray(1) => gl_disable_vertex_attrib_array;
    host glVertexAttribDivisor(2) => gl_vertex_attrib_divisor;
    host glVertexAttrib1f(2) => gl_vertex_attrib1f;
    host glVertexAttrib2f(3) => gl_vertex_attrib2f;
    host glVertexAttrib3f(4) => gl_vertex_attrib3f;
    host glVertexAttrib4f(5) => gl_vertex_attrib4f;
    host glVertexAttrib1fv(2) => gl_vertex_attrib1fv;
    host glVertexAttrib2fv(2) => gl_vertex_attrib2fv;
    host glVertexAttrib3fv(2) => gl_vertex_attrib3fv;
    host glVertexAttrib4fv(2) => gl_vertex_attrib4fv;
    host glVertexAttribI4i(5) => gl_vertex_attrib_i4i;
    host glVertexAttribI4ui(5) => gl_vertex_attrib_i4ui;
    host glVertexAttribI4iv(2) => gl_vertex_attrib_i4iv;
    host glVertexAttribI4uiv(2) => gl_vertex_attrib_i4uiv;
    host glTexStorage2D(5) => gl_tex_storage_2d;
    host glGenerateMipmap(1) => gl_generate_mipmap;
    host glGenFramebuffers(2) => gl_gen_names;
    host glDeleteFramebuffers(2) => gl_delete_framebuffers;
    host glBindFramebuffer(2) => gl_bind_framebuffer;
    host glFramebufferTexture2D(5) => gl_framebuffer_texture_2d;
    host glFramebufferTexture(4) => gl_framebuffer_texture;
    host glFramebufferRenderbuffer(4) => gl_framebuffer_renderbuffer;
    host glCheckFramebufferStatus(1) => gl_check_framebuffer_status;
    host glGenRenderbuffers(2) => gl_gen_names;
    host glDeleteRenderbuffers(2) => gl_delete_renderbuffers;
    host glBindRenderbuffer(2) => gl_bind_renderbuffer;
    host glRenderbufferStorage(4) => gl_renderbuffer_storage;
    host glRenderbufferStorageMultisample(5) => gl_renderbuffer_storage_multisample;
    host glBlitFramebuffer(10) => gl_blit_framebuffer;
    host glDrawBuffers(2) => gl_draw_buffers;
    host glInvalidateFramebuffer(3) => gl_noop;
    host glInvalidateSubFramebuffer(7) => gl_noop;
    host glGetUniformBlockIndex(2) => gl_get_uniform_block_index;
    host glUniformBlockBinding(3) => gl_uniform_block_binding;
    host glGetActiveUniformBlockiv(4) => gl_get_active_uniform_blockiv;
    host glGetActiveUniformBlockName(5) => gl_get_active_uniform_block_name;
    host glGenSamplers(2) => gl_gen_names;
    host glDeleteSamplers(2) => gl_delete_samplers;
    host glBindSampler(2) => gl_bind_sampler;
    host glSamplerParameteri(3) => gl_sampler_parameteri;
    host glSamplerParameterf(3) => gl_sampler_parameterf;
    host glSamplerParameteriv(3) => gl_sampler_parameteriv;
    host glSamplerParameterfv(3) => gl_sampler_parameterfv;
    host glDrawArraysInstanced(4) => gl_draw_arrays_instanced;
    host glDrawElementsInstanced(5) => gl_draw_elements_instanced;
    host glDrawElementsBaseVertex(5) => gl_draw_elements_base_vertex;
    host glDrawRangeElements(6) => gl_draw_range_elements;
    host glIsBuffer(1) => gl_is_name;
    host glIsProgram(1) => gl_is_name;
    host glIsShader(1) => gl_is_name;
    host glIsFramebuffer(1) => gl_is_name;
    host glIsRenderbuffer(1) => gl_is_name;
    host glIsVertexArray(1) => gl_is_name;
    host glIsSampler(1) => gl_is_name;
    host glFenceSync(2) => gl_fence_sync;
    host glClientWaitSync(3) => gl_client_wait_sync;
    host glWaitSync(3) => gl_noop;
    host glDeleteSync(1) => gl_noop;
    host glIsSync(1) => gl_is_sync;
    host glGetSynciv(5) => gl_get_synciv;
    host glSampleCoverage(2) => gl_noop;
    host glGetInternalformativ(5) => gl_get_internalformativ;
    host glBlendColor(4) => gl_noop;

    // WGL core and extension exports.
    host wglCreateContext(1) => wgl_create_context;
    host wglCreateContextAttribsARB(3) => wgl_create_context_attribs_arb;
    host wglDeleteContext(1) => wgl_delete_context;
    host wglGetCurrentContext(0) => wgl_get_current_context;
    host wglGetCurrentDC(0) => wgl_get_current_dc;
    host wglGetProcAddress(1) => wgl_get_proc_address;
    host wglMakeCurrent(2) => wgl_make_current;
    host wglShareLists(2) => wgl_share_lists;
    host wglGetExtensionsStringARB(1) => wgl_get_extensions_string;
    host wglGetExtensionsStringEXT(0) => wgl_get_extensions_string;
    host wglChoosePixelFormatARB(6) => wgl_choose_pixel_format_arb;
    host wglGetPixelFormatAttribivARB(6) => wgl_get_pixel_format_attribiv_arb;
    host wglGetPixelFormatAttribfvARB(6) => wgl_get_pixel_format_attribfv_arb;
    host wglSwapIntervalEXT(1) => wgl_swap_interval_ext;
    host wglGetSwapIntervalEXT(0) => wgl_get_swap_interval_ext;
}

const ERROR_NOT_SUPPORTED: u32 = 50;
const INVALID_ENUM: u32 = 0x0500;
const INVALID_VALUE: u32 = 0x0501;
const INVALID_OPERATION: u32 = 0x0502;
const FLOAT: u32 = 0x1406;
const SWAP_SENTINEL: u64 = 0xffff_ffff_0000_0000;
const IMM_VBO: u32 = 0x7fff_ffe0;
const IMM_VAO: u32 = 0x7fff_ffe1;

const EXTENSIONS_LIST: &[&str] = &[
    "GL_ARB_framebuffer_object",
    "GL_ARB_ES2_compatibility",
    "GL_ARB_ES3_compatibility",
    "GL_EXT_framebuffer_object",
    "GL_EXT_framebuffer_blit",
    "GL_ARB_draw_buffers",
    "GL_ARB_depth_texture",
    "GL_EXT_packed_depth_stencil",
    "GL_ARB_texture_float",
    "GL_ARB_half_float_pixel",
    "GL_ARB_vertex_buffer_object",
    "GL_ARB_vertex_array_object",
    "GL_ARB_texture_non_power_of_two",
    "GL_ARB_multitexture",
    "GL_ARB_texture_env_combine",
    "GL_EXT_texture_env_combine",
    "GL_ARB_texture_env_add",
    "GL_EXT_bgra",
    "GL_EXT_vertex_array_bgra",
    "GL_EXT_blend_func_separate",
    "GL_EXT_blend_minmax",
    "GL_EXT_blend_subtract",
    "GL_ARB_texture_mirrored_repeat",
    "GL_ARB_texture_border_clamp",
    "GL_EXT_texture_edge_clamp",
    "GL_ARB_instanced_arrays",
    "GL_ARB_draw_instanced",
    "GL_ARB_explicit_attrib_location",
    "GL_ARB_shading_language_100",
    "GL_ARB_shader_objects",
    "GL_ARB_vertex_shader",
    "GL_ARB_fragment_shader",
    "GL_ARB_uniform_buffer_object",
    "GL_ARB_sampler_objects",
    "GL_ARB_map_buffer_range",
];

const WGL_EXTENSIONS: &str = "WGL_ARB_extensions_string WGL_EXT_extensions_string WGL_ARB_pixel_format WGL_ARB_create_context WGL_ARB_create_context_profile WGL_EXT_swap_control WGL_ARB_multisample";

// ---- Per-process GL state ----

pub(crate) struct GlState {
    gl: Gl,
    inner: Mutex<GlInner>,
}

struct GlInner {
    next_ctx: u64,
    next_name: u32,
    contexts: HashMap<u64, CtxState>,
    current: HashMap<u64, (u64, u64)>, // tid -> (hdc, hglrc)
    strings_base: u64,
    strings_used: usize,
    strings_cap: usize,
    strings: HashMap<String, u64>,
    swap_interval: i32,
    last_swap: Option<Instant>,
}

struct PixelStore {
    values: [(u32, i32); 8],
}

impl Default for PixelStore {
    fn default() -> Self {
        Self {
            values: [
                (0x0d02, 0), // PACK_ROW_LENGTH
                (0x0d03, 0), // PACK_SKIP_ROWS
                (0x0d04, 0), // PACK_SKIP_PIXELS
                (0x0d05, 4), // PACK_ALIGNMENT
                (0x0cf2, 0), // UNPACK_ROW_LENGTH
                (0x0cf3, 0), // UNPACK_SKIP_ROWS
                (0x0cf4, 0), // UNPACK_SKIP_PIXELS
                (0x0cf5, 4), // UNPACK_ALIGNMENT
            ],
        }
    }
}

impl PixelStore {
    fn get(&self, name: u32) -> Option<i32> {
        self.values.iter().find(|v| v.0 == name).map(|v| v.1)
    }

    fn set(&mut self, name: u32, value: i32) -> Result<(), u32> {
        let Some(v) = self.values.iter_mut().find(|v| v.0 == name) else { return Err(INVALID_ENUM) };
        let alignment = name == 0x0d05 || name == 0x0cf5;
        if (alignment && ![1, 2, 4, 8].contains(&value)) || value < 0 {
            return Err(INVALID_VALUE);
        }
        v.1 = value;
        Ok(())
    }
}

struct BufferMap {
    target: u32,
    offset: u64,
    length: usize,
    guest_addr: u64,
    access: u32,
}

#[derive(Clone, Copy)]
struct ImmVert {
    pos: [f32; 4],
    normal: [f32; 3],
    color: [f32; 4],
    tex0: [f32; 4],
}

struct ImmBatch {
    mode: u32,
    verts: Vec<ImmVert>,
}

struct CtxState {
    store: PixelStore,
    profile_mask: i32,
    client_unit: u32,
    bound_vao: u32,
    bound_array_buffer: u32,
    drawable: u64,
    drawable_size: (u32, u32),
    maps: Vec<BufferMap>,
    imm_color: [f32; 4],
    imm_normal: [f32; 3],
    imm_tex0: [f32; 4],
    imm: Option<ImmBatch>,
}

impl Default for CtxState {
    fn default() -> Self {
        Self {
            store: PixelStore::default(),
            profile_mask: 2,
            client_unit: 0,
            bound_vao: 0,
            bound_array_buffer: 0,
            drawable: 0,
            drawable_size: (0, 0),
            maps: Vec::new(),
            imm_color: [1.0, 1.0, 1.0, 1.0],
            imm_normal: [0.0, 0.0, 1.0],
            imm_tex0: [0.0, 0.0, 0.0, 1.0],
            imm: None,
        }
    }
}

/// Guest memory adapter for `gpu_api::gl::Gl::execute`, capturing `SWAP_SENTINEL` writes into a host buffer.
struct AtlasGuest<'a, 'b> {
    task: &'a mut Task,
    cx: &'a mut Cx<'b>,
    swap_out: Option<Vec<u8>>,
}

impl Guest for AtlasGuest<'_, '_> {
    fn read(&mut self, addr: u64, out: &mut [u8]) -> Result<(), String> {
        let bytes = self.task.mem(self.cx, false, |t| user::read_vec(&t.space(), addr, out.len())).map_err(|_| format!("fault reading {addr:#x}"))?;
        out.copy_from_slice(&bytes);
        Ok(())
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), String> {
        if addr >= SWAP_SENTINEL {
            let off = (addr - SWAP_SENTINEL) as usize;
            let buf = self.swap_out.get_or_insert_with(Vec::new);
            if buf.len() < off + data.len() {
                buf.resize(off + data.len(), 0);
            }
            buf[off..off + data.len()].copy_from_slice(data);
            return Ok(());
        }
        self.task.mem(self.cx, true, |t| user::write(&mut t.space(), addr, data)).map_err(|_| format!("fault writing {addr:#x}"))
    }
}

fn ensure_state(task: &mut Task) -> Option<Arc<GlState>> {
    let mut ps = lock(&task.process.state);
    if let Some(gl) = &ps.gl {
        return Some(gl.clone());
    }
    let gpu = task.k.options.gpu.clone()?;
    let st = Arc::new(GlState {
        gl: Gl::new(gpu),
        inner: Mutex::new(GlInner {
            next_ctx: 0x474c_0001,
            next_name: 1,
            contexts: HashMap::new(),
            current: HashMap::new(),
            strings_base: 0,
            strings_used: 0,
            strings_cap: 0,
            strings: HashMap::new(),
            swap_interval: 0,
            last_swap: None,
        }),
    });
    ps.gl = Some(st.clone());
    Some(st)
}

fn current_ctx(task: &Task) -> Option<(Arc<GlState>, u64)> {
    let st = lock(&task.process.state).gl.clone()?;
    let hglrc = lock(&st.inner).current.get(&task.tid).map(|v| v.1)?;
    (hglrc != 0).then_some((st, hglrc))
}

fn exec_records(task: &mut Task, cx: &mut Cx<'_>, st: &GlState, hglrc: u64, recs: &[u8]) -> Option<Vec<u8>> {
    let mut batch = Vec::with_capacity(recs.len() + 16);
    if hglrc != 0 {
        records::record(&mut batch, wire::USE, &hglrc.to_le_bytes());
    }
    batch.extend_from_slice(recs);
    let mut guest = AtlasGuest { task, cx, swap_out: None };
    let reply = st.gl.execute(&batch, &mut guest);
    let mut r = Reader::new(&reply);
    if r.u32().ok()? != wire::OK {
        return None;
    }
    Some(reply[4..].to_vec())
}

fn send_record(task: &mut Task, cx: &mut Cx<'_>, kind: u32, payload: &[u8]) -> Option<Vec<u8>> {
    let (st, hglrc) = current_ctx(task)?;
    let mut recs = Vec::with_capacity(payload.len() + 8);
    records::record(&mut recs, kind, payload);
    exec_records(task, cx, &st, hglrc, &recs)
}

fn send_words(task: &mut Task, cx: &mut Cx<'_>, kind: u32, words: &[u32]) -> Option<Vec<u8>> {
    let mut p = Vec::with_capacity(words.len() * 4);
    for &w in words {
        records::put_u32(&mut p, w);
    }
    send_record(task, cx, kind, &p)
}

fn send_error(task: &mut Task, cx: &mut Cx<'_>, code: u32) {
    let _ = send_words(task, cx, wire::ERROR, &[code]);
}

fn f64_arg(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], index: usize, float: usize) -> f64 {
    let fp = cx.turn(|t| t.thread().fp());
    match (task.isa, fp) {
        (Isa::X86_64, FpRegs::X86(f)) if index < 4 => {
            let at = 160 + 16 * index;
            f64::from_le_bytes(f.fxsave[at..at + 8].try_into().unwrap())
        }
        (Isa::Arm64, FpRegs::Arm64(f)) if float < 8 => f64::from_bits(f.v[float] as u64),
        _ => f64::from_bits(a.get(index).copied().unwrap_or(0)),
    }
}

fn int_after_floats(task: &Task, a: &[u64], x64_idx: usize, arm64_idx: usize) -> u64 {
    let idx = if task.isa == Isa::Arm64 { arm64_idx } else { x64_idx };
    a.get(idx).copied().unwrap_or(0)
}

fn read_cstr(task: &mut Task, cx: &mut Cx<'_>, addr: u64) -> String {
    if addr == 0 {
        return String::new();
    }
    task.text_arg(cx, addr, false)
}

fn write_cstr_out(task: &mut Task, cx: &mut Cx<'_>, text: &[u8], size: i32, length_ptr: u64, out_ptr: u64) {
    let n = if size > 0 { text.len().min(size as usize - 1) } else { 0 };
    if out_ptr != 0 && size > 0 {
        let mut buf = Vec::with_capacity(n + 1);
        buf.extend_from_slice(&text[..n]);
        buf.push(0);
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out_ptr, &buf));
    }
    if length_ptr != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), length_ptr, &(n as i32).to_le_bytes()));
    }
}

fn intern_string(task: &mut Task, cx: &mut Cx<'_>, st: &GlState, text: &str) -> u64 {
    if let Some(&addr) = lock(&st.inner).strings.get(text) {
        return addr;
    }
    let need = text.len() + 1;
    let (base, off) = {
        let mut inner = lock(&st.inner);
        if inner.strings_base == 0 || inner.strings_used + need > inner.strings_cap {
            let cap = need.max(0x10000);
            drop(inner);
            let Some(page) = task.virtual_alloc(cx, 0, cap as u64, 0x3000, 0x04) else { return 0 };
            let mut inner = lock(&st.inner);
            inner.strings_base = page;
            inner.strings_used = 0;
            inner.strings_cap = cap;
            let off = inner.strings_used;
            inner.strings_used += need;
            (inner.strings_base, off)
        } else {
            let off = inner.strings_used;
            inner.strings_used += need;
            (inner.strings_base, off)
        }
    };
    let addr = base + off as u64;
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), addr, &bytes));
    lock(&st.inner).strings.insert(text.to_string(), addr);
    addr
}

fn window_size_for_dc(task: &Task, hdc: u64) -> (u64, u32, u32) {
    let w = task.win32k();
    let hwnd = w.gdi.dcs.get(&hdc).map(|dc| dc.hwnd).unwrap_or(0);
    let (width, height) = w.windows.get(&hwnd).map_or((640u32, 480u32), |win| {
        let c = win.client();
        ((c.right - c.left).max(1) as u32, (c.bottom - c.top).max(1) as u32)
    });
    let drawable = if hwnd != 0 { hwnd } else { hdc.max(1) };
    (drawable, width, height)
}

fn bind_drawable(task: &mut Task, cx: &mut Cx<'_>, st: &GlState, hglrc: u64, drawable: u64, width: u32, height: u32) {
    let mut p = Vec::with_capacity(28);
    records::put_u64(&mut p, drawable);
    for w in [width, height, 24u32, 8u32, 1u32] {
        records::put_u32(&mut p, w);
    }
    let mut recs = Vec::with_capacity(36);
    records::record(&mut recs, wire::BIND, &p);
    let _ = exec_records(task, cx, st, hglrc, &recs);
    if let Some(c) = lock(&st.inner).contexts.get_mut(&hglrc) {
        c.drawable = drawable;
        c.drawable_size = (width, height);
    }
}

// ---- SwapBuffers (called from gdi32::swap_buffers) ----

pub(crate) fn swap_buffers(task: &mut Task, cx: &mut Cx<'_>, hdc: u64) -> Reply {
    let Some(st) = lock(&task.process.state).gl.clone() else {
        return Reply::Value(1);
    };
    let (cur_hdc, hglrc) = lock(&st.inner).current.get(&task.tid).copied().unwrap_or((0, 0));
    if hglrc == 0 {
        return Reply::Value(1);
    }
    let dc = if hdc != 0 { hdc } else { cur_hdc };
    let (hwnd, w, h) = window_size_for_dc(task, dc);
    let (bound_drawable, bound_size) = lock(&st.inner).contexts.get(&hglrc).map(|c| (c.drawable, c.drawable_size)).unwrap_or((0, (0, 0)));
    let target = if bound_drawable != 0 { bound_drawable } else { hwnd };
    let (sw, sh) = if bound_size.0 > 0 && bound_size.1 > 0 { bound_size } else { (w, h) };

    let mut recs = Vec::with_capacity(48);
    records::record(&mut recs, wire::USE, &hglrc.to_le_bytes());
    let mut sp = Vec::with_capacity(20);
    records::put_u64(&mut sp, target);
    records::put_u64(&mut sp, SWAP_SENTINEL);
    records::put_u32(&mut sp, sw * 4);
    records::record(&mut recs, wire::SWAP, &sp);

    let mut guest = AtlasGuest { task, cx, swap_out: Some(vec![0u8; (sw * sh * 4) as usize]) };
    let reply = st.gl.execute(&recs, &mut guest);
    let ok = Reader::new(&reply).u32().is_ok_and(|s| s == wire::OK);
    let pixels_bytes = guest.swap_out.take().unwrap_or_default();

    if ok && !pixels_bytes.is_empty() && hwnd != 0 {
        let pixels: Vec<display::Rgb> = pixels_bytes
            .chunks_exact(4)
            .map(|p| u32::from_le_bytes([p[0], p[1], p[2], 0]))
            .collect();
        if pixels.len() == (sw as usize) * (sh as usize) {
            task.win32k().present(hwnd, sw as i32, sh as i32, &pixels);
        }
    }

    // Rebind framebuffer if the window was resized.
    if (w, h) != (sw, sh) || target != hwnd {
        bind_drawable(task, cx, &st, hglrc, hwnd, w, h);
    }

    // Pace frame if swap interval > 0.
    let (interval, last) = {
        let inner = lock(&st.inner);
        (inner.swap_interval, inner.last_swap)
    };
    if interval > 0
        && let Some(prev) = last
    {
        let target_dur = Duration::from_micros(16_667) * (interval as u32);
        if let Some(left) = target_dur.checked_sub(prev.elapsed()) {
            std::thread::sleep(left);
        }
    }
    lock(&st.inner).last_swap = Some(Instant::now());
    Reply::Value(1)
}

// ---- WGL ----

fn create_context_common(task: &mut Task, cx: &mut Cx<'_>, share: u64, profile_mask: i32) -> Reply {
    let Some(st) = ensure_state(task) else {
        task.set_last_error(cx, ERROR_NOT_SUPPORTED);
        return Reply::Value(0);
    };
    let hglrc = {
        let mut inner = lock(&st.inner);
        let id = inner.next_ctx;
        inner.next_ctx += 1;
        inner.contexts.insert(id, CtxState { profile_mask, ..CtxState::default() });
        id
    };
    let mut p = Vec::with_capacity(16);
    records::put_u64(&mut p, hglrc);
    records::put_u64(&mut p, share);
    let mut recs = Vec::with_capacity(24);
    records::record(&mut recs, wire::CONTEXT, &p);
    if exec_records(task, cx, &st, 0, &recs).is_none() {
        lock(&st.inner).contexts.remove(&hglrc);
        return Reply::Value(0);
    }
    Reply::Value(hglrc)
}

fn wgl_create_context(task: &mut Task, cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    create_context_common(task, cx, 0, 2)
}

fn wgl_create_context_attribs_arb(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const WGL_CONTEXT_MAJOR_VERSION_ARB: i32 = 0x2091;
    const WGL_CONTEXT_PROFILE_MASK_ARB: i32 = 0x9126;
    let mut major = 1i32;
    let mut requested_mask = 0i32;
    if a[2] != 0 {
        for i in 0..64u64 {
            let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[2] + i * 8, 8)) else { break };
            let attr = i32::from_le_bytes(raw[0..4].try_into().unwrap());
            if attr == 0 {
                break;
            }
            let val = i32::from_le_bytes(raw[4..8].try_into().unwrap());
            match attr {
                WGL_CONTEXT_MAJOR_VERSION_ARB => major = val,
                WGL_CONTEXT_PROFILE_MASK_ARB => requested_mask = val,
                _ => {}
            }
        }
    }
    let profile_mask = if requested_mask != 0 {
        requested_mask
    } else if major >= 3 {
        1
    } else {
        2
    };
    create_context_common(task, cx, a[1], profile_mask)
}

fn wgl_delete_context(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let hglrc = a[0];
    let Some(st) = lock(&task.process.state).gl.clone() else { return Reply::Value(0) };
    {
        let mut inner = lock(&st.inner);
        inner.contexts.remove(&hglrc);
        inner.current.retain(|_, v| v.1 != hglrc);
    }
    let mut recs = Vec::with_capacity(16);
    records::record(&mut recs, wire::FREE_CONTEXT, &hglrc.to_le_bytes());
    let _ = exec_records(task, cx, &st, 0, &recs);
    Reply::Value(1)
}

fn wgl_make_current(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (hdc, hglrc) = (a[0], a[1]);
    if hglrc == 0 {
        if let Some(st) = lock(&task.process.state).gl.clone() {
            lock(&st.inner).current.remove(&task.tid);
        }
        return Reply::Value(1);
    }
    let Some(st) = lock(&task.process.state).gl.clone() else { return Reply::Value(0) };
    if !lock(&st.inner).contexts.contains_key(&hglrc) {
        return Reply::Value(0);
    }
    lock(&st.inner).current.insert(task.tid, (hdc, hglrc));
    let (drawable, w, h) = window_size_for_dc(task, hdc);
    bind_drawable(task, cx, &st, hglrc, drawable, w, h);
    Reply::Value(1)
}

fn wgl_get_current_context(task: &mut Task, _cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    let h = lock(&task.process.state)
        .gl
        .as_ref()
        .and_then(|st| lock(&st.inner).current.get(&task.tid).map(|v| v.1))
        .unwrap_or(0);
    Reply::Value(h)
}

fn wgl_get_current_dc(task: &mut Task, _cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    let h = lock(&task.process.state)
        .gl
        .as_ref()
        .and_then(|st| lock(&st.inner).current.get(&task.tid).map(|v| v.0))
        .unwrap_or(0);
    Reply::Value(h)
}

fn wgl_share_lists(_task: &mut Task, _cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    // All contexts in `gpu_api::gl::Gl` share one object table already.
    Reply::Value(1)
}

fn resolve_gl_proc_name(name: &str) -> String {
    match name {
        "glCreateShaderObjectARB" => return "glCreateShader".to_string(),
        "glCreateProgramObjectARB" => return "glCreateProgram".to_string(),
        "glAttachObjectARB" => return "glAttachShader".to_string(),
        "glDetachObjectARB" => return "glDetachShader".to_string(),
        "glUseProgramObjectARB" => return "glUseProgram".to_string(),
        "glDeleteObjectARB" => return "glDeleteProgram".to_string(),
        "glGetObjectParameterivARB" => return "glGetProgramiv".to_string(),
        "glGetInfoLogARB" => return "glGetProgramInfoLog".to_string(),
        "glGetAttachedObjectsARB" => return "glGetAttachedShaders".to_string(),
        _ => {}
    }
    for suffix in ["ARB", "EXT", "OES"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            if DLL.exports.iter().any(|e| e.name == stripped) {
                return stripped.to_string();
            }
        }
    }
    name.to_string()
}

fn wgl_get_proc_address(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let raw = read_cstr(task, cx, a[0]);
    if raw.is_empty() {
        return Reply::Value(0);
    }
    if let Some(addr) = task.system_export("OPENGL32.dll", &raw) {
        return Reply::Value(addr);
    }
    let resolved = resolve_gl_proc_name(&raw);
    if resolved != raw
        && let Some(addr) = task.system_export("OPENGL32.dll", &resolved)
    {
        return Reply::Value(addr);
    }
    Reply::Value(0)
}

fn wgl_get_extensions_string(task: &mut Task, cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    let Some(st) = ensure_state(task) else { return Reply::Value(0) };
    Reply::Value(intern_string(task, cx, &st, WGL_EXTENSIONS))
}

fn wgl_choose_pixel_format_arb(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (max_formats, formats_ptr, num_ptr) = (a[3] as u32, a[4], a[5]);
    if max_formats > 0 && formats_ptr != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), formats_ptr, &1i32.to_le_bytes()));
    }
    if num_ptr != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), num_ptr, &1u32.to_le_bytes()));
    }
    Reply::Value(1)
}

fn wgl_pixel_format_attrib(attr: u32) -> i32 {
    match attr {
        0x2000 => 1,      // WGL_NUMBER_PIXEL_FORMATS_ARB
        0x2001 => 1,      // WGL_DRAW_TO_WINDOW_ARB
        0x2002 => 0,      // WGL_DRAW_TO_BITMAP_ARB
        0x2003 => 0x2027, // WGL_ACCELERATION_ARB -> WGL_FULL_ACCELERATION_ARB
        0x2004 => 0,      // WGL_NEED_PALETTE_ARB
        0x2005 => 0,      // WGL_NEED_SYSTEM_PALETTE_ARB
        0x2006 => 0,      // WGL_SWAP_LAYER_BUFFERS_ARB
        0x2007 => 0x2028, // WGL_SWAP_METHOD_ARB -> WGL_SWAP_EXCHANGE_ARB
        0x2008 | 0x2009 => 0,
        0x2010 => 1,      // WGL_SUPPORT_OPENGL_ARB
        0x2011 => 1,      // WGL_DOUBLE_BUFFER_ARB
        0x2012 => 0,      // WGL_STEREO_ARB
        0x2013 => 0x202b, // WGL_PIXEL_TYPE_ARB -> WGL_TYPE_RGBA_ARB
        0x2014 => 32,     // WGL_COLOR_BITS_ARB
        0x2015 | 0x2017 | 0x2019 | 0x201b => 8, // R/G/B/A bits
        0x2016 => 16,     // WGL_RED_SHIFT_ARB
        0x2018 => 8,      // WGL_GREEN_SHIFT_ARB
        0x201a => 0,      // WGL_BLUE_SHIFT_ARB
        0x201c => 24,     // WGL_ALPHA_SHIFT_ARB
        0x2022 => 24,     // WGL_DEPTH_BITS_ARB
        0x2023 => 8,      // WGL_STENCIL_BITS_ARB
        0x2041 | 0x2042 => 0, // SAMPLE_BUFFERS / SAMPLES
        0x20a9 => 1,      // WGL_FRAMEBUFFER_SRGB_CAPABLE_ARB
        _ => 0,
    }
}

fn wgl_get_pixel_format_attribiv_arb(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (n, attrs_ptr, vals_ptr) = (a[3] as u32 as usize, a[4], a[5]);
    if attrs_ptr == 0 || vals_ptr == 0 || n == 0 || n > 1024 {
        return Reply::Value(1);
    }
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), attrs_ptr, n * 4)) else { return Reply::Value(0) };
    let mut out = Vec::with_capacity(n * 4);
    for chunk in raw.chunks_exact(4) {
        let attr = u32::from_le_bytes(chunk.try_into().unwrap());
        out.extend_from_slice(&wgl_pixel_format_attrib(attr).to_le_bytes());
    }
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), vals_ptr, &out));
    Reply::Value(1)
}

fn wgl_get_pixel_format_attribfv_arb(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (n, attrs_ptr, vals_ptr) = (a[3] as u32 as usize, a[4], a[5]);
    if attrs_ptr == 0 || vals_ptr == 0 || n == 0 || n > 1024 {
        return Reply::Value(1);
    }
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), attrs_ptr, n * 4)) else { return Reply::Value(0) };
    let mut out = Vec::with_capacity(n * 4);
    for chunk in raw.chunks_exact(4) {
        let attr = u32::from_le_bytes(chunk.try_into().unwrap());
        out.extend_from_slice(&(wgl_pixel_format_attrib(attr) as f32).to_le_bytes());
    }
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), vals_ptr, &out));
    Reply::Value(1)
}

fn wgl_swap_interval_ext(task: &mut Task, _cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(st) = lock(&task.process.state).gl.clone() {
        lock(&st.inner).swap_interval = a[0] as u32 as i32;
    }
    Reply::Value(1)
}

fn wgl_get_swap_interval_ext(task: &mut Task, _cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    let v = lock(&task.process.state).gl.as_ref().map(|st| lock(&st.inner).swap_interval).unwrap_or(0);
    Reply::Value(v as u32 as u64)
}

// ---- Basic GL state & clears ----

fn gl_noop(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn gl_zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn gl_clear_color(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = task.f32_arg(cx, a, 0, 0).to_bits();
    let g = task.f32_arg(cx, a, 1, 1).to_bits();
    let b = task.f32_arg(cx, a, 2, 2).to_bits();
    let al = task.f32_arg(cx, a, 3, 3).to_bits();
    let _ = send_words(task, cx, wire::CLEAR_COLOR, &[r, g, b, al]);
    Reply::Value(0)
}

fn gl_clear_depth(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let d = f64_arg(task, cx, a, 0, 0);
    let _ = send_record(task, cx, wire::CLEAR_DEPTH, &d.to_bits().to_le_bytes());
    Reply::Value(0)
}

fn gl_clear_depthf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let d = f64::from(task.f32_arg(cx, a, 0, 0));
    let _ = send_record(task, cx, wire::CLEAR_DEPTH, &d.to_bits().to_le_bytes());
    Reply::Value(0)
}

fn gl_depth_range(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let near = f64_arg(task, cx, a, 0, 0);
    let far = f64_arg(task, cx, a, 1, 1);
    let mut p = Vec::with_capacity(16);
    records::put_u64(&mut p, near.to_bits());
    records::put_u64(&mut p, far.to_bits());
    let _ = send_record(task, cx, wire::DEPTH_RANGE, &p);
    Reply::Value(0)
}

fn gl_depth_rangef(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let near = f64::from(task.f32_arg(cx, a, 0, 0));
    let far = f64::from(task.f32_arg(cx, a, 1, 1));
    let mut p = Vec::with_capacity(16);
    records::put_u64(&mut p, near.to_bits());
    records::put_u64(&mut p, far.to_bits());
    let _ = send_record(task, cx, wire::DEPTH_RANGE, &p);
    Reply::Value(0)
}

fn gl_clear_stencil(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::CLEAR_STENCIL, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_clear(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::CLEAR, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_clear_bufferfv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (buf, db, ptr) = (a[0] as u32, (a[1] as u32 as i32).max(0) as u32, a[2]);
    if ptr == 0 {
        return Reply::Value(0);
    }
    let n = if buf == 0x1800 { 4 } else { 1 };
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), ptr, n * 4)) else { return Reply::Value(0) };
    let mut w = [0u32; 4];
    for (i, c) in raw.chunks_exact(4).enumerate() {
        w[i] = u32::from_le_bytes(c.try_into().unwrap());
    }
    let _ = send_words(task, cx, wire::CLEAR_BUFFER, &[buf, db, w[0], w[1], w[2], w[3]]);
    Reply::Value(0)
}

fn gl_clear_bufferiv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (buf, db, ptr) = (a[0] as u32, (a[1] as u32 as i32).max(0) as u32, a[2]);
    if ptr == 0 {
        return Reply::Value(0);
    }
    let n = if buf == 0x1800 { 4 } else { 1 };
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), ptr, n * 4)) else { return Reply::Value(0) };
    let w = if buf == 0x1800 {
        let mut out = [0u32; 4];
        for (i, c) in raw.chunks_exact(4).enumerate() {
            out[i] = (i32::from_le_bytes(c.try_into().unwrap()) as f32).to_bits();
        }
        out
    } else {
        [0, u32::from_le_bytes(raw[0..4].try_into().unwrap()), 0, 0]
    };
    let _ = send_words(task, cx, wire::CLEAR_BUFFER, &[buf, db, w[0], w[1], w[2], w[3]]);
    Reply::Value(0)
}

fn gl_clear_bufferuiv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (buf, db, ptr) = (a[0] as u32, (a[1] as u32 as i32).max(0) as u32, a[2]);
    if ptr == 0 {
        return Reply::Value(0);
    }
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), ptr, 16)) else { return Reply::Value(0) };
    let mut w = [0u32; 4];
    for (i, c) in raw.chunks_exact(4).enumerate() {
        w[i] = (u32::from_le_bytes(c.try_into().unwrap()) as f32).to_bits();
    }
    let _ = send_words(task, cx, wire::CLEAR_BUFFER, &[buf, db, w[0], w[1], w[2], w[3]]);
    Reply::Value(0)
}

fn gl_clear_bufferfi(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let buf = a[0] as u32;
    let db = (a[1] as u32 as i32).max(0) as u32;
    let depth = task.f32_arg(cx, a, 2, 0).to_bits();
    let stencil = int_after_floats(task, a, 3, 2) as u32;
    let _ = send_words(task, cx, wire::CLEAR_BUFFER, &[buf, db, depth, stencil, 0, 0]);
    Reply::Value(0)
}

fn gl_viewport(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::VIEWPORT, &[a[0] as u32, a[1] as u32, a[2] as u32, a[3] as u32]);
    Reply::Value(0)
}

fn gl_scissor(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::SCISSOR, &[a[0] as u32, a[1] as u32, a[2] as u32, a[3] as u32]);
    Reply::Value(0)
}

fn gl_enable(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::ENABLE, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_disable(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::DISABLE, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_is_enabled(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = send_words(task, cx, wire::IS_ENABLED, &[a[0] as u32])
        .and_then(|b| Reader::new(&b).u32().ok())
        .unwrap_or(0);
    Reply::Value(u64::from(v != 0))
}

fn gl_color_mask(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::COLOR_MASK, &[u32::from(a[0] as u8 != 0), u32::from(a[1] as u8 != 0), u32::from(a[2] as u8 != 0), u32::from(a[3] as u8 != 0)]);
    Reply::Value(0)
}

fn gl_depth_mask(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::DEPTH_MASK, &[u32::from(a[0] as u8 != 0)]);
    Reply::Value(0)
}

fn gl_finish(task: &mut Task, cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    let _ = send_record(task, cx, wire::FINISH, &[]);
    Reply::Value(0)
}

fn gl_flush(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    gl_finish(task, cx, a)
}

fn gl_get_error(task: &mut Task, cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    let err = send_record(task, cx, wire::GET_ERROR, &[])
        .and_then(|b| Reader::new(&b).u32().ok())
        .unwrap_or(0);
    Reply::Value(u64::from(err))
}

// ---- glGet* & glGetString* ----

#[derive(Clone, Copy)]
enum GlVal {
    Int(i64),
    Float(f64),
    Bool(bool),
}

fn get_values(task: &mut Task, cx: &mut Cx<'_>, pname: u32) -> Vec<GlVal> {
    let Some((st, hglrc)) = current_ctx(task) else { return Vec::new() };
    if let Some(v) = lock(&st.inner).contexts.get(&hglrc).and_then(|c| c.store.get(pname)) {
        return vec![GlVal::Int(i64::from(v))];
    }
    if pname == 0x821d {
        // GL_NUM_EXTENSIONS
        return vec![GlVal::Int(EXTENSIONS_LIST.len() as i64)];
    }
    if pname == 0x88ff {
        // GL_MAX_ARRAY_TEXTURE_LAYERS: 2D array textures are not yet supported in the GL backend.
        return vec![GlVal::Int(0)];
    }
    if pname == 0x9126 {
        // GL_CONTEXT_PROFILE_MASK
        let mask = lock(&st.inner).contexts.get(&hglrc).map(|c| c.profile_mask).unwrap_or(2);
        return vec![GlVal::Int(i64::from(mask))];
    }
    let Some(ans) = send_words(task, cx, wire::GET, &[pname]) else { return Vec::new() };
    let mut r = Reader::new(&ans);
    let n = r.u32().unwrap_or(0);
    (0..n)
        .map_while(|_| {
            let kind = r.u32().ok()?;
            let bits = r.u64().ok()?;
            Some(match kind {
                wire::INT => GlVal::Int(bits as i64),
                wire::FLOAT => GlVal::Float(f64::from_bits(bits)),
                _ => GlVal::Bool(bits != 0),
            })
        })
        .collect()
}

fn round_gl(f: f64) -> i64 {
    if f >= 0.0 { (f + 0.5) as i64 } else { (f - 0.5) as i64 }
}

fn as_f64(v: GlVal) -> f64 {
    match v {
        GlVal::Int(i) => i as f64,
        GlVal::Float(f) => f,
        GlVal::Bool(b) => f64::from(u8::from(b)),
    }
}

fn gl_get_integerv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (pname, out) = (a[0] as u32, a[1]);
    if out == 0 {
        return Reply::Value(0);
    }
    let vals = get_values(task, cx, pname);
    let spread = pname == 0x0c22 || pname == 0x0b73;
    let mut bytes = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        let i = match v {
            GlVal::Int(x) => x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            GlVal::Float(f) if spread => round_gl((f * (u32::MAX as f64) - 1.0) / 2.0) as i32,
            GlVal::Float(f) => round_gl(f) as i32,
            GlVal::Bool(b) => i32::from(b),
        };
        bytes.extend_from_slice(&i.to_le_bytes());
    }
    if !bytes.is_empty() {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes));
    }
    Reply::Value(0)
}

fn gl_get_integer64v(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (pname, out) = (a[0] as u32, a[1]);
    if out == 0 {
        return Reply::Value(0);
    }
    let vals = get_values(task, cx, pname);
    let mut bytes = Vec::with_capacity(vals.len() * 8);
    for v in vals {
        let i = match v {
            GlVal::Int(x) => x,
            GlVal::Float(f) => round_gl(f),
            GlVal::Bool(b) => i64::from(b),
        };
        bytes.extend_from_slice(&i.to_le_bytes());
    }
    if !bytes.is_empty() {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes));
    }
    Reply::Value(0)
}

fn gl_get_floatv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (pname, out) = (a[0] as u32, a[1]);
    if out == 0 {
        return Reply::Value(0);
    }
    let vals = get_values(task, cx, pname);
    let mut bytes = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        bytes.extend_from_slice(&(as_f64(v) as f32).to_le_bytes());
    }
    if !bytes.is_empty() {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes));
    }
    Reply::Value(0)
}

fn gl_get_doublev(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (pname, out) = (a[0] as u32, a[1]);
    if out == 0 {
        return Reply::Value(0);
    }
    let vals = get_values(task, cx, pname);
    let mut bytes = Vec::with_capacity(vals.len() * 8);
    for v in vals {
        bytes.extend_from_slice(&as_f64(v).to_le_bytes());
    }
    if !bytes.is_empty() {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes));
    }
    Reply::Value(0)
}

fn gl_get_booleanv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (pname, out) = (a[0] as u32, a[1]);
    if out == 0 {
        return Reply::Value(0);
    }
    let vals = get_values(task, cx, pname);
    let bytes: Vec<u8> = vals
        .into_iter()
        .map(|v| match v {
            GlVal::Int(i) => u8::from(i != 0),
            GlVal::Float(f) => u8::from(f != 0.0),
            GlVal::Bool(b) => u8::from(b),
        })
        .collect();
    if !bytes.is_empty() {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes));
    }
    Reply::Value(0)
}

fn gl_get_string(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, _)) = current_ctx(task) else { return Reply::Value(0) };
    let text: String = match a[0] as u32 {
        0x1f00 => "Carbon".to_string(),
        0x1f01 => {
            let dev = task.k.options.gpu.as_ref().map(|g| g.info().name.clone()).unwrap_or_else(|| "GPU".to_string());
            format!("Carbon GPU ({dev})")
        }
        0x1f02 => "3.3 Carbon 0.2.0".to_string(),
        0x1f03 => EXTENSIONS_LIST.join(" "),
        0x8b8c => "3.30".to_string(),
        _ => {
            send_error(task, cx, INVALID_ENUM);
            return Reply::Value(0);
        }
    };
    Reply::Value(intern_string(task, cx, &st, &text))
}

fn gl_get_stringi(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, _)) = current_ctx(task) else { return Reply::Value(0) };
    let (name, idx) = (a[0] as u32, a[1] as u32 as usize);
    if name != 0x1f03 {
        send_error(task, cx, INVALID_ENUM);
        return Reply::Value(0);
    }
    let Some(&ext) = EXTENSIONS_LIST.get(idx) else {
        send_error(task, cx, INVALID_VALUE);
        return Reply::Value(0);
    };
    Reply::Value(intern_string(task, cx, &st, ext))
}

fn gl_pixel_storei(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, hglrc)) = current_ctx(task) else { return Reply::Value(0) };
    let err = lock(&st.inner).contexts.get_mut(&hglrc).and_then(|c| c.store.set(a[0] as u32, a[1] as u32 as i32).err());
    if let Some(e) = err {
        send_error(task, cx, e);
    }
    Reply::Value(0)
}

fn gl_pixel_storef(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let val = round_gl(f64::from(task.f32_arg(cx, a, 1, 0))) as i32;
    gl_pixel_storei(task, cx, &[a[0], val as u32 as u64])
}

fn gl_read_pixels(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, hglrc)) = current_ctx(task) else { return Reply::Value(0) };
    let (x, y, width, height, format, kind, out) = (
        a[0] as u32, a[1] as u32, a[2] as u32 as i32, a[3] as u32 as i32, a[4] as u32, a[5] as u32, a[6],
    );
    if out == 0 {
        return Reply::Value(0);
    }
    let (row_len, alignment, skip_rows, skip_pixels) = {
        let inner = lock(&st.inner);
        let Some(c) = inner.contexts.get(&hglrc) else { return Reply::Value(0) };
        (
            c.store.get(0x0d02).unwrap_or(0),
            c.store.get(0x0d05).unwrap_or(4).max(1) as u64,
            c.store.get(0x0d03).unwrap_or(0).max(0) as u64,
            c.store.get(0x0d04).unwrap_or(0).max(0) as u64,
        )
    };
    let bytes = if format == 0x1907 { 3u64 } else { 4u64 };
    let row = (if row_len > 0 { row_len } else { width.max(0) }) as u64 * bytes;
    let stride = row.next_multiple_of(alignment);
    let skip = skip_rows * stride + skip_pixels * bytes;
    let mut p = Vec::with_capacity(36);
    for w in [x, y, width as u32, height as u32, format, kind] {
        records::put_u32(&mut p, w);
    }
    records::put_u64(&mut p, out + skip);
    records::put_u32(&mut p, stride as u32);
    let _ = send_record(task, cx, wire::READ_PIXELS, &p);
    Reply::Value(0)
}

fn gl_draw_buffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::DRAW_BUFFERS, &[1, a[0] as u32]);
    Reply::Value(0)
}

fn gl_read_buffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::READ_COLOR, &[a[0] as u32]);
    Reply::Value(0)
}

// ---- Names & object deletion helpers ----

fn alloc_name(st: &GlState) -> u32 {
    let mut inner = lock(&st.inner);
    let id = inner.next_name;
    inner.next_name += 1;
    id
}

fn gl_gen_names(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, _)) = current_ctx(task) else { return Reply::Value(0) };
    let (n, out) = (a[0] as u32 as i32, a[1]);
    if n <= 0 || out == 0 {
        return Reply::Value(0);
    }
    let mut bytes = Vec::with_capacity((n as usize) * 4);
    {
        let mut inner = lock(&st.inner);
        for _ in 0..n {
            let id = inner.next_name;
            inner.next_name += 1;
            bytes.extend_from_slice(&id.to_le_bytes());
        }
    }
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes));
    Reply::Value(0)
}

fn gl_gen_textures(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    gl_gen_names(task, cx, a)
}

fn delete_each(task: &mut Task, cx: &mut Cx<'_>, n: i32, list_ptr: u64, kind: u32) {
    if n <= 0 || list_ptr == 0 {
        return;
    }
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), list_ptr, (n as usize) * 4)) else { return };
    let Some((st, hglrc)) = current_ctx(task) else { return };
    let mut recs = Vec::with_capacity((n as usize) * 12);
    for c in raw.chunks_exact(4) {
        let name = u32::from_le_bytes(c.try_into().unwrap());
        if name != 0 {
            records::record(&mut recs, kind, &name.to_le_bytes());
        }
    }
    if !recs.is_empty() {
        let _ = exec_records(task, cx, &st, hglrc, &recs);
    }
}

fn gl_is_name(task: &mut Task, _cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, _)) = current_ctx(task) else { return Reply::Value(0) };
    let name = a[0] as u32;
    let next = lock(&st.inner).next_name;
    Reply::Value(u64::from(name != 0 && name < next))
}

// ---- Shaders and programs ----

fn gl_create_shader(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, _)) = current_ctx(task) else { return Reply::Value(0) };
    let name = alloc_name(&st);
    let _ = send_words(task, cx, wire::SHADER, &[name, a[0] as u32]);
    Reply::Value(u64::from(name))
}

fn gl_shader_source(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (shader, count, strings_ptr, lengths_ptr) = (a[0] as u32, a[1] as u32 as i32, a[2], a[3]);
    if count <= 0 || strings_ptr == 0 {
        return Reply::Value(0);
    }
    let abi = task.abi();
    let ptr_w = abi.width as usize;
    let Ok(ptrs_raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), strings_ptr, (count as usize) * ptr_w)) else {
        return Reply::Value(0);
    };
    let lens_raw = if lengths_ptr != 0 {
        task.mem(cx, false, |t| user::read_vec(&t.space(), lengths_ptr, (count as usize) * 4)).ok()
    } else {
        None
    };
    let mut source = Vec::new();
    for i in 0..count as usize {
        let s_ptr = abi.ptr_at(&ptrs_raw, i * ptr_w);
        if s_ptr == 0 {
            continue;
        }
        let len = lens_raw
            .as_ref()
            .map(|b| i32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()))
            .unwrap_or(-1);
        if len < 0 {
            let s = read_cstr(task, cx, s_ptr);
            source.extend_from_slice(s.as_bytes());
        } else if len > 0 {
            if let Ok(chunk) = task.mem(cx, false, |t| user::read_vec(&t.space(), s_ptr, len as usize)) {
                source.extend_from_slice(&chunk);
            }
        }
    }
    let mut p = Vec::with_capacity(source.len() + 8);
    records::put_u32(&mut p, shader);
    records::put_bytes(&mut p, &source);
    let _ = send_record(task, cx, wire::SHADER_SOURCE, &p);
    Reply::Value(0)
}

fn gl_compile_shader(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::COMPILE, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_delete_shader(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::DELETE_SHADER, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_get_shaderiv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = send_words(task, cx, wire::GET_SHADER, &[a[0] as u32, a[1] as u32])
        .and_then(|b| Reader::new(&b).u32().ok())
        .unwrap_or(0) as i32;
    if a[2] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &v.to_le_bytes()));
    }
    Reply::Value(0)
}

fn gl_get_shader_info_log(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let ans = send_words(task, cx, wire::SHADER_LOG, &[a[0] as u32]).unwrap_or_default();
    let text = Reader::new(&ans).bytes().unwrap_or(&[]).to_vec();
    write_cstr_out(task, cx, &text, a[1] as u32 as i32, a[2], a[3]);
    Reply::Value(0)
}

fn gl_get_shader_precision_format(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let precision = a[1] as u32;
    let (lo, hi, bits) = if (0x8df0..=0x8df2).contains(&precision) { (127i32, 127i32, 23i32) } else { (31, 30, 0) };
    if a[2] != 0 {
        let mut b = [0u8; 8];
        b[0..4].copy_from_slice(&lo.to_le_bytes());
        b[4..8].copy_from_slice(&hi.to_le_bytes());
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b));
    }
    if a[3] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &bits.to_le_bytes()));
    }
    Reply::Value(0)
}

fn gl_shader_binary(task: &mut Task, cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    send_error(task, cx, INVALID_ENUM);
    Reply::Value(0)
}

fn gl_create_program(task: &mut Task, cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    let Some((st, _)) = current_ctx(task) else { return Reply::Value(0) };
    let name = alloc_name(&st);
    let _ = send_words(task, cx, wire::PROGRAM, &[name]);
    Reply::Value(u64::from(name))
}

fn gl_attach_shader(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::ATTACH, &[a[0] as u32, a[1] as u32]);
    Reply::Value(0)
}

fn gl_detach_shader(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::DETACH, &[a[0] as u32, a[1] as u32]);
    Reply::Value(0)
}

fn gl_bind_attrib_location(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = read_cstr(task, cx, a[2]);
    let mut p = Vec::with_capacity(name.len() + 12);
    records::put_u32(&mut p, a[0] as u32);
    records::put_u32(&mut p, a[1] as u32);
    records::put_bytes(&mut p, name.as_bytes());
    let _ = send_record(task, cx, wire::BIND_ATTRIB, &p);
    Reply::Value(0)
}

fn gl_link_program(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::LINK, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_validate_program(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::VALIDATE, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_use_program(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::USE_PROGRAM, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_delete_program(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] as u32 != 0 {
        let _ = send_words(task, cx, wire::DELETE_PROGRAM, &[a[0] as u32]);
    }
    Reply::Value(0)
}

fn gl_get_programiv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = send_words(task, cx, wire::GET_PROGRAM, &[a[0] as u32, a[1] as u32])
        .and_then(|b| Reader::new(&b).u32().ok())
        .unwrap_or(0) as i32;
    if a[2] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &v.to_le_bytes()));
    }
    Reply::Value(0)
}

fn gl_get_program_info_log(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let ans = send_words(task, cx, wire::PROGRAM_LOG, &[a[0] as u32]).unwrap_or_default();
    let text = Reader::new(&ans).bytes().unwrap_or(&[]).to_vec();
    write_cstr_out(task, cx, &text, a[1] as u32 as i32, a[2], a[3]);
    Reply::Value(0)
}

fn gl_get_uniform_location(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = read_cstr(task, cx, a[1]);
    let mut p = Vec::with_capacity(name.len() + 8);
    records::put_u32(&mut p, a[0] as u32);
    records::put_bytes(&mut p, name.as_bytes());
    let loc = send_record(task, cx, wire::UNIFORM_LOCATION, &p)
        .and_then(|b| Reader::new(&b).u32().ok())
        .unwrap_or(u32::MAX);
    Reply::Value(i64::from(loc as i32) as u64)
}

fn gl_get_attrib_location(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = read_cstr(task, cx, a[1]);
    let mut p = Vec::with_capacity(name.len() + 8);
    records::put_u32(&mut p, a[0] as u32);
    records::put_bytes(&mut p, name.as_bytes());
    let loc = send_record(task, cx, wire::ATTRIB_LOCATION, &p)
        .and_then(|b| Reader::new(&b).u32().ok())
        .unwrap_or(u32::MAX);
    Reply::Value(i64::from(loc as i32) as u64)
}

fn active_query(task: &mut Task, cx: &mut Cx<'_>, kind: u32, a: &[u64]) -> Reply {
    let ans = send_words(task, cx, kind, &[a[0] as u32, a[1] as u32]).unwrap_or_default();
    let mut r = Reader::new(&ans);
    let count = r.u32().unwrap_or(0) as i32;
    let ty = r.u32().unwrap_or(0);
    let name = r.bytes().unwrap_or(&[]).to_vec();
    if a[4] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[4], &count.to_le_bytes()));
    }
    if a[5] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[5], &ty.to_le_bytes()));
    }
    write_cstr_out(task, cx, &name, a[2] as u32 as i32, a[3], a[6]);
    Reply::Value(0)
}

fn gl_get_active_uniform(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    active_query(task, cx, wire::ACTIVE_UNIFORM, a)
}

fn gl_get_active_attrib(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    active_query(task, cx, wire::ACTIVE_ATTRIB, a)
}

fn gl_get_attached_shaders(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let ans = send_words(task, cx, wire::ATTACHED_SHADERS, &[a[0] as u32]).unwrap_or_default();
    let mut r = Reader::new(&ans);
    let n = r.u32().unwrap_or(0) as usize;
    let most = (a[1] as u32 as i32).max(0) as usize;
    let take = n.min(most);
    let mut out = Vec::with_capacity(take * 4);
    for _ in 0..take {
        out.extend_from_slice(&r.u32().unwrap_or(0).to_le_bytes());
    }
    if a[3] != 0 && !out.is_empty() {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &out));
    }
    if a[2] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &(take as i32).to_le_bytes()));
    }
    Reply::Value(0)
}

// ---- Uniforms ----

fn send_uniform_bytes(task: &mut Task, cx: &mut Cx<'_>, location: u32, kind: u32, cols: u32, rows: u32, count: u32, transpose: bool, bytes: &[u8]) {
    let mut p = Vec::with_capacity(bytes.len() + 28);
    for w in [location, kind, cols, rows, count, u32::from(transpose)] {
        records::put_u32(&mut p, w);
    }
    records::put_bytes(&mut p, bytes);
    let _ = send_record(task, cx, wire::UNIFORM, &p);
}

fn send_uniform_ptr(task: &mut Task, cx: &mut Cx<'_>, location: u32, kind: u32, cols: u32, rows: u32, count: i32, transpose: bool, ptr: u64) -> Reply {
    if ptr == 0 || count <= 0 {
        return Reply::Value(0);
    }
    let nbytes = (cols * rows) as usize * (count as usize) * 4;
    if let Ok(bytes) = task.mem(cx, false, |t| user::read_vec(&t.space(), ptr, nbytes)) {
        send_uniform_bytes(task, cx, location, kind, cols, rows, count as u32, transpose, &bytes);
    }
    Reply::Value(0)
}

fn gl_uniform1f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = task.f32_arg(cx, a, 1, 0).to_le_bytes();
    send_uniform_bytes(task, cx, a[0] as u32, wire::FLOAT, 1, 1, 1, false, &x);
    Reply::Value(0)
}
fn gl_uniform2f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut b = [0u8; 8];
    b[0..4].copy_from_slice(&task.f32_arg(cx, a, 1, 0).to_le_bytes());
    b[4..8].copy_from_slice(&task.f32_arg(cx, a, 2, 1).to_le_bytes());
    send_uniform_bytes(task, cx, a[0] as u32, wire::FLOAT, 2, 1, 1, false, &b);
    Reply::Value(0)
}
fn gl_uniform3f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut b = [0u8; 12];
    b[0..4].copy_from_slice(&task.f32_arg(cx, a, 1, 0).to_le_bytes());
    b[4..8].copy_from_slice(&task.f32_arg(cx, a, 2, 1).to_le_bytes());
    b[8..12].copy_from_slice(&task.f32_arg(cx, a, 3, 2).to_le_bytes());
    send_uniform_bytes(task, cx, a[0] as u32, wire::FLOAT, 3, 1, 1, false, &b);
    Reply::Value(0)
}
fn gl_uniform4f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&task.f32_arg(cx, a, 1, 0).to_le_bytes());
    b[4..8].copy_from_slice(&task.f32_arg(cx, a, 2, 1).to_le_bytes());
    b[8..12].copy_from_slice(&task.f32_arg(cx, a, 3, 2).to_le_bytes());
    b[12..16].copy_from_slice(&task.f32_arg(cx, a, 4, 3).to_le_bytes());
    send_uniform_bytes(task, cx, a[0] as u32, wire::FLOAT, 4, 1, 1, false, &b);
    Reply::Value(0)
}

macro_rules! uniform_int_scalar {
    ($name:ident, $kind:expr, $n:expr) => {
        fn $name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
            let mut b = Vec::with_capacity($n * 4);
            for i in 0..$n {
                b.extend_from_slice(&(a[1 + i] as u32).to_le_bytes());
            }
            send_uniform_bytes(task, cx, a[0] as u32, $kind, $n as u32, 1, 1, false, &b);
            Reply::Value(0)
        }
    };
}

uniform_int_scalar!(gl_uniform1i, wire::INT, 1);
uniform_int_scalar!(gl_uniform2i, wire::INT, 2);
uniform_int_scalar!(gl_uniform3i, wire::INT, 3);
uniform_int_scalar!(gl_uniform4i, wire::INT, 4);
uniform_int_scalar!(gl_uniform1ui, wire::UINT, 1);
uniform_int_scalar!(gl_uniform2ui, wire::UINT, 2);
uniform_int_scalar!(gl_uniform3ui, wire::UINT, 3);
uniform_int_scalar!(gl_uniform4ui, wire::UINT, 4);

macro_rules! uniform_vec {
    ($name:ident, $kind:expr, $cols:expr) => {
        fn $name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
            send_uniform_ptr(task, cx, a[0] as u32, $kind, $cols, 1, a[1] as u32 as i32, false, a[2])
        }
    };
}

uniform_vec!(gl_uniform1fv, wire::FLOAT, 1);
uniform_vec!(gl_uniform2fv, wire::FLOAT, 2);
uniform_vec!(gl_uniform3fv, wire::FLOAT, 3);
uniform_vec!(gl_uniform4fv, wire::FLOAT, 4);
uniform_vec!(gl_uniform1iv, wire::INT, 1);
uniform_vec!(gl_uniform2iv, wire::INT, 2);
uniform_vec!(gl_uniform3iv, wire::INT, 3);
uniform_vec!(gl_uniform4iv, wire::INT, 4);
uniform_vec!(gl_uniform1uiv, wire::UINT, 1);
uniform_vec!(gl_uniform2uiv, wire::UINT, 2);
uniform_vec!(gl_uniform3uiv, wire::UINT, 3);
uniform_vec!(gl_uniform4uiv, wire::UINT, 4);

macro_rules! uniform_mat {
    ($name:ident, $cols:expr, $rows:expr) => {
        fn $name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
            send_uniform_ptr(task, cx, a[0] as u32, wire::FLOAT, $cols, $rows, a[1] as u32 as i32, a[2] as u8 != 0, a[3])
        }
    };
}

uniform_mat!(gl_uniform_matrix2fv, 2, 2);
uniform_mat!(gl_uniform_matrix3fv, 3, 3);
uniform_mat!(gl_uniform_matrix4fv, 4, 4);
uniform_mat!(gl_uniform_matrix2x3fv, 2, 3);
uniform_mat!(gl_uniform_matrix3x2fv, 3, 2);
uniform_mat!(gl_uniform_matrix2x4fv, 2, 4);
uniform_mat!(gl_uniform_matrix4x2fv, 4, 2);
uniform_mat!(gl_uniform_matrix3x4fv, 3, 4);
uniform_mat!(gl_uniform_matrix4x3fv, 4, 3);

// ---- Buffers and vertex arrays ----

fn gl_delete_buffers(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_each(task, cx, a[0] as u32 as i32, a[1], wire::DELETE_BUFFER);
    Reply::Value(0)
}

fn gl_bind_buffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (target, name) = (a[0] as u32, a[1] as u32);
    if target == 0x8892
        && let Some((st, hglrc)) = current_ctx(task)
        && let Some(c) = lock(&st.inner).contexts.get_mut(&hglrc)
    {
        c.bound_array_buffer = name;
    }
    let _ = send_words(task, cx, wire::BIND_BUFFER, &[target, name]);
    Reply::Value(0)
}

fn gl_buffer_data(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (target, size, data_ptr, usage) = (a[0] as u32, (a[1] as i64).max(0) as usize, a[2], a[3] as u32);
    let bytes = if data_ptr == 0 || size == 0 {
        Vec::new()
    } else {
        task.mem(cx, false, |t| user::read_vec(&t.space(), data_ptr, size)).unwrap_or_default()
    };
    let mut p = Vec::with_capacity(bytes.len() + 24);
    records::put_u32(&mut p, target);
    records::put_u64(&mut p, size as u64);
    records::put_u32(&mut p, usage);
    records::put_u32(&mut p, u32::from(data_ptr != 0));
    records::put_bytes(&mut p, &bytes);
    let _ = send_record(task, cx, wire::BUFFER_DATA, &p);
    Reply::Value(0)
}

fn gl_buffer_sub_data(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (target, offset, size, data_ptr) = (a[0] as u32, (a[1] as i64).max(0) as u64, (a[2] as i64).max(0) as usize, a[3]);
    if data_ptr == 0 || size == 0 {
        return Reply::Value(0);
    }
    let Ok(bytes) = task.mem(cx, false, |t| user::read_vec(&t.space(), data_ptr, size)) else { return Reply::Value(0) };
    let mut p = Vec::with_capacity(bytes.len() + 16);
    records::put_u32(&mut p, target);
    records::put_u64(&mut p, offset);
    records::put_bytes(&mut p, &bytes);
    let _ = send_record(task, cx, wire::BUFFER_SUB_DATA, &p);
    Reply::Value(0)
}

fn buffer_param(task: &mut Task, cx: &mut Cx<'_>, target: u32, pname: u32) -> i64 {
    let Some((st, hglrc)) = current_ctx(task) else { return 0 };
    if let Some(val) = lock(&st.inner).contexts.get(&hglrc).and_then(|c| {
        let m = c.maps.iter().find(|m| m.target == target);
        match pname {
            0x88bc => Some(i64::from(m.is_some())),
            0x911f => Some(m.map_or(0, |m| i64::from(m.access))),
            0x9120 => Some(m.map_or(0, |m| m.length as i64)),
            0x9121 => Some(m.map_or(0, |m| m.offset as i64)),
            _ => None,
        }
    }) {
        return val;
    }
    send_words(task, cx, wire::GET_BUFFER_PARAMETER, &[target, pname])
        .and_then(|b| Reader::new(&b).u64().ok())
        .unwrap_or(0) as i64
}

fn gl_map_buffer_range(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, hglrc)) = current_ctx(task) else { return Reply::Value(0) };
    let (target, offset, length, access) = (a[0] as u32, a[1] as i64, a[2] as i64, a[3] as u32);
    if length <= 0 || offset < 0 {
        send_error(task, cx, INVALID_VALUE);
        return Reply::Value(0);
    }
    if lock(&st.inner).contexts.get(&hglrc).is_some_and(|c| c.maps.iter().any(|m| m.target == target)) {
        send_error(task, cx, INVALID_OPERATION);
        return Reply::Value(0);
    }
    let Some(guest_addr) = task.virtual_alloc(cx, 0, length as u64, 0x3000, 0x04) else { return Reply::Value(0) };
    const MAP_READ: u32 = 0x1;
    const MAP_INVALIDATE_RANGE: u32 = 0x4;
    const MAP_INVALIDATE_BUFFER: u32 = 0x8;
    if access & MAP_READ != 0 || access & (MAP_INVALIDATE_RANGE | MAP_INVALIDATE_BUFFER) == 0 {
        let mut p = Vec::with_capacity(28);
        records::put_u32(&mut p, target);
        records::put_u64(&mut p, offset as u64);
        records::put_u64(&mut p, length as u64);
        records::put_u64(&mut p, guest_addr);
        let _ = send_record(task, cx, wire::READ_BUFFER, &p);
    }
    if let Some(c) = lock(&st.inner).contexts.get_mut(&hglrc) {
        c.maps.push(BufferMap { target, offset: offset as u64, length: length as usize, guest_addr, access });
    }
    Reply::Value(guest_addr)
}

fn gl_map_buffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (target, access) = (a[0] as u32, a[1] as u32);
    let size = buffer_param(task, cx, target, 0x8764);
    let bits: u32 = match access {
        0x88b8 => 0x1,
        0x88b9 => 0x2,
        _ => 0x3,
    };
    gl_map_buffer_range(task, cx, &[u64::from(target), 0, size as u64, u64::from(bits)])
}

fn gl_flush_mapped_buffer_range(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, hglrc)) = current_ctx(task) else { return Reply::Value(0) };
    let (target, offset, length) = (a[0] as u32, (a[1] as i64).max(0) as usize, (a[2] as i64).max(0) as usize);
    let found = lock(&st.inner).contexts.get(&hglrc).and_then(|c| {
        c.maps.iter().find(|m| m.target == target).map(|m| (m.offset, m.length, m.guest_addr))
    });
    let Some((base_off, map_len, guest_addr)) = found else { return Reply::Value(0) };
    if offset + length > map_len || length == 0 {
        return Reply::Value(0);
    }
    let Ok(bytes) = task.mem(cx, false, |t| user::read_vec(&t.space(), guest_addr + offset as u64, length)) else {
        return Reply::Value(0);
    };
    let mut p = Vec::with_capacity(bytes.len() + 16);
    records::put_u32(&mut p, target);
    records::put_u64(&mut p, base_off + offset as u64);
    records::put_bytes(&mut p, &bytes);
    let _ = send_record(task, cx, wire::BUFFER_SUB_DATA, &p);
    Reply::Value(0)
}

fn gl_unmap_buffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, hglrc)) = current_ctx(task) else { return Reply::Value(0) };
    let target = a[0] as u32;
    let removed = lock(&st.inner).contexts.get_mut(&hglrc).and_then(|c| {
        let i = c.maps.iter().position(|m| m.target == target)?;
        Some(c.maps.remove(i))
    });
    let Some(m) = removed else { return Reply::Value(0) };
    if m.access & 0x2 != 0 && m.access & 0x10 == 0 {
        if let Ok(bytes) = task.mem(cx, false, |t| user::read_vec(&t.space(), m.guest_addr, m.length)) {
            let mut p = Vec::with_capacity(bytes.len() + 16);
            records::put_u32(&mut p, target);
            records::put_u64(&mut p, m.offset);
            records::put_bytes(&mut p, &bytes);
            let _ = send_record(task, cx, wire::BUFFER_SUB_DATA, &p);
        }
    }
    let _ = task.virtual_free(cx, m.guest_addr, 0, 0x8000);
    Reply::Value(1)
}

fn gl_get_buffer_pointerv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((st, hglrc)) = current_ctx(task) else { return Reply::Value(0) };
    let addr = lock(&st.inner)
        .contexts
        .get(&hglrc)
        .and_then(|c| c.maps.iter().find(|m| m.target == a[0] as u32).map(|m| m.guest_addr))
        .unwrap_or(0);
    if a[2] != 0 {
        let _ = task.write_ptr(cx, a[2], addr);
    }
    Reply::Value(0)
}

fn gl_get_buffer_parameteriv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = buffer_param(task, cx, a[0] as u32, a[1] as u32) as i32;
    if a[2] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &v.to_le_bytes()));
    }
    Reply::Value(0)
}

fn gl_get_buffer_parameteri64v(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = buffer_param(task, cx, a[0] as u32, a[1] as u32);
    if a[2] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &v.to_le_bytes()));
    }
    Reply::Value(0)
}

fn bind_buffer_range_impl(task: &mut Task, cx: &mut Cx<'_>, target: u32, index: u32, buffer: u32, offset: u64, size: u64) {
    let mut p = Vec::with_capacity(28);
    for w in [target, index, buffer] {
        records::put_u32(&mut p, w);
    }
    records::put_u64(&mut p, offset);
    records::put_u64(&mut p, size);
    let _ = send_record(task, cx, wire::BIND_BUFFER_RANGE, &p);
}

fn gl_bind_buffer_base(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    bind_buffer_range_impl(task, cx, a[0] as u32, a[1] as u32, a[2] as u32, 0, 0);
    Reply::Value(0)
}

fn gl_bind_buffer_range(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    bind_buffer_range_impl(task, cx, a[0] as u32, a[1] as u32, a[2] as u32, (a[3] as i64).max(0) as u64, (a[4] as i64).max(0) as u64);
    Reply::Value(0)
}

fn gl_delete_vertex_arrays(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_each(task, cx, a[0] as u32 as i32, a[1], wire::DELETE_VERTEX_ARRAY);
    Reply::Value(0)
}

fn gl_bind_vertex_array(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = a[0] as u32;
    if let Some((st, hglrc)) = current_ctx(task)
        && let Some(c) = lock(&st.inner).contexts.get_mut(&hglrc)
    {
        c.bound_vao = name;
    }
    let _ = send_words(task, cx, wire::BIND_VERTEX_ARRAY, &[name]);
    Reply::Value(0)
}

fn send_attrib_pointer(task: &mut Task, cx: &mut Cx<'_>, index: u32, size: u32, kind: u32, normalized: bool, integer: bool, stride: i32, at: u64) {
    let mut p = Vec::with_capacity(32);
    for w in [index, size, kind, u32::from(normalized), u32::from(integer), stride.max(0) as u32] {
        records::put_u32(&mut p, w);
    }
    records::put_u64(&mut p, at);
    let _ = send_record(task, cx, wire::ATTRIB_POINTER, &p);
}

fn gl_vertex_attrib_pointer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_attrib_pointer(task, cx, a[0] as u32, a[1] as u32, a[2] as u32, a[3] as u8 != 0, false, a[4] as u32 as i32, a[5]);
    Reply::Value(0)
}

fn gl_vertex_attrib_i_pointer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_attrib_pointer(task, cx, a[0] as u32, a[1] as u32, a[2] as u32, false, true, a[3] as u32 as i32, a[4]);
    Reply::Value(0)
}

fn gl_enable_vertex_attrib_array(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::ENABLE_ATTRIB, &[a[0] as u32, 1]);
    Reply::Value(0)
}

fn gl_disable_vertex_attrib_array(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::ENABLE_ATTRIB, &[a[0] as u32, 0]);
    Reply::Value(0)
}

fn gl_vertex_attrib_divisor(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::ATTRIB_DIVISOR, &[a[0] as u32, a[1] as u32]);
    Reply::Value(0)
}

fn send_attrib_f32(task: &mut Task, cx: &mut Cx<'_>, index: u32, v: [f32; 4]) {
    let _ = send_words(task, cx, wire::ATTRIB_VALUE, &[index, v[0].to_bits(), v[1].to_bits(), v[2].to_bits(), v[3].to_bits()]);
}

fn gl_vertex_attrib1f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 1, 0), 0.0, 0.0, 1.0];
    send_attrib_f32(task, cx, a[0] as u32, v);
    Reply::Value(0)
}
fn gl_vertex_attrib2f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 1, 0), task.f32_arg(cx, a, 2, 1), 0.0, 1.0];
    send_attrib_f32(task, cx, a[0] as u32, v);
    Reply::Value(0)
}
fn gl_vertex_attrib3f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 1, 0), task.f32_arg(cx, a, 2, 1), task.f32_arg(cx, a, 3, 2), 1.0];
    send_attrib_f32(task, cx, a[0] as u32, v);
    Reply::Value(0)
}
fn gl_vertex_attrib4f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 1, 0), task.f32_arg(cx, a, 2, 1), task.f32_arg(cx, a, 3, 2), task.f32_arg(cx, a, 4, 3)];
    send_attrib_f32(task, cx, a[0] as u32, v);
    Reply::Value(0)
}

fn read_f32s(task: &mut Task, cx: &mut Cx<'_>, ptr: u64, n: usize) -> Option<Vec<f32>> {
    if ptr == 0 {
        return None;
    }
    let raw = task.mem(cx, false, |t| user::read_vec(&t.space(), ptr, n * 4)).ok()?;
    Some(raw.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
}

fn gl_vertex_attrib1fv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(v) = read_f32s(task, cx, a[1], 1) {
        send_attrib_f32(task, cx, a[0] as u32, [v[0], 0.0, 0.0, 1.0]);
    }
    Reply::Value(0)
}
fn gl_vertex_attrib2fv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(v) = read_f32s(task, cx, a[1], 2) {
        send_attrib_f32(task, cx, a[0] as u32, [v[0], v[1], 0.0, 1.0]);
    }
    Reply::Value(0)
}
fn gl_vertex_attrib3fv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(v) = read_f32s(task, cx, a[1], 3) {
        send_attrib_f32(task, cx, a[0] as u32, [v[0], v[1], v[2], 1.0]);
    }
    Reply::Value(0)
}
fn gl_vertex_attrib4fv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(v) = read_f32s(task, cx, a[1], 4) {
        send_attrib_f32(task, cx, a[0] as u32, [v[0], v[1], v[2], v[3]]);
    }
    Reply::Value(0)
}
fn gl_vertex_attrib_i4i(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::ATTRIB_VALUE, &[a[0] as u32, a[1] as u32, a[2] as u32, a[3] as u32, a[4] as u32]);
    Reply::Value(0)
}
fn gl_vertex_attrib_i4ui(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    gl_vertex_attrib_i4i(task, cx, a)
}
fn gl_vertex_attrib_i4iv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] != 0
        && let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], 16))
    {
        let w: [u32; 4] = std::array::from_fn(|i| u32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap()));
        let _ = send_words(task, cx, wire::ATTRIB_VALUE, &[a[0] as u32, w[0], w[1], w[2], w[3]]);
    }
    Reply::Value(0)
}
fn gl_vertex_attrib_i4uiv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    gl_vertex_attrib_i4iv(task, cx, a)
}

// ---- Textures ----

fn gl_delete_textures(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_each(task, cx, a[0] as u32 as i32, a[1], wire::DELETE_TEXTURE);
    Reply::Value(0)
}

fn gl_bind_texture(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::BIND_TEXTURE, &[a[0] as u32, a[1] as u32]);
    Reply::Value(0)
}

fn gl_active_texture(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::ACTIVE_TEXTURE, &[(a[0] as u32).wrapping_sub(0x84c0)]);
    Reply::Value(0)
}

fn pixel_bytes(format: u32, kind: u32) -> usize {
    let comps = match format {
        0x1903 | 0x1906 | 0x1909 | 0x1902 => 1,
        0x8227 | 0x190a => 2,
        0x1907 | 0x80e0 => 3,
        _ => 4,
    };
    match kind {
        0x1401 | 0x1400 => comps,
        0x1402 | 0x1403 | 0x140b => comps * 2,
        0x8363 | 0x8033 | 0x8034 | 0x8365 | 0x8366 => 2,
        0x8035 | 0x8367 | 0x8036 | 0x8368 | 0x8c3b | 0x84fa => 4,
        _ => comps * 4,
    }
}

fn unpack_texels(task: &mut Task, cx: &mut Cx<'_>, width: i32, height: i32, format: u32, kind: u32, pixels_ptr: u64) -> Vec<u8> {
    let Some((st, hglrc)) = current_ctx(task) else { return Vec::new() };
    let (w, h) = (width.max(0) as usize, height.max(0) as usize);
    if w == 0 || h == 0 || pixels_ptr == 0 {
        return Vec::new();
    }
    let (row_len, alignment, skip_rows, skip_pixels) = {
        let inner = lock(&st.inner);
        let Some(c) = inner.contexts.get(&hglrc) else { return Vec::new() };
        (
            c.store.get(0x0cf2).unwrap_or(0).max(0) as usize,
            c.store.get(0x0cf5).unwrap_or(4).max(1) as usize,
            c.store.get(0x0cf3).unwrap_or(0).max(0) as usize,
            c.store.get(0x0cf4).unwrap_or(0).max(0) as usize,
        )
    };
    let size = pixel_bytes(format, kind);
    let row = w * size;
    let stride = (if row_len > 0 { row_len } else { w } * size).next_multiple_of(alignment);
    let start = skip_rows * stride + skip_pixels * size;
    if stride == row {
        return task.mem(cx, false, |t| user::read_vec(&t.space(), pixels_ptr + start as u64, row * h)).unwrap_or_default();
    }
    let total_src = (h - 1) * stride + row;
    let Ok(src) = task.mem(cx, false, |t| user::read_vec(&t.space(), pixels_ptr + start as u64, total_src)) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(row * h);
    for y in 0..h {
        let s = y * stride;
        out.extend_from_slice(&src[s..s + row]);
    }
    out
}

fn gl_tex_image_2d(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (target, level, internal, width, height, format, kind, pixels_ptr) = (
        a[0] as u32, a[1] as u32 as i32, a[2] as u32 as i32, a[3] as u32 as i32, a[4] as u32 as i32, a[6] as u32, a[7] as u32, a[8],
    );
    let texels = if pixels_ptr == 0 { Vec::new() } else { unpack_texels(task, cx, width, height, format, kind, pixels_ptr) };
    let mut p = Vec::with_capacity(texels.len() + 36);
    for w in [target, level as u32, internal as u32, width as u32, height as u32, format, kind, u32::from(pixels_ptr != 0)] {
        records::put_u32(&mut p, w);
    }
    records::put_bytes(&mut p, &texels);
    let _ = send_record(task, cx, wire::TEX_IMAGE, &p);
    Reply::Value(0)
}

fn gl_tex_sub_image_2d(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (target, level, x, y, width, height, format, kind, pixels_ptr) = (
        a[0] as u32, a[1] as u32, a[2] as u32, a[3] as u32, a[4] as u32 as i32, a[5] as u32 as i32, a[6] as u32, a[7] as u32, a[8],
    );
    if pixels_ptr == 0 {
        return Reply::Value(0);
    }
    let texels = unpack_texels(task, cx, width, height, format, kind, pixels_ptr);
    let mut p = Vec::with_capacity(texels.len() + 36);
    for w in [target, level, x, y, width as u32, height as u32, format, kind] {
        records::put_u32(&mut p, w);
    }
    records::put_bytes(&mut p, &texels);
    let _ = send_record(task, cx, wire::TEX_SUB_IMAGE, &p);
    Reply::Value(0)
}

fn gl_tex_storage_2d(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (target, levels, internal, width, height) = (a[0] as u32, (a[1] as u32 as i32).max(1), a[2] as u32, a[3] as u32 as i32, a[4] as u32 as i32);
    let (format, kind): (u32, u32) = match internal {
        0x8058 | 0x8c43 | 0x8059 | 0x8056 | 0x8057 => (0x1908, 0x1401),
        0x8051 | 0x8c41 | 0x8d62 => (0x1907, 0x1401),
        0x8229 => (0x1903, 0x1401),
        0x822b => (0x8227, 0x1401),
        0x881a | 0x881b | 0x822d | 0x822f | 0x8c3a => (0x1908, 0x140b),
        0x8814 | 0x8815 | 0x822e | 0x8230 => (0x1908, 0x1406),
        0x81a5 => (0x1902, 0x1403),
        0x81a6 | 0x81a7 => (0x1902, 0x1405),
        0x8cac => (0x1902, 0x1406),
        0x88f0 => (0x84f9, 0x84fa),
        0x8cad => (0x84f9, 0x8dad),
        _ => (0x1908, 0x1401),
    };
    for level in 0..levels {
        let w = (width >> level).max(1) as u32;
        let h = (height >> level).max(1) as u32;
        let mut p = Vec::with_capacity(36);
        for word in [target, level as u32, internal, w, h, format, kind, 0] {
            records::put_u32(&mut p, word);
        }
        records::put_bytes(&mut p, &[]);
        let _ = send_record(task, cx, wire::TEX_IMAGE, &p);
    }
    Reply::Value(0)
}

fn gl_tex_parameteri(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::TEX_PARAMETER, &[a[0] as u32, a[1] as u32, a[2] as u32, 0]);
    Reply::Value(0)
}

fn gl_tex_parameterf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = task.f32_arg(cx, a, 2, 0).to_bits();
    let _ = send_words(task, cx, wire::TEX_PARAMETER, &[a[0] as u32, a[1] as u32, v, 1]);
    Reply::Value(0)
}

fn gl_tex_parameteriv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[2] != 0
        && let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[2], 4))
    {
        let v = u32::from_le_bytes(raw.try_into().unwrap());
        let _ = send_words(task, cx, wire::TEX_PARAMETER, &[a[0] as u32, a[1] as u32, v, 0]);
    }
    Reply::Value(0)
}

fn gl_tex_parameterfv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[2] != 0
        && let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[2], 4))
    {
        let v = u32::from_le_bytes(raw.try_into().unwrap());
        let _ = send_words(task, cx, wire::TEX_PARAMETER, &[a[0] as u32, a[1] as u32, v, 1]);
    }
    Reply::Value(0)
}

fn gl_generate_mipmap(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::GENERATE_MIPMAP, &[a[0] as u32]);
    Reply::Value(0)
}

// ---- Framebuffer objects, renderbuffers, uniform blocks, samplers ----

fn gl_delete_framebuffers(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_each(task, cx, a[0] as u32 as i32, a[1], wire::DELETE_FRAMEBUFFER);
    Reply::Value(0)
}

fn gl_bind_framebuffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::BIND_FRAMEBUFFER, &[a[0] as u32, a[1] as u32]);
    Reply::Value(0)
}

fn gl_framebuffer_texture_2d(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::FRAMEBUFFER_TEXTURE, &[a[0] as u32, a[1] as u32, a[2] as u32, a[3] as u32, (a[4] as u32 as i32).max(0) as u32]);
    Reply::Value(0)
}

fn gl_framebuffer_texture(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::FRAMEBUFFER_TEXTURE, &[a[0] as u32, a[1] as u32, 0x0de1, a[2] as u32, (a[3] as u32 as i32).max(0) as u32]);
    Reply::Value(0)
}

fn gl_framebuffer_renderbuffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::FRAMEBUFFER_RENDERBUFFER, &[a[0] as u32, a[1] as u32, a[2] as u32, a[3] as u32]);
    Reply::Value(0)
}

fn gl_check_framebuffer_status(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = send_words(task, cx, wire::CHECK_FRAMEBUFFER, &[a[0] as u32])
        .and_then(|b| Reader::new(&b).u32().ok())
        .unwrap_or(0);
    Reply::Value(u64::from(v))
}

fn gl_delete_renderbuffers(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_each(task, cx, a[0] as u32 as i32, a[1], wire::DELETE_RENDERBUFFER);
    Reply::Value(0)
}

fn gl_bind_renderbuffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::BIND_RENDERBUFFER, &[a[0] as u32, a[1] as u32]);
    Reply::Value(0)
}

fn gl_renderbuffer_storage(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::RENDERBUFFER_STORAGE, &[a[0] as u32, a[1] as u32, (a[2] as u32 as i32).max(0) as u32, (a[3] as u32 as i32).max(0) as u32]);
    Reply::Value(0)
}

fn gl_renderbuffer_storage_multisample(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    gl_renderbuffer_storage(task, cx, &[a[0], a[2], a[3], a[4]])
}

fn gl_blit_framebuffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(
        task,
        cx,
        wire::BLIT_FRAMEBUFFER,
        &[a[0] as u32, a[1] as u32, a[2] as u32, a[3] as u32, a[4] as u32, a[5] as u32, a[6] as u32, a[7] as u32, a[8] as u32, a[9] as u32],
    );
    Reply::Value(0)
}

fn gl_draw_buffers(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let n = (a[0] as u32 as i32).max(0) as usize;
    let mut words = Vec::with_capacity(n + 1);
    words.push(n as u32);
    if n > 0 && a[1] != 0
        && let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], n * 4))
    {
        for c in raw.chunks_exact(4) {
            words.push(u32::from_le_bytes(c.try_into().unwrap()));
        }
    }
    let _ = send_words(task, cx, wire::DRAW_BUFFERS, &words);
    Reply::Value(0)
}

fn gl_get_uniform_block_index(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = read_cstr(task, cx, a[1]);
    let mut p = Vec::with_capacity(name.len() + 8);
    records::put_u32(&mut p, a[0] as u32);
    records::put_bytes(&mut p, name.as_bytes());
    let v = send_record(task, cx, wire::UNIFORM_BLOCK_INDEX, &p)
        .and_then(|b| Reader::new(&b).u32().ok())
        .unwrap_or(u32::MAX);
    Reply::Value(u64::from(v))
}

fn gl_uniform_block_binding(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::UNIFORM_BLOCK_BINDING, &[a[0] as u32, a[1] as u32, a[2] as u32]);
    Reply::Value(0)
}

fn gl_get_active_uniform_blockiv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[2] as u32 == 0x8a43 {
        return Reply::Value(0);
    }
    let v = send_words(task, cx, wire::ACTIVE_UNIFORM_BLOCK, &[a[0] as u32, a[1] as u32, a[2] as u32])
        .and_then(|b| Reader::new(&b).u32().ok())
        .unwrap_or(0) as i32;
    if a[3] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &v.to_le_bytes()));
    }
    Reply::Value(0)
}

fn gl_get_active_uniform_block_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let ans = send_words(task, cx, wire::ACTIVE_UNIFORM_BLOCK_NAME, &[a[0] as u32, a[1] as u32]).unwrap_or_default();
    let text = Reader::new(&ans).bytes().unwrap_or(&[]).to_vec();
    write_cstr_out(task, cx, &text, a[2] as u32 as i32, a[3], a[4]);
    Reply::Value(0)
}

fn gl_delete_samplers(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    delete_each(task, cx, a[0] as u32 as i32, a[1], wire::DELETE_SAMPLER);
    Reply::Value(0)
}

fn gl_bind_sampler(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::BIND_SAMPLER, &[a[0] as u32, a[1] as u32]);
    Reply::Value(0)
}

fn gl_sampler_parameteri(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::SAMPLER_PARAMETER, &[a[0] as u32, a[1] as u32, a[2] as u32, 0]);
    Reply::Value(0)
}

fn gl_sampler_parameterf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = task.f32_arg(cx, a, 2, 0).to_bits();
    let _ = send_words(task, cx, wire::SAMPLER_PARAMETER, &[a[0] as u32, a[1] as u32, v, 1]);
    Reply::Value(0)
}

fn gl_sampler_parameteriv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[2] != 0
        && let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[2], 4))
    {
        let _ = send_words(task, cx, wire::SAMPLER_PARAMETER, &[a[0] as u32, a[1] as u32, u32::from_le_bytes(raw.try_into().unwrap()), 0]);
    }
    Reply::Value(0)
}

fn gl_sampler_parameterfv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[2] != 0
        && let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[2], 4))
    {
        let _ = send_words(task, cx, wire::SAMPLER_PARAMETER, &[a[0] as u32, a[1] as u32, u32::from_le_bytes(raw.try_into().unwrap()), 1]);
    }
    Reply::Value(0)
}

// ---- Blend, depth, cull, sync, queries ----

fn gl_blend_func(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (s, d) = (a[0] as u32, a[1] as u32);
    let _ = send_words(task, cx, wire::BLEND_FUNC, &[s, d, s, d]);
    Reply::Value(0)
}

fn gl_blend_func_separate(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::BLEND_FUNC, &[a[0] as u32, a[1] as u32, a[2] as u32, a[3] as u32]);
    Reply::Value(0)
}

fn gl_blend_equation(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let m = a[0] as u32;
    let _ = send_words(task, cx, wire::BLEND_EQUATION, &[m, m]);
    Reply::Value(0)
}

fn gl_blend_equation_separate(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::BLEND_EQUATION, &[a[0] as u32, a[1] as u32]);
    Reply::Value(0)
}

fn gl_depth_func(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::DEPTH_FUNC, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_cull_face(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::CULL_FACE, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_front_face(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::FRONT_FACE, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_fence_sync(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

fn gl_client_wait_sync(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    gl_finish(task, cx, a);
    Reply::Value(0x911a) // GL_ALREADY_SIGNALED
}

fn gl_is_sync(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(a[0] != 0))
}

fn gl_get_synciv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v: i32 = match a[1] as u32 {
        0x9112 => 0x9116,
        0x9114 => 0x9119,
        0x9113 => 0x9117,
        _ => 0,
    };
    if (a[2] as u32 as i32) > 0 && a[4] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[4], &v.to_le_bytes()));
    }
    if a[3] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &1i32.to_le_bytes()));
    }
    Reply::Value(0)
}

fn gl_get_internalformativ(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[2] as u32 == 0x9380 && (a[3] as u32 as i32) > 0 && a[4] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[4], &0i32.to_le_bytes()));
    }
    Reply::Value(0)
}

// ---- Draws ----

fn gl_draw_arrays(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::DRAW_ARRAYS, &[a[0] as u32, a[1] as u32, (a[2] as u32 as i32).max(0) as u32, 1]);
    Reply::Value(0)
}

fn gl_draw_arrays_instanced(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(
        task,
        cx,
        wire::DRAW_ARRAYS,
        &[a[0] as u32, a[1] as u32, (a[2] as u32 as i32).max(0) as u32, (a[3] as u32 as i32).max(0) as u32],
    );
    Reply::Value(0)
}

fn send_draw_elements(task: &mut Task, cx: &mut Cx<'_>, mode: u32, count: i32, kind: u32, instances: i32, base: i32, at: u64) {
    let mut p = Vec::with_capacity(28);
    for w in [mode, count.max(0) as u32, kind, instances.max(0) as u32, base as u32] {
        records::put_u32(&mut p, w);
    }
    records::put_u64(&mut p, at);
    let _ = send_record(task, cx, wire::DRAW_ELEMENTS, &p);
}

fn gl_draw_elements(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_draw_elements(task, cx, a[0] as u32, a[1] as u32 as i32, a[2] as u32, 1, 0, a[3]);
    Reply::Value(0)
}

fn gl_draw_elements_instanced(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_draw_elements(task, cx, a[0] as u32, a[1] as u32 as i32, a[2] as u32, a[4] as u32 as i32, 0, a[3]);
    Reply::Value(0)
}

fn gl_draw_elements_base_vertex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_draw_elements(task, cx, a[0] as u32, a[1] as u32 as i32, a[2] as u32, 1, a[4] as u32 as i32, a[3]);
    Reply::Value(0)
}

fn gl_draw_range_elements(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_draw_elements(task, cx, a[0] as u32, a[3] as u32 as i32, a[4] as u32, 1, 0, a[5]);
    Reply::Value(0)
}

// ---- Compatibility profile: matrices, client arrays, fog, texenv, alpha test, immediate mode ----

fn send_matrix(task: &mut Task, cx: &mut Cx<'_>, op: u32, floats: &[f32]) {
    let mut p = Vec::with_capacity(floats.len() * 4 + 8);
    records::put_u32(&mut p, op);
    let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
    records::put_bytes(&mut p, &bytes);
    let _ = send_record(task, cx, wire::MATRIX, &p);
}

fn read_mat16_f32(task: &mut Task, cx: &mut Cx<'_>, ptr: u64, transpose: bool) -> [f32; 16] {
    let raw = read_f32s(task, cx, ptr, 16).unwrap_or_else(|| vec![0.0; 16]);
    std::array::from_fn(|i| raw[if transpose { (i % 4) * 4 + i / 4 } else { i }])
}

fn read_mat16_f64(task: &mut Task, cx: &mut Cx<'_>, ptr: u64) -> [f32; 16] {
    if ptr == 0 {
        return [0.0; 16];
    }
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), ptr, 128)) else { return [0.0; 16] };
    std::array::from_fn(|i| f64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap()) as f32)
}

fn gl_matrix_mode(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::MATRIX_MODE, &[a[0] as u32]);
    Reply::Value(0)
}

fn gl_load_identity(task: &mut Task, cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    send_matrix(task, cx, wire::MATRIX_IDENTITY, &[]);
    Reply::Value(0)
}

fn gl_load_matrixf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let m = read_mat16_f32(task, cx, a[0], false);
    send_matrix(task, cx, wire::MATRIX_LOAD, &m);
    Reply::Value(0)
}

fn gl_load_matrixd(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let m = read_mat16_f64(task, cx, a[0]);
    send_matrix(task, cx, wire::MATRIX_LOAD, &m);
    Reply::Value(0)
}

fn gl_mult_matrixf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let m = read_mat16_f32(task, cx, a[0], false);
    send_matrix(task, cx, wire::MATRIX_MULTIPLY, &m);
    Reply::Value(0)
}

fn gl_mult_matrixd(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let m = read_mat16_f64(task, cx, a[0]);
    send_matrix(task, cx, wire::MATRIX_MULTIPLY, &m);
    Reply::Value(0)
}

fn gl_load_transpose_matrixf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let m = read_mat16_f32(task, cx, a[0], true);
    send_matrix(task, cx, wire::MATRIX_LOAD, &m);
    Reply::Value(0)
}

fn gl_mult_transpose_matrixf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let m = read_mat16_f32(task, cx, a[0], true);
    send_matrix(task, cx, wire::MATRIX_MULTIPLY, &m);
    Reply::Value(0)
}

fn gl_push_matrix(task: &mut Task, cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    send_matrix(task, cx, wire::MATRIX_PUSH, &[]);
    Reply::Value(0)
}

fn gl_pop_matrix(task: &mut Task, cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    send_matrix(task, cx, wire::MATRIX_POP, &[]);
    Reply::Value(0)
}

fn gl_translatef(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 0, 0), task.f32_arg(cx, a, 1, 1), task.f32_arg(cx, a, 2, 2)];
    send_matrix(task, cx, wire::MATRIX_TRANSLATE, &v);
    Reply::Value(0)
}

fn gl_translated(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [f64_arg(task, cx, a, 0, 0) as f32, f64_arg(task, cx, a, 1, 1) as f32, f64_arg(task, cx, a, 2, 2) as f32];
    send_matrix(task, cx, wire::MATRIX_TRANSLATE, &v);
    Reply::Value(0)
}

fn gl_scalef(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 0, 0), task.f32_arg(cx, a, 1, 1), task.f32_arg(cx, a, 2, 2)];
    send_matrix(task, cx, wire::MATRIX_SCALE, &v);
    Reply::Value(0)
}

fn gl_scaled(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [f64_arg(task, cx, a, 0, 0) as f32, f64_arg(task, cx, a, 1, 1) as f32, f64_arg(task, cx, a, 2, 2) as f32];
    send_matrix(task, cx, wire::MATRIX_SCALE, &v);
    Reply::Value(0)
}

fn gl_rotatef(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 0, 0), task.f32_arg(cx, a, 1, 1), task.f32_arg(cx, a, 2, 2), task.f32_arg(cx, a, 3, 3)];
    send_matrix(task, cx, wire::MATRIX_ROTATE, &v);
    Reply::Value(0)
}

fn gl_rotated(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [
        f64_arg(task, cx, a, 0, 0) as f32,
        f64_arg(task, cx, a, 1, 1) as f32,
        f64_arg(task, cx, a, 2, 2) as f32,
        f64_arg(task, cx, a, 3, 3) as f32,
    ];
    send_matrix(task, cx, wire::MATRIX_ROTATE, &v);
    Reply::Value(0)
}

fn gl_ortho(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [
        f64_arg(task, cx, a, 0, 0) as f32,
        f64_arg(task, cx, a, 1, 1) as f32,
        f64_arg(task, cx, a, 2, 2) as f32,
        f64_arg(task, cx, a, 3, 3) as f32,
        f64_arg(task, cx, a, 4, 4) as f32,
        f64_arg(task, cx, a, 5, 5) as f32,
    ];
    send_matrix(task, cx, wire::MATRIX_ORTHO, &v);
    Reply::Value(0)
}

fn gl_frustum(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [
        f64_arg(task, cx, a, 0, 0) as f32,
        f64_arg(task, cx, a, 1, 1) as f32,
        f64_arg(task, cx, a, 2, 2) as f32,
        f64_arg(task, cx, a, 3, 3) as f32,
        f64_arg(task, cx, a, 4, 4) as f32,
        f64_arg(task, cx, a, 5, 5) as f32,
    ];
    send_matrix(task, cx, wire::MATRIX_FRUSTUM, &v);
    Reply::Value(0)
}

fn gl_vertex_pointer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_attrib_pointer(task, cx, 0, a[0] as u32, a[1] as u32, false, false, a[2] as u32 as i32, a[3]);
    Reply::Value(0)
}

fn gl_normal_pointer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let kind = a[0] as u32;
    send_attrib_pointer(task, cx, 2, 3, kind, kind != FLOAT, false, a[1] as u32 as i32, a[2]);
    Reply::Value(0)
}

fn gl_color_pointer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let kind = a[1] as u32;
    send_attrib_pointer(task, cx, 3, a[0] as u32, kind, kind != FLOAT, false, a[2] as u32 as i32, a[3]);
    Reply::Value(0)
}

fn gl_secondary_color_pointer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let kind = a[1] as u32;
    send_attrib_pointer(task, cx, 4, a[0] as u32, kind, kind != FLOAT, false, a[2] as u32 as i32, a[3]);
    Reply::Value(0)
}

fn gl_fog_coord_pointer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_attrib_pointer(task, cx, 5, 1, a[0] as u32, false, false, a[1] as u32 as i32, a[2]);
    Reply::Value(0)
}

fn gl_tex_coord_pointer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let unit = current_ctx(task)
        .and_then(|(st, h)| lock(&st.inner).contexts.get(&h).map(|c| c.client_unit))
        .unwrap_or(0);
    send_attrib_pointer(task, cx, 8 + unit, a[0] as u32, a[1] as u32, false, false, a[2] as u32 as i32, a[3]);
    Reply::Value(0)
}

fn gl_client_active_texture(task: &mut Task, _cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some((st, h)) = current_ctx(task)
        && let Some(c) = lock(&st.inner).contexts.get_mut(&h)
    {
        c.client_unit = (a[0] as u32).wrapping_sub(0x84c0).min(7);
    }
    Reply::Value(0)
}

fn client_array_index(task: &Task, array: u32) -> Option<u32> {
    Some(match array {
        0x8074 => 0,
        0x8075 => 2,
        0x8076 => 3,
        0x845e => 4,
        0x8457 => 5,
        0x8078 => {
            let unit = current_ctx(task)
                .and_then(|(st, h)| lock(&st.inner).contexts.get(&h).map(|c| c.client_unit))
                .unwrap_or(0);
            8 + unit
        }
        _ => return None,
    })
}

fn gl_enable_client_state(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(i) = client_array_index(task, a[0] as u32) {
        let _ = send_words(task, cx, wire::ENABLE_ATTRIB, &[i, 1]);
    }
    Reply::Value(0)
}

fn gl_disable_client_state(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(i) = client_array_index(task, a[0] as u32) {
        let _ = send_words(task, cx, wire::ENABLE_ATTRIB, &[i, 0]);
    }
    Reply::Value(0)
}

fn set_color(task: &mut Task, cx: &mut Cx<'_>, rgba: [f32; 4]) {
    if let Some((st, h)) = current_ctx(task) {
        if let Some(c) = lock(&st.inner).contexts.get_mut(&h) {
            c.imm_color = rgba;
            if c.imm.is_some() {
                return;
            }
        }
    }
    send_attrib_f32(task, cx, 3, rgba);
}

fn ub(x: u8) -> f32 {
    f32::from(x) / 255.0
}

fn gl_color3f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 0, 0), task.f32_arg(cx, a, 1, 1), task.f32_arg(cx, a, 2, 2), 1.0];
    set_color(task, cx, v);
    Reply::Value(0)
}
fn gl_color4f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 0, 0), task.f32_arg(cx, a, 1, 1), task.f32_arg(cx, a, 2, 2), task.f32_arg(cx, a, 3, 3)];
    set_color(task, cx, v);
    Reply::Value(0)
}
fn gl_color3d(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [f64_arg(task, cx, a, 0, 0) as f32, f64_arg(task, cx, a, 1, 1) as f32, f64_arg(task, cx, a, 2, 2) as f32, 1.0];
    set_color(task, cx, v);
    Reply::Value(0)
}
fn gl_color4d(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [f64_arg(task, cx, a, 0, 0) as f32, f64_arg(task, cx, a, 1, 1) as f32, f64_arg(task, cx, a, 2, 2) as f32, f64_arg(task, cx, a, 3, 3) as f32];
    set_color(task, cx, v);
    Reply::Value(0)
}
fn gl_color3ub(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_color(task, cx, [ub(a[0] as u8), ub(a[1] as u8), ub(a[2] as u8), 1.0]);
    Reply::Value(0)
}
fn gl_color4ub(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_color(task, cx, [ub(a[0] as u8), ub(a[1] as u8), ub(a[2] as u8), ub(a[3] as u8)]);
    Reply::Value(0)
}
fn gl_color3fv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(v) = read_f32s(task, cx, a[0], 3) {
        set_color(task, cx, [v[0], v[1], v[2], 1.0]);
    }
    Reply::Value(0)
}
fn gl_color4fv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(v) = read_f32s(task, cx, a[0], 4) {
        set_color(task, cx, [v[0], v[1], v[2], v[3]]);
    }
    Reply::Value(0)
}
fn gl_color4ubv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] != 0
        && let Ok(v) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], 4))
    {
        set_color(task, cx, [ub(v[0]), ub(v[1]), ub(v[2]), ub(v[3])]);
    }
    Reply::Value(0)
}

fn set_normal(task: &mut Task, cx: &mut Cx<'_>, n: [f32; 3]) {
    if let Some((st, h)) = current_ctx(task) {
        if let Some(c) = lock(&st.inner).contexts.get_mut(&h) {
            c.imm_normal = n;
            if c.imm.is_some() {
                return;
            }
        }
    }
    send_attrib_f32(task, cx, 2, [n[0], n[1], n[2], 1.0]);
}

fn gl_normal3f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 0, 0), task.f32_arg(cx, a, 1, 1), task.f32_arg(cx, a, 2, 2)];
    set_normal(task, cx, v);
    Reply::Value(0)
}
fn gl_normal3fv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(v) = read_f32s(task, cx, a[0], 3) {
        set_normal(task, cx, [v[0], v[1], v[2]]);
    }
    Reply::Value(0)
}

fn set_texcoord0(task: &mut Task, cx: &mut Cx<'_>, tc: [f32; 4]) {
    if let Some((st, h)) = current_ctx(task) {
        if let Some(c) = lock(&st.inner).contexts.get_mut(&h) {
            c.imm_tex0 = tc;
            if c.imm.is_some() {
                return;
            }
        }
    }
    send_attrib_f32(task, cx, 8, tc);
}

fn gl_tex_coord2f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 0, 0), task.f32_arg(cx, a, 1, 1), 0.0, 1.0];
    set_texcoord0(task, cx, v);
    Reply::Value(0)
}
fn gl_tex_coord2d(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [f64_arg(task, cx, a, 0, 0) as f32, f64_arg(task, cx, a, 1, 1) as f32, 0.0, 1.0];
    set_texcoord0(task, cx, v);
    Reply::Value(0)
}
fn gl_tex_coord2i(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_texcoord0(task, cx, [(a[0] as u32 as i32) as f32, (a[1] as u32 as i32) as f32, 0.0, 1.0]);
    Reply::Value(0)
}
fn gl_tex_coord2fv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(v) = read_f32s(task, cx, a[0], 2) {
        set_texcoord0(task, cx, [v[0], v[1], 0.0, 1.0]);
    }
    Reply::Value(0)
}
fn gl_multi_tex_coord2f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let unit = (a[0] as u32).wrapping_sub(0x84c0).min(7);
    let tc = [task.f32_arg(cx, a, 1, 0), task.f32_arg(cx, a, 2, 1), 0.0, 1.0];
    if unit == 0 {
        set_texcoord0(task, cx, tc);
    } else {
        send_attrib_f32(task, cx, 8 + unit, tc);
    }
    Reply::Value(0)
}
fn gl_multi_tex_coord4f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let unit = (a[0] as u32).wrapping_sub(0x84c0).min(7);
    let tc = [task.f32_arg(cx, a, 1, 0), task.f32_arg(cx, a, 2, 1), task.f32_arg(cx, a, 3, 2), task.f32_arg(cx, a, 4, 3)];
    if unit == 0 {
        set_texcoord0(task, cx, tc);
    } else {
        send_attrib_f32(task, cx, 8 + unit, tc);
    }
    Reply::Value(0)
}
fn gl_secondary_color3f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 0, 0), task.f32_arg(cx, a, 1, 1), task.f32_arg(cx, a, 2, 2), 1.0];
    send_attrib_f32(task, cx, 4, v);
    Reply::Value(0)
}
fn gl_fog_coordf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [task.f32_arg(cx, a, 0, 0), 0.0, 0.0, 1.0];
    send_attrib_f32(task, cx, 5, v);
    Reply::Value(0)
}

// ---- Immediate mode (`glBegin` / `glVertex*` / `glEnd`) ----

fn gl_begin(task: &mut Task, _cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some((st, h)) = current_ctx(task)
        && let Some(c) = lock(&st.inner).contexts.get_mut(&h)
    {
        c.imm = Some(ImmBatch { mode: a[0] as u32, verts: Vec::new() });
    }
    Reply::Value(0)
}

fn push_vertex(task: &Task, pos: [f32; 4]) {
    if let Some((st, h)) = current_ctx(task)
        && let Some(c) = lock(&st.inner).contexts.get_mut(&h)
    {
        let (normal, color, tex0) = (c.imm_normal, c.imm_color, c.imm_tex0);
        if let Some(imm) = &mut c.imm {
            imm.verts.push(ImmVert { pos, normal, color, tex0 });
        }
    }
}

fn gl_vertex2f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pos = [task.f32_arg(cx, a, 0, 0), task.f32_arg(cx, a, 1, 1), 0.0, 1.0];
    push_vertex(task, pos);
    Reply::Value(0)
}
fn gl_vertex2i(task: &mut Task, _cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    push_vertex(task, [(a[0] as u32 as i32) as f32, (a[1] as u32 as i32) as f32, 0.0, 1.0]);
    Reply::Value(0)
}
fn gl_vertex3f(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pos = [task.f32_arg(cx, a, 0, 0), task.f32_arg(cx, a, 1, 1), task.f32_arg(cx, a, 2, 2), 1.0];
    push_vertex(task, pos);
    Reply::Value(0)
}
fn gl_vertex3d(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pos = [f64_arg(task, cx, a, 0, 0) as f32, f64_arg(task, cx, a, 1, 1) as f32, f64_arg(task, cx, a, 2, 2) as f32, 1.0];
    push_vertex(task, pos);
    Reply::Value(0)
}

fn gl_end(task: &mut Task, cx: &mut Cx<'_>, _a: &[u64]) -> Reply {
    let Some((st, hglrc)) = current_ctx(task) else { return Reply::Value(0) };
    let (batch, saved_vao, saved_vbo) = {
        let mut inner = lock(&st.inner);
        let Some(c) = inner.contexts.get_mut(&hglrc) else { return Reply::Value(0) };
        (c.imm.take(), c.bound_vao, c.bound_array_buffer)
    };
    let Some(imm) = batch else { return Reply::Value(0) };
    if imm.verts.is_empty() {
        return Reply::Value(0);
    }
    // Convert GL_QUADS (7) -> GL_TRIANGLES (4), GL_POLYGON (9) -> GL_TRIANGLE_FAN (6).
    let (mode, verts): (u32, Vec<ImmVert>) = match imm.mode {
        7 => {
            let mut tri = Vec::with_capacity((imm.verts.len() / 4) * 6);
            for q in imm.verts.chunks_exact(4) {
                tri.extend_from_slice(&[q[0], q[1], q[2], q[0], q[2], q[3]]);
            }
            (4, tri)
        }
        9 => (6, imm.verts),
        other => (other, imm.verts),
    };
    if verts.is_empty() {
        return Reply::Value(0);
    }
    const STRIDE: u32 = 15 * 4; // 4 pos + 3 normal + 4 color + 4 tex0
    let mut vdata = Vec::with_capacity(verts.len() * STRIDE as usize);
    for v in &verts {
        for f in v.pos.iter().chain(&v.normal).chain(&v.color).chain(&v.tex0) {
            vdata.extend_from_slice(&f.to_le_bytes());
        }
    }
    let mut recs = Vec::with_capacity(vdata.len() + 256);
    fn push_words(recs: &mut Vec<u8>, kind: u32, w: &[u32]) {
        let mut p = Vec::with_capacity(w.len() * 4);
        for &x in w {
            records::put_u32(&mut p, x);
        }
        records::record(recs, kind, &p);
    }
    fn push_ptr(recs: &mut Vec<u8>, index: u32, size: u32, offset: u64) {
        let mut p = Vec::with_capacity(32);
        for w in [index, size, FLOAT, 0, 0, STRIDE] {
            records::put_u32(&mut p, w);
        }
        records::put_u64(&mut p, offset);
        records::record(recs, wire::ATTRIB_POINTER, &p);
        let mut ep = Vec::with_capacity(8);
        records::put_u32(&mut ep, index);
        records::put_u32(&mut ep, 1);
        records::record(recs, wire::ENABLE_ATTRIB, &ep);
    }
    push_words(&mut recs, wire::BIND_VERTEX_ARRAY, &[IMM_VAO]);
    push_words(&mut recs, wire::BIND_BUFFER, &[0x8892, IMM_VBO]);
    {
        let mut p = Vec::with_capacity(vdata.len() + 24);
        records::put_u32(&mut p, 0x8892);
        records::put_u64(&mut p, vdata.len() as u64);
        records::put_u32(&mut p, 0x88e0); // GL_STREAM_DRAW
        records::put_u32(&mut p, 1);
        records::put_bytes(&mut p, &vdata);
        records::record(&mut recs, wire::BUFFER_DATA, &p);
    }
    push_ptr(&mut recs, 0, 4, 0);
    push_ptr(&mut recs, 2, 3, 16);
    push_ptr(&mut recs, 3, 4, 28);
    push_ptr(&mut recs, 8, 4, 44);
    push_words(&mut recs, wire::DRAW_ARRAYS, &[mode, 0, verts.len() as u32, 1]);
    push_words(&mut recs, wire::BIND_VERTEX_ARRAY, &[saved_vao]);
    push_words(&mut recs, wire::BIND_BUFFER, &[0x8892, saved_vbo]);
    let _ = exec_records(task, cx, &st, hglrc, &recs);
    Reply::Value(0)
}

fn gl_rectf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = [
        task.f32_arg(cx, a, 0, 0).to_bits(),
        task.f32_arg(cx, a, 1, 1).to_bits(),
        task.f32_arg(cx, a, 2, 2).to_bits(),
        task.f32_arg(cx, a, 3, 3).to_bits(),
    ];
    let _ = send_words(task, cx, wire::RECT, &v);
    Reply::Value(0)
}

// ---- Texture environments, alpha test, fog, shading ----

fn compat_param(task: &mut Task, cx: &mut Cx<'_>, kind: u32, pname: u32, values: [u32; 4], float: bool) {
    let _ = send_words(task, cx, kind, &[pname, values[0], values[1], values[2], values[3], u32::from(float)]);
}

fn param_count(pname: u32) -> usize {
    if matches!(pname, 0x2201 | 0x0b66) { 4 } else { 1 }
}

fn read_param4(task: &mut Task, cx: &mut Cx<'_>, ptr: u64, n: usize) -> [u32; 4] {
    let mut out = [0u32; 4];
    if ptr != 0
        && let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), ptr, n * 4))
    {
        for (i, c) in raw.chunks_exact(4).enumerate().take(4) {
            out[i] = u32::from_le_bytes(c.try_into().unwrap());
        }
    }
    out
}

fn gl_tex_envi(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] as u32 == 0x2300 {
        compat_param(task, cx, wire::TEX_ENV, a[1] as u32, [a[2] as u32, 0, 0, 0], false);
    }
    Reply::Value(0)
}

fn gl_tex_envf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] as u32 == 0x2300 {
        let bits = task.f32_arg(cx, a, 2, 0).to_bits();
        compat_param(task, cx, wire::TEX_ENV, a[1] as u32, [bits, 0, 0, 0], true);
    }
    Reply::Value(0)
}

fn gl_tex_enviv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] as u32 == 0x2300 {
        let pname = a[1] as u32;
        let v = read_param4(task, cx, a[2], param_count(pname));
        compat_param(task, cx, wire::TEX_ENV, pname, v, false);
    }
    Reply::Value(0)
}

fn gl_tex_envfv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] as u32 == 0x2300 {
        let pname = a[1] as u32;
        let v = read_param4(task, cx, a[2], param_count(pname));
        compat_param(task, cx, wire::TEX_ENV, pname, v, true);
    }
    Reply::Value(0)
}

fn gl_alpha_func(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let reference = task.f32_arg(cx, a, 1, 0).to_bits();
    let _ = send_words(task, cx, wire::ALPHA_FUNC, &[a[0] as u32, reference]);
    Reply::Value(0)
}

fn gl_fogf(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let bits = task.f32_arg(cx, a, 1, 0).to_bits();
    compat_param(task, cx, wire::FOG, a[0] as u32, [bits, 0, 0, 0], true);
    Reply::Value(0)
}

fn gl_fogi(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    compat_param(task, cx, wire::FOG, a[0] as u32, [a[1] as u32, 0, 0, 0], false);
    Reply::Value(0)
}

fn gl_fogfv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pname = a[0] as u32;
    let v = read_param4(task, cx, a[1], param_count(pname));
    compat_param(task, cx, wire::FOG, pname, v, true);
    Reply::Value(0)
}

fn gl_fogiv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pname = a[0] as u32;
    let v = read_param4(task, cx, a[1], param_count(pname));
    compat_param(task, cx, wire::FOG, pname, v, false);
    Reply::Value(0)
}

fn gl_shade_model(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = send_words(task, cx, wire::SHADE_MODEL, &[a[0] as u32]);
    Reply::Value(0)
}
