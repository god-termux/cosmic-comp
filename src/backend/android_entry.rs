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
use std::{ffi::c_void, sync::Mutex};

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
    super::android_gl::set_window(window, refresh_mhz);
    // Wayland-сокет композитора и конфиги берутся из окружения процесса приложения (XDG_RUNTIME_DIR и т.д.)
    match crate::run(Default::default()) {
        Ok(()) => 0,
        Err(err) => {
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
