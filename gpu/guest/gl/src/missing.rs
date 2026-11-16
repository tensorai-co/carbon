//! OpenGL ES 2.0 and 3.0 functions this library hasn't built yet: each, called, says so once on the program's
//! stderr and does nothing (what it answers is undefined, as an unknown function's would be). They're here, not
//! left to GLVND's no-op, so a program's use of one is seen.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

fn said(name: &[u8], once: &AtomicBool) {
    if !once.swap(true, Ordering::Relaxed) {
        let mut m = name.to_vec();
        m.extend_from_slice(b" isn't built yet");
        crate::complain(&m);
    }
}

macro_rules! missing {
    ($($name:ident),* $(,)?) => {
        $(
            #[allow(non_snake_case)]
            unsafe extern "C" fn $name() {
                static ONCE: AtomicBool = AtomicBool::new(false);
                said(stringify!($name).as_bytes(), &ONCE);
            }
        )*

        /// The functions here, by name.
        pub(crate) fn lookup(name: &[u8]) -> Option<*const c_void> {
            let table: &[(&[u8], *const c_void)] = &[$((stringify!($name).as_bytes(), $name as *const c_void)),*];
            table.iter().find(|e| e.0 == name).map(|e| e.1)
        }
    };
}

missing!(
    glBlendColor, glCompressedTexImage2D, glCompressedTexSubImage2D, glCopyTexImage2D,
    glGetFramebufferAttachmentParameteriv, glGetRenderbufferParameteriv, glGetShaderSource, glGetTexParameterfv,
    glGetTexParameteriv, glGetUniformfv, glGetUniformiv, glGetVertexAttribfv, glGetVertexAttribiv,
    glGetVertexAttribPointerv,
    glTexImage3D, glTexSubImage3D,
    glCopyTexSubImage3D, glCompressedTexImage3D, glCompressedTexSubImage3D, glGenQueries, glDeleteQueries, glIsQuery,
    glBeginQuery, glEndQuery, glGetQueryiv, glGetQueryObjectuiv, glFramebufferTextureLayer,
    glGetIntegeri_v, glBeginTransformFeedback, glEndTransformFeedback, glTransformFeedbackVaryings,
    glGetTransformFeedbackVarying, glGetVertexAttribIiv, glGetVertexAttribIuiv,
    glGetUniformuiv, glGetFragDataLocation,
    glCopyBufferSubData, glGetUniformIndices, glGetActiveUniformsiv, glGetInteger64i_v, glGetSamplerParameteriv,
    glGetSamplerParameterfv, glBindTransformFeedback, glDeleteTransformFeedbacks, glGenTransformFeedbacks,
    glIsTransformFeedback, glPauseTransformFeedback, glResumeTransformFeedback, glGetProgramBinary, glProgramBinary,
    glTexStorage3D,
);
