//! opengl32.dll: Atlas has no OpenGL driver yet (programs use D3D11/D3D9/software), so the WGL entry points report
//! that no OpenGL rendering context can be created while the `gl*` entry points remain `missing`.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "OPENGL32.dll";
    missing glBegin;
    missing glBindTexture;
    missing glBlendFunc;
    missing glClear;
    missing glColor4fv;
    missing glColor4ub;
    missing glColorMask;
    missing glDeleteTextures;
    missing glDepthMask;
    missing glDepthRange;
    missing glDisable;
    missing glDisableClientState;
    missing glDrawArrays;
    missing glDrawElements;
    missing glEnable;
    missing glEnd;
    missing glGenTextures;
    missing glGetBooleanv;
    missing glGetDoublev;
    missing glGetError;
    missing glGetFloatv;
    missing glGetIntegerv;
    missing glGetString;
    missing glIsEnabled;
    missing glIsTexture;
    missing glLineWidth;
    missing glLoadIdentity;
    missing glLoadMatrixf;
    missing glMatrixMode;
    missing glOrtho;
    missing glPixelStorei;
    missing glPixelZoom;
    missing glPolygonMode;
    missing glPopAttrib;
    missing glPopClientAttrib;
    missing glPushAttrib;
    missing glPushClientAttrib;
    missing glReadPixels;
    missing glRenderMode;
    missing glScalef;
    missing glScissor;
    missing glShadeModel;
    missing glTexCoord2d;
    missing glTexCoord2f;
    missing glTexCoord2i;
    missing glTexEnvf;
    missing glTexImage2D;
    missing glTexParameterf;
    missing glTexParameteri;
    missing glTexSubImage2D;
    missing glTranslatef;
    missing glVertex2f;
    missing glVertex3f;
    missing glViewport;
    host wglCreateContext(1) => wgl_create_context;
    host wglDeleteContext(1) => zero;
    host wglGetCurrentContext(0) => zero;
    host wglGetCurrentDC(0) => zero;
    host wglGetProcAddress(1) => zero;
    host wglMakeCurrent(2) => wgl_make_current;
    host wglShareLists(2) => zero;
}

const ERROR_NOT_SUPPORTED: u32 = 50;

fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `wglCreateContext(hdc)`: no OpenGL driver available; fail cleanly so callers fall back to D3D/software.
fn wgl_create_context(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    task.set_last_error(cx, ERROR_NOT_SUPPORTED);
    Reply::Value(0)
}

/// `wglMakeCurrent(hdc, hglrc)`: detaching (`hglrc == NULL`) succeeds; attaching a non-null context fails.
fn wgl_make_current(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(a[1] == 0))
}
