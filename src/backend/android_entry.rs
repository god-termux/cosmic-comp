// SPDX-License-Identifier: GPL-3.0-only
//
// C-интерфейс для приложения god-termux: запуск композитора в потоке приложения и приём ввода.
// Приложение вызывает cosmic_start() из отдельного потока (она блокируется до остановки),
// остальные функции — из любых потоков.

use super::android_input::{
    AndroidInput, AndroidKeyboardInputEvent, AndroidMouseButton, AndroidMouseInputEvent,
    AndroidMouseMovedEvent, AndroidMouseWheelEvent, AndroidTouchCancelledEvent,
    AndroidTouchEndedEvent, AndroidTouchMovedEvent, AndroidTouchStartedEvent, AndroidVirtualDevice,
    ElementState, MouseScrollDelta, PhysicalPosition, RelativePosition,
};
use smithay::{
    backend::input::{InputEvent, InputTime},
    reexports::calloop::channel::Sender,
};
use std::{
    ffi::{CString, c_void},
    sync::Mutex,
};

unsafe extern "C" {
    fn open(path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    fn dup2(old: i32, new: i32) -> i32;
    fn close(fd: i32) -> i32;
}

/// stdout/stderr приложения Android ведут в никуда, а логгер cosmic-comp пишет в stdout:
/// направляем оба потока в файл XDG_RUNTIME_DIR/cosmic-comp.log
fn redirect_output() {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else { return };
    let path = std::path::Path::new(&dir).join("cosmic-comp.log");
    let Ok(c) = CString::new(path.to_string_lossy().as_bytes()) else { return };
    // O_WRONLY | O_CREAT | O_TRUNC (bionic: 1 | 0x40 | 0x200)
    let fd = unsafe { open(c.as_ptr(), 1 | 0x40 | 0x200, 0o600u32) };
    if fd >= 0 {
        unsafe {
            dup2(fd, 1);
            dup2(fd, 2);
            close(fd);
        }
    }
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: i32, tag: *const std::ffi::c_char, text: *const std::ffi::c_char) -> i32;
}

/// Сообщение в logcat (тег cosmic-comp); stderr приложения никуда не идёт.
pub fn alog(msg: &str) {
    if let (Ok(t), Ok(m)) = (CString::new("cosmic-comp"), CString::new(msg.replace('\0', ""))) {
        unsafe { __android_log_write(6, t.as_ptr(), m.as_ptr()) };
    }
}

/// Сырое событие ввода от приложения (координаты в пикселях окна).
#[derive(Debug, Clone, Copy)]
pub enum RawInput {
    Motion { x: f64, y: f64 },
    Button { button: u32, pressed: bool },
    Scroll { dy: f64 },
    Touch { kind: u32, id: u64, x: f64, y: f64 },
    Key { code: u32, pressed: bool },
    Quit,
}

static SENDER: Mutex<Option<Sender<RawInput>>> = Mutex::new(None);

pub fn register_sender(sender: Sender<RawInput>) {
    *SENDER.lock().unwrap() = Some(sender);
}

fn send(ev: RawInput) {
    if let Some(s) = SENDER.lock().unwrap().as_ref() {
        let _ = s.send(ev);
    }
}

fn state_of(pressed: bool) -> ElementState {
    if pressed { ElementState::Pressed } else { ElementState::Released }
}

/// Превращает сырое событие в событие smithay. `size` — размер окна в пикселях.
pub fn to_input_event(raw: RawInput, size: (i32, i32)) -> Option<InputEvent<AndroidInput>> {
    let time = InputTime::now();
    let rel = |x: f64, y: f64| RelativePosition::new(x / size.0.max(1) as f64, y / size.1.max(1) as f64);
    let pos = |x, y| PhysicalPosition { x, y };
    Some(match raw {
        RawInput::Motion { x, y } => InputEvent::PointerMotionAbsolute {
            event: AndroidMouseMovedEvent { time, position: rel(x, y), global_position: pos(x, y) },
        },
        RawInput::Button { button, pressed } => InputEvent::PointerButton {
            event: AndroidMouseInputEvent {
                time,
                button: match button {
                    0 => AndroidMouseButton::Left,
                    1 => AndroidMouseButton::Right,
                    2 => AndroidMouseButton::Middle,
                    3 => AndroidMouseButton::Back,
                    _ => AndroidMouseButton::Forward,
                },
                state: state_of(pressed),
                is_x11: false,
            },
        },
        RawInput::Scroll { dy } => InputEvent::PointerAxis {
            event: AndroidMouseWheelEvent { time, delta: MouseScrollDelta::PixelDelta(pos(0.0, dy)) },
        },
        RawInput::Touch { kind, id, x, y } => match kind {
            0 => InputEvent::TouchDown {
                event: AndroidTouchStartedEvent { time, position: rel(x, y), global_position: pos(x, y), id },
            },
            1 => InputEvent::TouchMotion {
                event: AndroidTouchMovedEvent { time, position: rel(x, y), global_position: pos(x, y), id },
            },
            2 => InputEvent::TouchUp { event: AndroidTouchEndedEvent { time, id } },
            _ => InputEvent::TouchCancel { event: AndroidTouchCancelledEvent { time, id } },
        },
        RawInput::Key { code, pressed } => InputEvent::Keyboard {
            event: AndroidKeyboardInputEvent { time, key: code, count: 1, state: state_of(pressed) },
        },
        RawInput::Quit => return None,
    })
}

pub fn device_added() -> InputEvent<AndroidInput> {
    InputEvent::DeviceAdded { device: AndroidVirtualDevice }
}

/// Запуск композитора. window — ANativeWindow*, refresh_mhz — частота экрана.
/// Блокируется до остановки; возвращает 0 при штатном выходе.
#[unsafe(no_mangle)]
pub extern "C" fn cosmic_start(window: *mut c_void, refresh_mhz: u32) -> i32 {
    // run() не рассчитан на повторный вызов в одном процессе (глобальный логгер, HOOKS): повторный
    // запуск вызвал бы панику и abort() всего приложения, поэтому отказываем заранее
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        alog("cosmic_start: композитор уже запускался в этом процессе, перезапустите приложение");
        return 2;
    }
    // уровень логов без пересборки: содержимое ~/.termux/cosmic-rust-log (например "smithay=debug,cosmic_comp=info")
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(level) = std::fs::read_to_string(std::path::Path::new(&home).join(".termux/cosmic-rust-log")) {
            let level = level.trim();
            if !level.is_empty() {
                unsafe { std::env::set_var("RUST_LOG", level) };
            }
        }
    }
    redirect_output();
    super::android_gl::set_window(window, refresh_mhz);
    std::panic::set_hook(Box::new(|info| alog(&format!("panic: {info}"))));
    // Wayland-сокет композитора и конфиги берутся из окружения процесса приложения (XDG_RUNTIME_DIR и т.д.)
    // паника через границу extern "C" убила бы приложение
    let res = std::panic::catch_unwind(|| crate::run(Default::default()));
    match res {
        Err(payload) => {
            let why = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "неизвестная причина".into());
            alog(&format!("cosmic_start: паника: {why}"));
            3
        }
        Ok(Ok(())) => 0,
        Ok(Err(err)) => {
            alog(&format!("cosmic_start: ошибка: {err:?}"));
            tracing::error!("cosmic_start: {err}");
            1
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn cosmic_stop() {
    send(RawInput::Quit);
}

#[unsafe(no_mangle)]
pub extern "C" fn cosmic_pointer_motion(x: f64, y: f64) {
    send(RawInput::Motion { x, y });
}

/// button: 0 левая, 1 правая, 2 средняя, 3 назад, 4 вперёд
#[unsafe(no_mangle)]
pub extern "C" fn cosmic_pointer_button(button: u32, pressed: bool) {
    send(RawInput::Button { button, pressed });
}

#[unsafe(no_mangle)]
pub extern "C" fn cosmic_scroll(dy: f64) {
    send(RawInput::Scroll { dy });
}

/// kind: 0 down, 1 move, 2 up, 3 cancel
#[unsafe(no_mangle)]
pub extern "C" fn cosmic_touch(kind: u32, id: u64, x: f64, y: f64) {
    send(RawInput::Touch { kind, id, x, y });
}

/// code — evdev-код клавиши (KEY_*), не Android keycode
#[unsafe(no_mangle)]
pub extern "C" fn cosmic_key(code: u32, pressed: bool) {
    send(RawInput::Key { code, pressed });
}
