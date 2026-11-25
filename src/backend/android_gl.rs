// SPDX-License-Identifier: GPL-3.0-only
//
// EGL-вывод в ANativeWindow приложения god-termux (Android, bionic).
// Заменяет smithay::backend::winit: окно приходит от приложения (JNI), а не от оконной системы.

use smithay::{
    backend::{
        SwapBuffersError,
        egl::{
            EGLContext, EGLDisplay, EGLSurface, Error as EGLError, ffi,
            context::{GlAttributes, PixelFormatRequirements},
            display::EGLDisplayHandle,
            native::{EGLNativeDisplay, EGLNativeSurface, EGLPlatform},
            wrap_egl_call_ptr,
        },
        renderer::{
            Bind, RendererSuper,
            gles::GlesRenderer,
            glow::GlowRenderer,
        },
    },
    utils::{Physical, Rectangle, Size},
};
use std::{
    ffi::c_void,
    sync::{
        Arc,
        atomic::{AtomicPtr, AtomicU32, Ordering},
    },
};

#[link(name = "android")]
unsafe extern "C" {
    fn ANativeWindow_getWidth(window: *mut c_void) -> i32;
    fn ANativeWindow_getHeight(window: *mut c_void) -> i32;
}

/// ANativeWindow, переданный приложением (nativeCosmicStart). Задаётся до запуска композитора.
static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static REFRESH_MHZ: AtomicU32 = AtomicU32::new(60_000);

/// Приложение передаёт окно и реальную частоту экрана.
pub fn set_window(window: *mut c_void, refresh_mhz: u32) {
    WINDOW.store(window, Ordering::Release);
    REFRESH_MHZ.store(refresh_mhz.clamp(24_000, 360_000), Ordering::Release);
}

pub fn refresh_mhz() -> u32 {
    REFRESH_MHZ.load(Ordering::Acquire)
}

/// EGL_PLATFORM_ANDROID_KHR
const PLATFORM_ANDROID_KHR: ffi::egl::types::EGLenum = 0x3141;

struct AndroidDisplay;

impl EGLNativeDisplay for AndroidDisplay {
    fn supported_platforms(&self) -> Vec<EGLPlatform<'_>> {
        vec![EGLPlatform::new(
            PLATFORM_ANDROID_KHR,
            "EGL_PLATFORM_ANDROID_KHR",
            std::ptr::null_mut(),
            vec![ffi::egl::NONE as ffi::EGLint],
            &["EGL_KHR_platform_android"],
        )]
    }
}

struct AndroidSurface(*mut c_void);

// ANativeWindow потокобезопасен (счётчик ссылок внутри)
unsafe impl Send for AndroidSurface {}

unsafe impl EGLNativeSurface for AndroidSurface {
    unsafe fn create(
        &self,
        display: &Arc<EGLDisplayHandle>,
        config_id: ffi::egl::types::EGLConfig,
    ) -> Result<*const c_void, smithay::backend::egl::EGLError> {
        wrap_egl_call_ptr(|| unsafe {
            ffi::egl::CreateWindowSurface(display.handle, config_id, self.0 as *mut _, std::ptr::null())
        })
    }

    fn identifier(&self) -> Option<String> {
        Some("Android/ANativeWindow".into())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("приложение не передало ANativeWindow")]
    NoWindow,
    #[error("EGL: {0}")]
    Egl(#[from] EGLError),
    #[error("рендерер: {0}")]
    Renderer(#[from] smithay::backend::renderer::gles::GlesError),
}

#[derive(Debug)]
pub struct AndroidGraphicsBackend {
    renderer: GlowRenderer,
    // дисплей должен жить, пока жив рендерер
    _display: EGLDisplay,
    egl_surface: EGLSurface,
    window: *mut c_void,
    damage_tracking: bool,
}

// окно используется только из потока цикла событий композитора
unsafe impl Send for AndroidGraphicsBackend {}

impl AndroidGraphicsBackend {
    pub fn new() -> Result<Self, Error> {
        let window = WINDOW.load(Ordering::Acquire);
        if window.is_null() {
            return Err(Error::NoWindow);
        }

        let display = unsafe { EGLDisplay::new(AndroidDisplay)? };
        let attrs = GlAttributes {
            version: (3, 0),
            profile: None,
            debug: false,
            vsync: true,
        };
        let context = EGLContext::new_with_config(&display, attrs, PixelFormatRequirements::_10_bit())
            .or_else(|_| EGLContext::new_with_config(&display, attrs, PixelFormatRequirements::_8_bit()))?;
        let surface = unsafe {
            EGLSurface::new(
                &display,
                context.pixel_format().unwrap(),
                context.config_id(),
                AndroidSurface(window),
            )
            .map_err(EGLError::CreationFailed)?
        };
        let _ = context.unbind();

        let renderer: GlowRenderer = unsafe { GlesRenderer::new(context)?.into() };
        let damage_tracking = display.supports_damage();
        Ok(Self {
            renderer,
            _display: display,
            egl_surface: surface,
            window,
            damage_tracking,
        })
    }

    pub fn window_size(&self) -> Size<i32, Physical> {
        let (w, h) = unsafe { (ANativeWindow_getWidth(self.window), ANativeWindow_getHeight(self.window)) };
        (w, h).into()
    }

    pub fn renderer(&mut self) -> &mut GlowRenderer {
        &mut self.renderer
    }

    pub fn bind(
        &mut self,
    ) -> Result<(&mut GlowRenderer, <GlowRenderer as RendererSuper>::Framebuffer<'_>), SwapBuffersError> {
        let fb = self.renderer.bind(&mut self.egl_surface)?;
        Ok((&mut self.renderer, fb))
    }

    pub fn buffer_age(&self) -> Option<usize> {
        if self.damage_tracking {
            self.egl_surface.buffer_age().map(|x| x as usize)
        } else {
            Some(0)
        }
    }

    pub fn submit(&mut self, damage: Option<&[Rectangle<i32, Physical>]>) -> Result<(), SwapBuffersError> {
        let size = self.window_size();
        let mut damage = match damage {
            Some(damage) if self.damage_tracking && !damage.is_empty() => Some(
                damage
                    .iter()
                    .map(|rect| {
                        Rectangle::new((rect.loc.x, size.h - rect.loc.y - rect.size.h).into(), rect.size)
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        };
        self.egl_surface.swap_buffers(damage.as_deref_mut())?;
        Ok(())
    }
}
