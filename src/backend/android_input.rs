use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicalPosition<T> {
    pub x: T,
    pub y: T,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseScrollDelta {
    LineDelta(f32, f32),
    PixelDelta(PhysicalPosition<f64>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AndroidMouseButton {
    Left,
    Right,
    Middle,
    Forward,
    Back,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementState {
    Pressed,
    Released,
}

use smithay::backend::input::{
    self, AbsolutePositionEvent, Axis, AxisRelativeDirection, AxisSource, ButtonState, Device,
    DeviceCapability, Event, InputBackend, InputTime, KeyState,
    KeyboardKeyEvent, Keycode, PointerAxisEvent, PointerButtonEvent, PointerMotionAbsoluteEvent,
    TouchCancelEvent, TouchDownEvent, TouchEvent, TouchMotionEvent, TouchSlot, TouchUpEvent, UnusedEvent,
};

/// Marker used to define the `InputBackend` types for the android backend.
#[derive(Debug)]
pub struct AndroidInput;

/// Virtual input device used by the backend to associate input events
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct AndroidVirtualDevice;

impl Device for AndroidVirtualDevice {
    fn id(&self) -> String {
        String::from("android")
    }

    fn name(&self) -> String {
        String::from("android virtual input")
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(
            capability,
            DeviceCapability::Keyboard
                | DeviceCapability::Pointer
                | DeviceCapability::Touch
                | DeviceCapability::Gesture
        )
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<PathBuf> {
        None
    }
}

/// Android-Backend internal event wrapping `android`'s types into a [`KeyboardKeyEvent`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AndroidKeyboardInputEvent {
    pub time: InputTime,
    pub key: u32,
    pub count: u32,
    pub state: ElementState,
}

impl Event<AndroidInput> for AndroidKeyboardInputEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> AndroidVirtualDevice {
        AndroidVirtualDevice
    }
}

impl KeyboardKeyEvent<AndroidInput> for AndroidKeyboardInputEvent {
    fn key_code(&self) -> Keycode {
        (self.key + 8).into()
    }

    fn state(&self) -> KeyState {
        self.state.into()
    }

    fn count(&self) -> u32 {
        self.count
    }
}

/// Android-Backend internal event wrapping `android`'s types into a [`PointerMotionAbsoluteEvent`]
#[derive(Debug, Clone)]
pub struct AndroidMouseMovedEvent {
    pub time: InputTime,
    pub position: RelativePosition,
    pub global_position: PhysicalPosition<f64>,
}

impl Event<AndroidInput> for AndroidMouseMovedEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> AndroidVirtualDevice {
        AndroidVirtualDevice
    }
}

impl PointerMotionAbsoluteEvent<AndroidInput> for AndroidMouseMovedEvent {}
impl AbsolutePositionEvent<AndroidInput> for AndroidMouseMovedEvent {
    fn x(&self) -> f64 {
        self.global_position.x
    }

    fn y(&self) -> f64 {
        self.global_position.y
    }

    fn x_transformed(&self, width: i32) -> f64 {
        f64::max(self.position.x * width as f64, 0.0)
    }

    fn y_transformed(&self, height: i32) -> f64 {
        f64::max(self.position.y * height as f64, 0.0)
    }
}

/// Android-Backend internal event wrapping `android`'s types into a [`PointerAxisEvent`]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AndroidMouseWheelEvent {
    pub time: InputTime,
    pub delta: MouseScrollDelta,
}

impl Event<AndroidInput> for AndroidMouseWheelEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> AndroidVirtualDevice {
        AndroidVirtualDevice
    }
}

impl PointerAxisEvent<AndroidInput> for AndroidMouseWheelEvent {
    fn source(&self) -> AxisSource {
        match self.delta {
            MouseScrollDelta::LineDelta(_, _) => AxisSource::Wheel,
            MouseScrollDelta::PixelDelta(_) => AxisSource::Continuous,
            _ => AxisSource::Continuous,
        }
    }

    fn amount(&self, axis: Axis) -> Option<f64> {
        match (axis, self.delta) {
            (Axis::Horizontal, MouseScrollDelta::PixelDelta(delta)) => Some(-delta.x),
            (Axis::Vertical, MouseScrollDelta::PixelDelta(delta)) => Some(-delta.y),
            _ => None,
        }
    }

    // TODO: Use high-res scroll where backend supports it
    fn amount_v120(&self, axis: Axis) -> Option<f64> {
        match (axis, self.delta) {
            (Axis::Horizontal, MouseScrollDelta::LineDelta(x, _)) => Some(-x as f64 * 120.),
            (Axis::Vertical, MouseScrollDelta::LineDelta(_, y)) => Some(-y as f64 * 120.),
            _ => None,
        }
    }

    // TODO: Implement with Wayland if `wl_pointer` version >= 9
    fn relative_direction(&self, _axis: Axis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}

/// Android-Backend internal event wrapping `android`'s types into a [`PointerButtonEvent`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AndroidMouseInputEvent {
    pub time: InputTime,
    pub button: AndroidMouseButton,
    pub state: ElementState,
    pub is_x11: bool,
}

impl Event<AndroidInput> for AndroidMouseInputEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> AndroidVirtualDevice {
        AndroidVirtualDevice
    }
}

impl PointerButtonEvent<AndroidInput> for AndroidMouseInputEvent {
    fn button_code(&self) -> u32 {
        match self.button {
            AndroidMouseButton::Left => 0x110,
            AndroidMouseButton::Right => 0x111,
            AndroidMouseButton::Middle => 0x112,
            AndroidMouseButton::Forward => 0x115,
            AndroidMouseButton::Back => 0x116,
        }
    }

    fn state(&self) -> ButtonState {
        self.state.into()
    }
}

/// Android-Backend internal event wrapping `android`'s types into a [`TouchDownEvent`]
#[derive(Debug, Clone)]
pub struct AndroidTouchStartedEvent {
    pub time: InputTime,
    pub position: RelativePosition,
    pub global_position: PhysicalPosition<f64>,
    pub id: u64,
}

impl Event<AndroidInput> for AndroidTouchStartedEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> AndroidVirtualDevice {
        AndroidVirtualDevice
    }
}

impl TouchDownEvent<AndroidInput> for AndroidTouchStartedEvent {}

impl TouchEvent<AndroidInput> for AndroidTouchStartedEvent {
    fn slot(&self) -> TouchSlot {
        Some(self.id as u32).into()
    }
}

impl AbsolutePositionEvent<AndroidInput> for AndroidTouchStartedEvent {
    fn x(&self) -> f64 {
        self.global_position.x
    }

    fn y(&self) -> f64 {
        self.global_position.y
    }

    fn x_transformed(&self, width: i32) -> f64 {
        f64::max(self.position.x * width as f64, 0.0)
    }

    fn y_transformed(&self, height: i32) -> f64 {
        f64::max(self.position.y * height as f64, 0.0)
    }
}

/// Android-Backend internal event wrapping `android`'s types into a [`TouchMotionEvent`]
#[derive(Debug, Clone)]
pub struct AndroidTouchMovedEvent {
    pub time: InputTime,
    pub position: RelativePosition,
    pub global_position: PhysicalPosition<f64>,
    pub id: u64,
}

impl Event<AndroidInput> for AndroidTouchMovedEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> AndroidVirtualDevice {
        AndroidVirtualDevice
    }
}

impl TouchMotionEvent<AndroidInput> for AndroidTouchMovedEvent {}

impl TouchEvent<AndroidInput> for AndroidTouchMovedEvent {
    fn slot(&self) -> TouchSlot {
        Some(self.id as u32).into()
    }
}

impl AbsolutePositionEvent<AndroidInput> for AndroidTouchMovedEvent {
    fn x(&self) -> f64 {
        self.global_position.x
    }

    fn y(&self) -> f64 {
        self.global_position.y
    }

    fn x_transformed(&self, width: i32) -> f64 {
        f64::max(self.position.x * width as f64, 0.0)
    }

    fn y_transformed(&self, height: i32) -> f64 {
        f64::max(self.position.y * height as f64, 0.0)
    }
}

/// Android-Backend internal event wrapping `android`'s types into a [`TouchUpEvent`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AndroidTouchEndedEvent {
    pub time: InputTime,
    pub id: u64,
}

impl Event<AndroidInput> for AndroidTouchEndedEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> AndroidVirtualDevice {
        AndroidVirtualDevice
    }
}

impl TouchUpEvent<AndroidInput> for AndroidTouchEndedEvent {}

impl TouchEvent<AndroidInput> for AndroidTouchEndedEvent {
    fn slot(&self) -> TouchSlot {
        Some(self.id as u32).into()
    }
}

/// Android-Backend internal event wrapping `android`'s types into a [`TouchCancelEvent`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AndroidTouchCancelledEvent {
    pub time: InputTime,
    pub id: u64,
}

impl Event<AndroidInput> for AndroidTouchCancelledEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> AndroidVirtualDevice {
        AndroidVirtualDevice
    }
}

impl TouchCancelEvent<AndroidInput> for AndroidTouchCancelledEvent {}

impl TouchEvent<AndroidInput> for AndroidTouchCancelledEvent {
    fn slot(&self) -> TouchSlot {
        Some(self.id as u32).into()
    }
}

impl From<ElementState> for KeyState {
    #[inline]
    fn from(state: ElementState) -> Self {
        match state {
            ElementState::Pressed => KeyState::Pressed,
            ElementState::Released => KeyState::Released,
        }
    }
}

impl From<ElementState> for ButtonState {
    #[inline]
    fn from(state: ElementState) -> Self {
        match state {
            ElementState::Pressed => ButtonState::Pressed,
            ElementState::Released => ButtonState::Released,
        }
    }
}

/// Position relative to the source window, so each coordinate lays inside
/// the range from [0;1].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelativePosition {
    /// Position of the `x` relative to the window.
    x: f64,
    /// Position of the `y` relative to the window.
    y: f64,
}

impl RelativePosition {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

impl InputBackend for AndroidInput {
    type Device = AndroidVirtualDevice;
    type KeyboardKeyEvent = AndroidKeyboardInputEvent;
    type PointerAxisEvent = AndroidMouseWheelEvent;
    type PointerButtonEvent = AndroidMouseInputEvent;
    type PointerMotionEvent = UnusedEvent;
    type PointerMotionAbsoluteEvent = AndroidMouseMovedEvent;

    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;

    type TouchDownEvent = AndroidTouchStartedEvent;
    type TouchUpEvent = AndroidTouchEndedEvent;
    type TouchMotionEvent = AndroidTouchMovedEvent;
    type TouchCancelEvent = AndroidTouchCancelledEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;

    type SwitchToggleEvent = UnusedEvent;

    type SpecialEvent = UnusedEvent;
}
