
use anyhow::{Context, Result};
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, EventType, InputEvent, KeyCode, PropType,
    RelativeAxisCode, UinputAbsSetup,
};
use std::collections::HashMap;

use super::{RemoteInput, Rect};

const ABS_RANGE: i32 = 65535;
const SCROLL_STEP: f64 = 18.0;
const PRESSURE_MAX: i32 = 4095;
const TILT_LIMIT: i32 = 90;

pub struct UinputSink {
    pointer: VirtualDevice,
    keyboard: VirtualDevice,
    pen: VirtualDevice,
    pen_tool: Option<KeyCode>,
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
            .build()?;

        let pen = build_pen()?;

        let mut keys = AttributeSet::<KeyCode>::new();
        for code in 1..=248u16 {
            keys.insert(KeyCode::new(code));
        }
        let keyboard = VirtualDevice::builder()?
            .name("Linux Link Second Screen Keyboard")
            .with_keys(&keys)?
            .build()?;

        let touch = TouchTranslator::with_scale(monitor.w, monitor.h);
        Ok(Self { pointer, keyboard, pen, pen_tool: None, monitor, desktop, touch })
    }

    pub fn handle(&mut self, ev: &RemoteInput, now: u64) -> Result<()> {
        match *ev {
            RemoteInput::Move { x, y } => self.move_abs(x, y)?,
            RemoteInput::Button { b, d } => self.button(b, d)?,
            RemoteInput::Scroll { dx, dy, .. } => self.wheel(dx, dy)?,
            RemoteInput::Key { c, d } => {
                self.keyboard
                    .emit(&[InputEvent::new(EventType::KEY.0, c, i32::from(d))])?;
            }
            RemoteInput::Pen { x, y, p, d, tx, ty, e, bar, prox } => {
                self.pen_event(x, y, p, d, tx, ty, e, bar, prox)?
            }
            RemoteInput::Touch { id, ph, x, y } => {
                let actions = self.touch.push(id, ph, x, y, now);
                self.apply(actions)?;
            }
        }
        Ok(())
    }

    pub fn tick(&mut self, now: u64) -> Result<()> {
        let actions = self.touch.tick(now);
        self.apply(actions)
    }

    fn apply(&mut self, actions: Vec<TouchAction>) -> Result<()> {
        for action in actions {
            match action {
                TouchAction::Move(u, v) => self.move_abs(u, v)?,
                TouchAction::Button(down) => self.button(0, down)?,
                TouchAction::RightClick => {
                    self.button(1, true)?;
                    self.button(1, false)?;
                }
                TouchAction::Wheel(dx, dy) => self.wheel(dx, dy)?,
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

    #[allow(clippy::too_many_arguments)]
    fn pen_event(
        &mut self,
        x: f64,
        y: f64,
        p: f64,
        down: bool,
        tilt_x: f64,
        tilt_y: f64,
        eraser: bool,
        barrel: bool,
        in_range: bool,
    ) -> Result<()> {
        let wanted = if !in_range {
            None
        } else if eraser {
            Some(KeyCode::BTN_TOOL_RUBBER)
        } else {
            Some(KeyCode::BTN_TOOL_PEN)
        };

        if self.pen_tool != wanted {
            if let Some(old) = self.pen_tool.take() {
                self.pen.emit(&[
                    InputEvent::new(EventType::KEY.0, KeyCode::BTN_TOUCH.0, 0),
                    InputEvent::new(EventType::KEY.0, KeyCode::BTN_STYLUS.0, 0),
                    InputEvent::new(EventType::KEY.0, old.0, 0),
                ])?;
            }
            if let Some(new) = wanted {
                self.pen
                    .emit(&[InputEvent::new(EventType::KEY.0, new.0, 1)])?;
            }
            self.pen_tool = wanted;
        }
        if wanted.is_none() {
            return Ok(());
        }

        let (ax, ay) = self.to_abs(x, y);
        let pressure = (p.clamp(0.0, 1.0) * f64::from(PRESSURE_MAX)) as i32;
        let limit = f64::from(TILT_LIMIT);
        self.pen.emit(&[
            InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_X.0, ax),
            InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_Y.0, ay),
            InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_PRESSURE.0, pressure),
            InputEvent::new(
                EventType::ABSOLUTE.0,
                AbsoluteAxisCode::ABS_TILT_X.0,
                tilt_x.clamp(-limit, limit) as i32,
            ),
            InputEvent::new(
                EventType::ABSOLUTE.0,
                AbsoluteAxisCode::ABS_TILT_Y.0,
                tilt_y.clamp(-limit, limit) as i32,
            ),
            InputEvent::new(
                EventType::ABSOLUTE.0,
                AbsoluteAxisCode::ABS_DISTANCE.0,
                i32::from(!down),
            ),
            InputEvent::new(EventType::KEY.0, KeyCode::BTN_TOUCH.0, i32::from(down)),
            InputEvent::new(EventType::KEY.0, KeyCode::BTN_STYLUS.0, i32::from(barrel)),
        ])?;
        Ok(())
    }

    fn wheel(&mut self, dx: f64, dy: f64) -> Result<()> {
        let mut events = Vec::new();
        if dy.abs() >= 1.0 {
            events.push(InputEvent::new(
                EventType::RELATIVE.0,
                RelativeAxisCode::REL_WHEEL.0,
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

fn build_pen() -> Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for k in [
        KeyCode::BTN_TOOL_PEN,
        KeyCode::BTN_TOOL_RUBBER,
        KeyCode::BTN_TOUCH,
        KeyCode::BTN_STYLUS,
    ] {
        keys.insert(k);
    }
    let mut props = AttributeSet::<PropType>::new();
    props.insert(PropType::DIRECT);

    let abs = AbsInfo::new(0, 0, ABS_RANGE, 0, 0, 0);
    let tilt = AbsInfo::new(0, -TILT_LIMIT, TILT_LIMIT, 0, 0, 0);
    VirtualDevice::builder()
        .context("open /dev/uinput for the pen")?
        .name("Linux Link Second Screen Pen")
        .with_keys(&keys)?
        .with_properties(&props)?
        .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, abs))?
        .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, abs))?
        .with_absolute_axis(&UinputAbsSetup::new(
            AbsoluteAxisCode::ABS_PRESSURE,
            AbsInfo::new(0, 0, PRESSURE_MAX, 0, 0, 0),
        ))?
        .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_TILT_X, tilt))?
        .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_TILT_Y, tilt))?
        .with_absolute_axis(&UinputAbsSetup::new(
            AbsoluteAxisCode::ABS_DISTANCE,
            AbsInfo::new(0, 0, 1, 0, 0, 0),
        ))?
        .build()
        .context("create the virtual pen")
}

#[derive(Debug, PartialEq)]
pub enum TouchAction {
    Move(f64, f64),
    Button(bool),
    RightClick,
    Wheel(f64, f64),
}
#[derive(Default)]
pub struct TouchTranslator {
    slots: HashMap<u32, (f64, f64)>,
    dragging: bool,
    scroll_acc: (f64, f64),
    last_centroid: Option<(f64, f64)>,
    scale: (f64, f64),
    press: Option<(u64, f64, f64)>,
    two_at: Option<u64>,
    scrolled: bool,
}

const LONG_PRESS_MS: u64 = 500;
const LONG_PRESS_SLOP: f64 = 12.0;
const TWO_FINGER_TAP_MS: u64 = 250;

impl TouchTranslator {
    pub fn with_scale(w: f64, h: f64) -> Self {
        Self { scale: (w, h), ..Default::default() }
    }

    fn px(&self) -> (f64, f64) {
        if self.scale.0 > 0.0 {
            self.scale
        } else {
            (1280.0, 800.0)
        }
    }

    pub fn push(&mut self, id: u32, ph: u8, x: f64, y: f64, now: u64) -> Vec<TouchAction> {
        let (sw, sh) = self.px();
        let mut out = Vec::new();
        match ph {
            0 => {
                self.slots.insert(id, (x, y));
                match self.slots.len() {
                    1 => {
                        out.push(TouchAction::Move(x, y));
                        out.push(TouchAction::Button(true));
                        self.dragging = true;
                        self.press = Some((now, x, y));
                    }
                    2 => {
                        if self.dragging {
                            out.push(TouchAction::Button(false));
                            self.dragging = false;
                        }
                        self.press = None;
                        self.last_centroid = Some(self.centroid());
                        self.scroll_acc = (0.0, 0.0);
                        self.two_at = Some(now);
                        self.scrolled = false;
                    }
                    _ => {
                        self.press = None;
                        self.two_at = None;
                    }
                }
            }
            1 => {
                if self.slots.contains_key(&id) {
                    self.slots.insert(id, (x, y));
                }
                if self.dragging && self.slots.len() == 1 {
                    out.push(TouchAction::Move(x, y));
                    if let Some((_, ox, oy)) = self.press {
                        let moved = ((x - ox) * sw).hypot((y - oy) * sh);
                        if moved > LONG_PRESS_SLOP {
                            self.press = None;
                        }
                    }
                } else if self.slots.len() == 2 {
                    let c = self.centroid();
                    if let Some(prev) = self.last_centroid {
                        self.scroll_acc.0 += (c.0 - prev.0) * sw;
                        self.scroll_acc.1 += (c.1 - prev.1) * sh;
                        while self.scroll_acc.1.abs() >= SCROLL_STEP {
                            let s = self.scroll_acc.1.signum();
                            out.push(TouchAction::Wheel(0.0, s * SCROLL_STEP));
                            self.scroll_acc.1 -= s * SCROLL_STEP;
                            self.scrolled = true;
                        }
                        while self.scroll_acc.0.abs() >= SCROLL_STEP {
                            let s = self.scroll_acc.0.signum();
                            out.push(TouchAction::Wheel(s * SCROLL_STEP, 0.0));
                            self.scroll_acc.0 -= s * SCROLL_STEP;
                            self.scrolled = true;
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
                    let tapped = self
                        .two_at
                        .is_some_and(|t| now.saturating_sub(t) <= TWO_FINGER_TAP_MS)
                        && !self.scrolled;
                    if tapped {
                        out.push(TouchAction::RightClick);
                    }
                    self.last_centroid = None;
                    self.press = None;
                    self.two_at = None;
                    self.scrolled = false;
                } else if self.slots.len() == 1 {
                    self.last_centroid = None;
                }
            }
        }
        out
    }

    pub fn tick(&mut self, now: u64) -> Vec<TouchAction> {
        let Some((at, _, _)) = self.press else {
            return Vec::new();
        };
        if self.slots.len() != 1 || !self.dragging || now.saturating_sub(at) < LONG_PRESS_MS {
            return Vec::new();
        }
        self.press = None;
        self.dragging = false;
        vec![TouchAction::Button(false), TouchAction::RightClick]
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
        let down = t.push(1, 0, 0.5, 0.5, 0);
        assert_eq!(down, vec![TouchAction::Move(0.5, 0.5), TouchAction::Button(true)]);
        let mv = t.push(1, 1, 0.6, 0.5, 10);
        assert_eq!(mv, vec![TouchAction::Move(0.6, 0.5)]);
        let up = t.push(1, 2, 0.6, 0.5, 20);
        assert_eq!(up, vec![TouchAction::Button(false)]);
    }

    #[test]
    fn second_finger_releases_the_drag_and_scrolls() {
        let mut t = TouchTranslator::with_scale(1000.0, 1000.0);
        t.push(1, 0, 0.5, 0.5, 0);
        let second = t.push(2, 0, 0.5, 0.6, 5);
        assert_eq!(second, vec![TouchAction::Button(false)]);
        let a = t.push(1, 1, 0.5, 0.55, 20);
        let b = t.push(2, 1, 0.5, 0.65, 25);
        let wheels = a.iter().chain(b.iter()).filter(|x| matches!(x, TouchAction::Wheel(..))).count();
        assert!(wheels >= 2, "expected wheel events, got {a:?} {b:?}");
    }

    #[test]
    fn cancel_clears_everything() {
        let mut t = TouchTranslator::with_scale(1000.0, 1000.0);
        t.push(1, 0, 0.1, 0.1, 0);
        let cancel = t.push(1, 3, 0.1, 0.1, 5);
        assert_eq!(cancel, vec![TouchAction::Button(false)]);
        assert!(t.slots.is_empty());
    }

    #[test]
    fn a_held_finger_becomes_a_right_click() {
        let mut t = TouchTranslator::with_scale(1000.0, 1000.0);
        t.push(1, 0, 0.4, 0.4, 0);
        assert!(t.tick(LONG_PRESS_MS - 1).is_empty(), "fired too early");
        let fired = t.tick(LONG_PRESS_MS);
        assert_eq!(fired, vec![TouchAction::Button(false), TouchAction::RightClick]);
        assert!(t.tick(LONG_PRESS_MS + 500).is_empty());
        assert!(t.push(1, 2, 0.4, 0.4, LONG_PRESS_MS + 600).is_empty());
    }

    #[test]
    fn a_dragging_finger_never_long_presses() {
        let mut t = TouchTranslator::with_scale(1000.0, 1000.0);
        t.push(1, 0, 0.4, 0.4, 0);
        t.push(1, 1, 0.45, 0.4, 100);
        assert!(t.tick(LONG_PRESS_MS + 100).is_empty());
    }

    #[test]
    fn two_finger_tap_is_a_right_click_but_a_scroll_is_not() {
        let mut t = TouchTranslator::with_scale(1000.0, 1000.0);
        t.push(1, 0, 0.5, 0.5, 0);
        t.push(2, 0, 0.55, 0.5, 10);
        t.push(1, 2, 0.5, 0.5, 60);
        let up = t.push(2, 2, 0.55, 0.5, 70);
        assert_eq!(up, vec![TouchAction::RightClick]);

        let mut t = TouchTranslator::with_scale(1000.0, 1000.0);
        t.push(1, 0, 0.5, 0.5, 0);
        t.push(2, 0, 0.55, 0.5, 10);
        t.push(1, 1, 0.5, 0.6, 30);
        t.push(2, 1, 0.55, 0.6, 35);
        t.push(1, 2, 0.5, 0.6, 60);
        let up = t.push(2, 2, 0.55, 0.6, 70);
        assert!(!up.contains(&TouchAction::RightClick), "a scroll is not a tap");
    }

    #[test]
    fn a_slow_two_finger_lift_is_not_a_tap() {
        let mut t = TouchTranslator::with_scale(1000.0, 1000.0);
        t.push(1, 0, 0.5, 0.5, 0);
        t.push(2, 0, 0.55, 0.5, 10);
        t.push(1, 2, 0.5, 0.5, 1_000);
        let up = t.push(2, 2, 0.55, 0.5, 1_010);
        assert!(up.is_empty(), "expected nothing, got {up:?}");
    }
}
