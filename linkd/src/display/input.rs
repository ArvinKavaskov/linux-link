//! Injects the tablet's input into the session through /dev/uinput.
//!
//! This is the path for every compositor except GNOME (which has a proper
//! remote-desktop D-Bus API, see `backends::MutterSession`). Two virtual
//! devices are created: an absolute pointer and a keyboard. Absolute pointer
//! devices — the same species as a VM's tablet mouse — get mapped by libinput
//! across the whole output layout, which is exactly the property we need: we
//! know where the virtual monitor sits in that layout, so a normalized tablet
//! coordinate becomes one absolute position, no grabs, no focus games.
//!
//! Finger touches arrive raw and are translated here rather than on the
//! tablet: one finger drives the pointer (touch = click, like any touchscreen),
//! two fingers scroll. Keeping the translation on the PC means the Android
//! side stays a dumb sensor and future backends (GNOME) can consume the same
//! raw events natively.

use anyhow::{Context, Result};
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode,
    UinputAbsSetup,
};
use std::collections::HashMap;

use super::{RemoteInput, Rect};

const ABS_RANGE: i32 = 65535;
/// Finger travel (in desktop pixels) worth one wheel detent.
const SCROLL_STEP: f64 = 18.0;

pub struct UinputSink {
    pointer: VirtualDevice,
    keyboard: VirtualDevice,
    /// Where the tablet's monitor sits in the desktop, and how big the whole
    /// desktop is — the two rectangles that turn a normalized coordinate into
    /// an absolute one.
    monitor: Rect,
    desktop: Rect,
    touch: TouchTranslator,
}

impl UinputSink {
    pub fn new(monitor: Rect, desktop: Rect) -> Result<Self> {
        let mut buttons = AttributeSet::<KeyCode>::new();
        for b in [KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT, KeyCode::BTN_MIDDLE] {
            buttons.insert(b);
        }
        let mut rels = AttributeSet::<RelativeAxisCode>::new();
        rels.insert(RelativeAxisCode::REL_WHEEL);
        rels.insert(RelativeAxisCode::REL_HWHEEL);

        let abs = AbsInfo::new(0, 0, ABS_RANGE, 0, 0, 0);
        let pointer = VirtualDevice::builder()
            .context("open /dev/uinput — is the udev rule installed? (install.sh sets it up)")?
            .name("Linux Link Second Screen Pointer")
            .with_keys(&buttons)?
            .with_relative_axes(&rels)?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, abs))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, abs))?
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisCode::ABS_PRESSURE,
                AbsInfo::new(0, 0, 4095, 0, 0, 0),
            ))?
            .build()?;

        // Every ordinary key: 1 (ESC) through 248 covers the whole keyboard
        // block including media keys, without the buttons range.
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in 1..=248u16 {
            keys.insert(KeyCode::new(code));
        }
        let keyboard = VirtualDevice::builder()?
            .name("Linux Link Second Screen Keyboard")
            .with_keys(&keys)?
            .build()?;

        let touch = TouchTranslator::with_scale(monitor.w, monitor.h);
        Ok(Self { pointer, keyboard, monitor, desktop, touch })
    }

    pub fn handle(&mut self, ev: &RemoteInput) -> Result<()> {
        match *ev {
            RemoteInput::Move { x, y } => self.move_abs(x, y)?,
            RemoteInput::Button { b, d } => self.button(b, d)?,
            RemoteInput::Scroll { dx, dy, .. } => self.wheel(dx, dy)?,
            RemoteInput::Key { c, d } => {
                self.keyboard
                    .emit(&[InputEvent::new(EventType::KEY.0, c, i32::from(d))])?;
            }
            RemoteInput::Pen { x, y, p, d } => {
                let (ax, ay) = self.to_abs(x, y);
                let pressure = (p.clamp(0.0, 1.0) * 4095.0) as i32;
                self.pointer.emit(&[
                    InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_X.0, ax),
                    InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_Y.0, ay),
                    InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_PRESSURE.0, pressure),
                    InputEvent::new(EventType::KEY.0, KeyCode::BTN_LEFT.0, i32::from(d)),
                ])?;
            }
            RemoteInput::Touch { id, ph, x, y } => {
                for action in self.touch.push(id, ph, x, y) {
                    match action {
                        TouchAction::Move(u, v) => self.move_abs(u, v)?,
                        TouchAction::Button(down) => self.button(0, down)?,
                        TouchAction::Wheel(dx, dy) => self.wheel(dx, dy)?,
                    }
                }
            }
        }
        Ok(())
    }

    fn to_abs(&self, u: f64, v: f64) -> (i32, i32) {
        let x = self.monitor.x + u.clamp(0.0, 1.0) * self.monitor.w;
        let y = self.monitor.y + v.clamp(0.0, 1.0) * self.monitor.h;
        (
            (x / self.desktop.w.max(1.0) * f64::from(ABS_RANGE)) as i32,
            (y / self.desktop.h.max(1.0) * f64::from(ABS_RANGE)) as i32,
        )
    }

    fn move_abs(&mut self, u: f64, v: f64) -> Result<()> {
        let (ax, ay) = self.to_abs(u, v);
        self.pointer.emit(&[
            InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_X.0, ax),
            InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_Y.0, ay),
        ])?;
        Ok(())
    }

    fn button(&mut self, b: u8, down: bool) -> Result<()> {
        let code = match b {
            0 => KeyCode::BTN_LEFT,
            1 => KeyCode::BTN_RIGHT,
            _ => KeyCode::BTN_MIDDLE,
        };
        self.pointer
            .emit(&[InputEvent::new(EventType::KEY.0, code.0, i32::from(down))])?;
        Ok(())
    }

    fn wheel(&mut self, dx: f64, dy: f64) -> Result<()> {
        let mut events = Vec::new();
        if dy.abs() >= 1.0 {
            events.push(InputEvent::new(
                EventType::RELATIVE.0,
                RelativeAxisCode::REL_WHEEL.0,
                // Finger moves down → content follows the finger → wheel up.
                -dy.signum() as i32,
            ));
        }
        if dx.abs() >= 1.0 {
            events.push(InputEvent::new(
                EventType::RELATIVE.0,
                RelativeAxisCode::REL_HWHEEL.0,
                -dx.signum() as i32,
            ));
        }
        if !events.is_empty() {
            self.pointer.emit(&events)?;
        }
        Ok(())
    }
}

/// What a raw touch event should become on a pointer-only device.
#[derive(Debug, PartialEq)]
pub enum TouchAction {
    Move(f64, f64),
    Button(bool),
    Wheel(f64, f64),
}

/// One finger is a pointer with the button held; two fingers scroll. The
/// logic is pure (no device handle) so it can be unit-tested.
#[derive(Default)]
pub struct TouchTranslator {
    slots: HashMap<u32, (f64, f64)>,
    dragging: bool,
    /// Accumulated two-finger travel, in normalized units scaled by SCROLL_STEP
    /// at consumption time.
    scroll_acc: (f64, f64),
    last_centroid: Option<(f64, f64)>,
    /// Approximate size of the tablet monitor, used to turn normalized deltas
    /// into something like pixels for the scroll threshold.
    scale: (f64, f64),
}

impl TouchTranslator {
    pub fn with_scale(w: f64, h: f64) -> Self {
        Self { scale: (w, h), ..Default::default() }
    }

    /// ph: 0 = down, 1 = move, 2 = up, 3 = cancel.
    pub fn push(&mut self, id: u32, ph: u8, x: f64, y: f64) -> Vec<TouchAction> {
        let (sw, sh) = if self.scale.0 > 0.0 { self.scale } else { (1280.0, 800.0) };
        let mut out = Vec::new();
        match ph {
            0 => {
                self.slots.insert(id, (x, y));
                match self.slots.len() {
                    1 => {
                        out.push(TouchAction::Move(x, y));
                        out.push(TouchAction::Button(true));
                        self.dragging = true;
                    }
                    2 => {
                        // The second finger turns a drag into a scroll.
                        if self.dragging {
                            out.push(TouchAction::Button(false));
                            self.dragging = false;
                        }
                        self.last_centroid = Some(self.centroid());
                        self.scroll_acc = (0.0, 0.0);
                    }
                    _ => {}
                }
            }
            1 => {
                if self.slots.contains_key(&id) {
                    self.slots.insert(id, (x, y));
                }
                if self.dragging && self.slots.len() == 1 {
                    out.push(TouchAction::Move(x, y));
                } else if self.slots.len() == 2 {
                    let c = self.centroid();
                    if let Some(prev) = self.last_centroid {
                        self.scroll_acc.0 += (c.0 - prev.0) * sw;
                        self.scroll_acc.1 += (c.1 - prev.1) * sh;
                        while self.scroll_acc.1.abs() >= SCROLL_STEP {
                            let s = self.scroll_acc.1.signum();
                            out.push(TouchAction::Wheel(0.0, s * SCROLL_STEP));
                            self.scroll_acc.1 -= s * SCROLL_STEP;
                        }
                        while self.scroll_acc.0.abs() >= SCROLL_STEP {
                            let s = self.scroll_acc.0.signum();
                            out.push(TouchAction::Wheel(s * SCROLL_STEP, 0.0));
                            self.scroll_acc.0 -= s * SCROLL_STEP;
                        }
                    }
                    self.last_centroid = Some(c);
                }
            }
            _ => {
                self.slots.remove(&id);
                if self.slots.is_empty() {
                    if self.dragging {
                        out.push(TouchAction::Button(false));
                        self.dragging = false;
                    }
                    self.last_centroid = None;
                } else if self.slots.len() == 1 {
                    self.last_centroid = None;
                }
            }
        }
        out
    }

    fn centroid(&self) -> (f64, f64) {
        let n = self.slots.len().max(1) as f64;
        let (sx, sy) = self
            .slots
            .values()
            .fold((0.0, 0.0), |(ax, ay), (x, y)| (ax + x, ay + y));
        (sx / n, sy / n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_finger_is_a_click_and_drag() {
        let mut t = TouchTranslator::with_scale(1000.0, 1000.0);
        let down = t.push(1, 0, 0.5, 0.5);
        assert_eq!(down, vec![TouchAction::Move(0.5, 0.5), TouchAction::Button(true)]);
        let mv = t.push(1, 1, 0.6, 0.5);
        assert_eq!(mv, vec![TouchAction::Move(0.6, 0.5)]);
        let up = t.push(1, 2, 0.6, 0.5);
        assert_eq!(up, vec![TouchAction::Button(false)]);
    }

    #[test]
    fn second_finger_releases_the_drag_and_scrolls() {
        let mut t = TouchTranslator::with_scale(1000.0, 1000.0);
        t.push(1, 0, 0.5, 0.5);
        let second = t.push(2, 0, 0.5, 0.6);
        assert_eq!(second, vec![TouchAction::Button(false)]);
        // Both fingers travel 5% of the screen down → several wheel steps.
        let a = t.push(1, 1, 0.5, 0.55);
        let b = t.push(2, 1, 0.5, 0.65);
        let wheels = a.iter().chain(b.iter()).filter(|x| matches!(x, TouchAction::Wheel(..))).count();
        assert!(wheels >= 2, "expected wheel events, got {a:?} {b:?}");
    }

    #[test]
    fn cancel_clears_everything() {
        let mut t = TouchTranslator::with_scale(1000.0, 1000.0);
        t.push(1, 0, 0.1, 0.1);
        let cancel = t.push(1, 3, 0.1, 0.1);
        assert_eq!(cancel, vec![TouchAction::Button(false)]);
        assert!(t.slots.is_empty());
    }
}
