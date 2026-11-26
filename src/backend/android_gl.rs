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
        atomic::{AtomicPtr, AtomicU32, AtomicU64, Ordering},
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
/// Окно потеряно (Surface уничтожен): рендер приостановлен до cosmic_set_window()
pub static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Приложение передаёт окно и реальную частоту экрана.
pub fn set_window(window: *mut c_void, refresh_mhz: u32) {
    WINDOW.store(window, Ordering::Release);
    REFRESH_MHZ.store(refresh_mhz.clamp(24_000, 360_000), Ordering::Release);
}

/// Безоконный режим (тесты): рендер в EGL pbuffer, размер w<<32|h. 0 — выключен.
static HEADLESS: AtomicU64 = AtomicU64::new(0);

pub fn set_headless(w: u32, h: u32) {
    HEADLESS.store(((w as u64) << 32) | h as u64, Ordering::Release);
}

fn headless_size() -> Option<(i32, i32)> {
    let v = HEADLESS.load(Ordering::Acquire);
    (v != 0).then(|| ((v >> 32) as i32, (v & 0xffff_ffff) as i32))
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

struct PbufferSurface(i32, i32);

unsafe impl EGLNativeSurface for PbufferSurface {
    unsafe fn create(
        &self,
        display: &Arc<EGLDisplayHandle>,
        config_id: ffi::egl::types::EGLConfig,
    ) -> Result<*const c_void, smithay::backend::egl::EGLError> {
        let attrs = [
            ffi::egl::WIDTH as i32, self.0,
            ffi::egl::HEIGHT as i32, self.1,
            ffi::egl::NONE as i32,
        ];
        wrap_egl_call_ptr(|| unsafe { ffi::egl::CreatePbufferSurface(display.handle, config_id, attrs.as_ptr()) })
    }

    fn identifier(&self) -> Option<String> {
        Some("Android/pbuffer".into())
    }
}

/// eglGetDisplay(EGL_DEFAULT_DISPLAY) + eglInitialize + первая подходящая конфигурация окна (или pbuffer) GLES.
unsafe fn default_display(pbuffer: bool) -> Result<EGLDisplay, String> {
    ffi::make_sure_egl_is_loaded().map_err(|e| format!("загрузка EGL: {e:?}"))?;
    let dpy = unsafe { ffi::egl::GetDisplay(std::ptr::null()) };
    if dpy.is_null() {
        return Err("eglGetDisplay вернул NO_DISPLAY".into());
    }
    let (mut major, mut minor) = (0, 0);
    if unsafe { ffi::egl::Initialize(dpy, &mut major, &mut minor) } == 0 {
        return Err("eglInitialize не удался".into());
    }
    unsafe { ffi::egl::BindAPI(ffi::egl::OPENGL_ES_API) };
    let attrs = [
        ffi::egl::SURFACE_TYPE as i32, if pbuffer { ffi::egl::PBUFFER_BIT } else { ffi::egl::WINDOW_BIT } as i32,
        ffi::egl::RENDERABLE_TYPE as i32, ffi::egl::OPENGL_ES2_BIT as i32,
        ffi::egl::NONE as i32,
    ];
    let mut cfg: ffi::egl::types::EGLConfig = std::ptr::null();
    let mut n = 0;
    if unsafe { ffi::egl::ChooseConfig(dpy, attrs.as_ptr(), &mut cfg, 1, &mut n) } == 0 || n < 1 || cfg.is_null() {
        return Err("нет EGL-конфигурации для окна".into());
    }
    unsafe { EGLDisplay::from_raw(dpy, cfg) }.map_err(|e| format!("EGLDisplay::from_raw: {e:?}"))
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
    headless: Option<(i32, i32)>,
    pixel_format: smithay::backend::egl::display::PixelFormat,
    config_id: ffi::egl::types::EGLConfig,
    damage_tracking: bool,
}

// окно используется только из потока цикла событий композитора
unsafe impl Send for AndroidGraphicsBackend {}

impl AndroidGraphicsBackend {
    pub fn new() -> Result<Self, Error> {
        let window = WINDOW.load(Ordering::Acquire);
        let headless = if window.is_null() { headless_size() } else { None };
        if window.is_null() && headless.is_none() {
            return Err(Error::NoWindow);
        }

        // На Android eglGetDisplay(EGL_DEFAULT_DISPLAY) надёжнее платформенного eglGetPlatformDisplayEXT:
        // клиентские расширения системного EGL не всегда содержат EGL_KHR_platform_android
        let display = match unsafe { default_display(headless.is_some()) } {
            Ok(d) => d,
            Err(e) => {
                super::android_entry::alog(&format!("EGL: eglGetDisplay не подошёл ({e}), пробую платформу Android"));
                unsafe { EGLDisplay::new(AndroidDisplay)? }
            }
        };
        let attrs = GlAttributes {
            version: (3, 0),
            profile: None,
            debug: false,
            vsync: true,
        };
        // 8 бит первым: буфер SurfaceView 8-битный, а с 10-битной конфигурацией eglSwapBuffers даёт EGL_BAD_ALLOC
        let context = EGLContext::new_with_config(&display, attrs, PixelFormatRequirements::_8_bit())
            .or_else(|_| EGLContext::new_with_config(&display, attrs, PixelFormatRequirements::_10_bit()))?;
        let pixel_format = context.pixel_format().unwrap();
        let config_id = context.config_id();
        let surface = unsafe {
            if let Some((w, h)) = headless {
                EGLSurface::new(&display, context.pixel_format().unwrap(), context.config_id(), PbufferSurface(w, h))
            } else {
                EGLSurface::new(&display, context.pixel_format().unwrap(), context.config_id(), AndroidSurface(window))
            }
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
            headless,
            pixel_format,
            config_id,
            damage_tracking,
        })
    }

    pub fn window_size(&self) -> Size<i32, Physical> {
        if let Some((w, h)) = self.headless {
            return (w, h).into();
        }
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
        // pbuffer одинарный и сохраняет содержимое между кадрами: возраст всегда 1
        if self.headless.is_some() {
            return Some(1);
        }
        if self.damage_tracking {
            self.egl_surface.buffer_age().map(|x| x as usize)
        } else {
            Some(0)
        }
    }

    pub fn submit(&mut self, damage: Option<&[Rectangle<i32, Physical>]>) -> Result<(), SwapBuffersError> {
        let size = self.window_size();
        let damage = if self.headless.is_some() { None } else { damage };
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

impl AndroidGraphicsBackend {
    /// Снимок текущего кадра (RGBA, строки сверху вниз). Для тестов: штатный ExportMem::copy_framebuffer.
    pub fn read_rgba(&mut self) -> Result<(i32, i32, Vec<u8>), String> {
        use smithay::backend::{
            allocator::Fourcc,
            renderer::{ExportMem, TextureMapping},
        };
        let size = self.window_size();
        let (w, h) = (size.w, size.h);
        let (renderer, fb) = self.bind().map_err(|e| format!("bind: {e:?}"))?;
        let region = Rectangle::<i32, smithay::utils::Buffer>::new((0, 0).into(), (w, h).into());
        let mapping = renderer
            .copy_framebuffer(&fb, region, Fourcc::Abgr8888)
            .map_err(|e| format!("copy_framebuffer: {e:?}"))?;
        let flipped = mapping.flipped();
        let data = renderer.map_texture(&mapping).map_err(|e| format!("map_texture: {e:?}"))?.to_vec();
        drop(fb);
        if !flipped {
            return Ok((w, h, data));
        }
        let stride = (w * 4) as usize;
        let mut out = vec![0u8; data.len()];
        for y in 0..h as usize {
            let src = (h as usize - 1 - y) * stride;
            out[y * stride..(y + 1) * stride].copy_from_slice(&data[src..src + stride]);
        }
        Ok((w, h, out))
    }
}

impl AndroidGraphicsBackend {
    /// Приложение пересоздало Surface: новая EGL-поверхность на новом ANativeWindow, старая удаляется.
    pub fn replace_window(&mut self, window: *mut c_void) -> Result<(), String> {
        if self.headless.is_some() || window.is_null() {
            return Err("replace_window: нет окна или безоконный режим".into());
        }
        let surface = unsafe {
            EGLSurface::new(&self._display, self.pixel_format, self.config_id, AndroidSurface(window))
                .map_err(|e| format!("EGLSurface::new: {e:?}"))?
        };
        self.egl_surface = surface;
        self.window = window;
        Ok(())
    }
}
