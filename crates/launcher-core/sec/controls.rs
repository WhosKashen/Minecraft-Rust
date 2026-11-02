//! Touch/controller input model: layouts, validation, import/export, alignment guides and the
//! gameplay `InputRouter` (multi-touch, hold/toggle, joystick, camera look). The router is pure
//! logic with no Android dependency; the game activity feeds it normalized touch events and
//! forwards the produced `InputEvent`s to the native input bridge. Layout *editing* lives in the
//! UI and never touches the router, so editing and gameplay input cannot mix.
use crate::error::{Error, Result};
use crate::paths::Dirs;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const SCHEMA: &str = "rustcraft-controls";
pub const SCHEMA_VERSION: u32 = 1;
const CAMERA_BASE: f32 = 1000.0; // virtual pixels per screen height at sensitivity 1.0

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseButton { Left, Right, Middle }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Binding {
    /// Symbolic key name ("W", "SPACE", "LSHIFT", "E", "1", "F5", "ESCAPE", …). The native bridge maps it.
    Key(String),
    Mouse(MouseButton),
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode { Hold, Toggle }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ElementKind {
    Button { binding: Binding, mode: Mode },
    Joystick { deadzone: f32, sprint_threshold: f32 },
    Camera { sensitivity: f32 },
    Hotbar { slots: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool { px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h }
    pub fn cx(&self) -> f32 { self.x + self.w / 2.0 }
    pub fn cy(&self) -> f32 { self.y + self.h / 2.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub id: String,
    pub label: String,
    pub kind: ElementKind,
    /// Normalized (0..1) inside the usable game area (after cutouts and system bars).
    pub rect: Rect,
    pub opacity: f32,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    pub schema: String,
    pub version: u32,
    pub name: String,
    /// Later elements are drawn and hit-tested on top.
    pub elements: Vec<Element>,
}

fn btn(id: &str, label: &str, key: &str, mode: Mode, x: f32, y: f32, w: f32, h: f32) -> Element {
    Element { id: id.into(), label: label.into(), kind: ElementKind::Button { binding: Binding::Key(key.into()), mode }, rect: Rect { x, y, w, h }, opacity: 0.6, visible: true }
}
fn mbtn(id: &str, label: &str, b: MouseButton, x: f32, y: f32, w: f32, h: f32) -> Element {
    Element { id: id.into(), label: label.into(), kind: ElementKind::Button { binding: Binding::Mouse(b), mode: Mode::Hold }, rect: Rect { x, y, w, h }, opacity: 0.6, visible: true }
}

pub fn default_layout() -> Layout {
    let mut e = vec![
        Element { id: "camera".into(), label: "Look".into(), kind: ElementKind::Camera { sensitivity: 1.0 }, rect: Rect { x: 0.35, y: 0.0, w: 0.65, h: 1.0 }, opacity: 0.0, visible: true },
        Element { id: "move".into(), label: "Move".into(), kind: ElementKind::Joystick { deadzone: 0.25, sprint_threshold: 0.9 }, rect: Rect { x: 0.03, y: 0.52, w: 0.17, h: 0.44 }, opacity: 0.55, visible: true },
        mbtn("attack", "Attack", MouseButton::Left, 0.80, 0.55, 0.10, 0.22),
        mbtn("use", "Use", MouseButton::Right, 0.70, 0.62, 0.10, 0.22),
        btn("jump", "Jump", "SPACE", Mode::Hold, 0.88, 0.30, 0.10, 0.22),
        btn("sneak", "Sneak", "LSHIFT", Mode::Toggle, 0.24, 0.78, 0.09, 0.18),
        btn("sprint", "Sprint", "LCTRL", Mode::Toggle, 0.24, 0.56, 0.09, 0.18),
        btn("inventory", "Inv", "E", Mode::Hold, 0.60, 0.04, 0.07, 0.14),
        btn("chat", "Chat", "T", Mode::Hold, 0.68, 0.04, 0.07, 0.14),
        btn("pause", "Pause", "ESCAPE", Mode::Hold, 0.76, 0.04, 0.07, 0.14),
        btn("drop", "Drop", "Q", Mode::Hold, 0.84, 0.04, 0.07, 0.14),
        btn("perspective", "F5", "F5", Mode::Hold, 0.92, 0.04, 0.06, 0.14),
        Element { id: "hotbar".into(), label: "Hotbar".into(), kind: ElementKind::Hotbar { slots: 9 }, rect: Rect { x: 0.36, y: 0.88, w: 0.30, h: 0.11 }, opacity: 0.45, visible: true },
    ];
    for el in e.iter_mut() { el.rect = clamp_rect(el.rect); }
    Layout { schema: SCHEMA.into(), version: SCHEMA_VERSION, name: "Default".into(), elements: e }
}

fn clamp_rect(r: Rect) -> Rect {
    let w = r.w.clamp(0.04, 1.0);
    let h = r.h.clamp(0.04, 1.0);
    Rect { x: r.x.clamp(0.0, 1.0 - w), y: r.y.clamp(0.0, 1.0 - h), w, h }
}

impl Layout {
    pub fn clamp(&mut self) {
        for e in self.elements.iter_mut() {
            e.rect = clamp_rect(e.rect);
            e.opacity = e.opacity.clamp(0.0, 1.0);
            match &mut e.kind {
                ElementKind::Camera { sensitivity } => *sensitivity = sensitivity.clamp(0.1, 5.0),
                ElementKind::Joystick { deadzone, sprint_threshold } => { *deadzone = deadzone.clamp(0.05, 0.6); *sprint_threshold = sprint_threshold.clamp(0.5, 1.0); }
                ElementKind::Hotbar { slots } => *slots = (*slots).clamp(1, 9),
                ElementKind::Button { .. } => {}
            }
        }
    }

    /// Human-readable problems (empty = fine).
    pub fn validate(&self) -> Vec<String> {
        let mut v = Vec::new();
        let mut ids = HashSet::new();
        for e in &self.elements {
            if !ids.insert(&e.id) { v.push(format!("Duplicate control id '{}'.", e.id)); }
        }
        let has = |f: fn(&ElementKind) -> bool| self.elements.iter().any(|e| e.visible && f(&e.kind));
        if !has(|k| matches!(k, ElementKind::Joystick { .. })) && !self.elements.iter().any(|e| e.visible && matches!(&e.kind, ElementKind::Button { binding: Binding::Key(k), .. } if k == "W")) {
            v.push("No movement control (joystick or forward button) is visible.".into());
        }
        if !has(|k| matches!(k, ElementKind::Camera { .. })) { v.push("No camera-look region is visible; you will not be able to turn.".into()); }
        let m = |e: &Element| matches!(&e.kind, ElementKind::Button { binding: Binding::Key(k), .. } if k == "ESCAPE");
        if !self.elements.iter().any(|e| e.visible && m(e)) { v.push("No pause button is visible.".into()); }
        v
    }

    pub fn to_json(&self) -> Result<String> { Ok(serde_json::to_string_pretty(self)?) }

    pub fn from_json(s: &str) -> Result<Layout> {
        let mut l: Layout = serde_json::from_str(s).map_err(|e| Error::invalid(format!("Not a valid controls layout: {e}")))?;
        if l.schema != SCHEMA { return Err(Error::invalid("This file is not a Rustcraft controls layout.")); }
        if l.version > SCHEMA_VERSION { return Err(Error::invalid("This layout was made by a newer version of the launcher.")); }
        l.clamp();
        Ok(l)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide { pub vertical: bool, pub pos: f32 }

/// Snap a rect's edges/centre to the screen edges/centre and to other elements; returns the guides hit.
pub fn snap(mut r: Rect, others: &[Rect], threshold: f32) -> (Rect, Vec<Guide>) {
    let mut guides = Vec::new();
    let mut xs = vec![0.0, 0.5, 1.0];
    let mut ys = vec![0.0, 0.5, 1.0];
    for o in others { xs.extend([o.x, o.cx(), o.x + o.w]); ys.extend([o.y, o.cy(), o.y + o.h]); }
    let try_axis = |lo: f32, size: f32, targets: &[f32]| -> Option<(f32, f32)> {
        let mut best: Option<(f32, f32)> = None; // (delta, guide)
        for t in targets {
            for p in [lo, lo + size / 2.0, lo + size] {
                let d = t - p;
                if d.abs() <= threshold && best.map_or(true, |(bd, _)| d.abs() < bd.abs()) { best = Some((d, *t)); }
            }
        }
        best
    };
    if let Some((d, g)) = try_axis(r.x, r.w, &xs) { r.x += d; guides.push(Guide { vertical: true, pos: g }); }
    if let Some((d, g)) = try_axis(r.y, r.h, &ys) { r.y += d; guides.push(Guide { vertical: false, pos: g }); }
    (clamp_rect(r), guides)
}

// ------------------------------------------------------------------ gameplay router

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchPhase { Down, Move, Up, Cancel }

#[derive(Debug, Clone, Copy)]
pub struct TouchEvent { pub pointer: u32, pub phase: TouchPhase, pub x: f32, pub y: f32 }

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    KeyDown(String), KeyUp(String),
    MouseDown(MouseButton), MouseUp(MouseButton),
    MouseMove { dx: f32, dy: f32 },
    Scroll(i32),
}

enum Capture {
    Button(usize),
    Joystick(usize),
    Camera { idx: usize, last: (f32, f32) },
}

pub struct InputRouter {
    layout: Layout,
    aspect: f32,
    captures: HashMap<u32, Capture>,
    held: HashSet<Binding>,
    toggled: HashSet<String>,
    joy_keys: HashSet<&'static str>,
}

impl InputRouter {
    /// `aspect` = usable width / usable height of the game area.
    pub fn new(layout: Layout, aspect: f32) -> Self {
        Self { layout, aspect: aspect.max(0.1), captures: HashMap::new(), held: HashSet::new(), toggled: HashSet::new(), joy_keys: HashSet::new() }
    }

    fn press(&mut self, b: &Binding, out: &mut Vec<InputEvent>) {
        match b {
            Binding::ScrollUp => out.push(InputEvent::Scroll(1)),
            Binding::ScrollDown => out.push(InputEvent::Scroll(-1)),
            _ => if self.held.insert(b.clone()) {
                out.push(match b { Binding::Key(k) => InputEvent::KeyDown(k.clone()), Binding::Mouse(m) => InputEvent::MouseDown(*m), _ => unreachable!() });
            },
        }
    }
    fn release(&mut self, b: &Binding, out: &mut Vec<InputEvent>) {
        if self.held.remove(b) {
            out.push(match b { Binding::Key(k) => InputEvent::KeyUp(k.clone()), Binding::Mouse(m) => InputEvent::MouseUp(*m), _ => return });
        }
    }

    fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.layout.elements.iter().enumerate().rev().find(|(_, e)| e.visible && e.rect.contains(x, y)).map(|(i, _)| i)
    }

    fn update_joystick(&mut self, idx: usize, x: f32, y: f32, out: &mut Vec<InputEvent>) {
        let ElementKind::Joystick { deadzone, sprint_threshold } = self.layout.elements[idx].kind else { return };
        let r = self.layout.elements[idx].rect;
        let radius = ((r.w * self.aspect).min(r.h) / 2.0).max(0.001);
        let (mut nx, mut ny) = (((x - r.cx()) * self.aspect) / radius, (y - r.cy()) / radius);
        let len = (nx * nx + ny * ny).sqrt();
        if len > 1.0 { nx /= len; ny /= len; }
        let len = len.min(1.0);
        let mut want: HashSet<&'static str> = HashSet::new();
        if ny < -deadzone { want.insert("W"); }
        if ny > deadzone { want.insert("S"); }
        if nx < -deadzone { want.insert("A"); }
        if nx > deadzone { want.insert("D"); }
        if want.contains("W") && len >= sprint_threshold { want.insert("LCTRL"); }
        self.sync_joy(want, out);
    }

    fn sync_joy(&mut self, want: HashSet<&'static str>, out: &mut Vec<InputEvent>) {
        let mut off: Vec<&'static str> = self.joy_keys.difference(&want).copied().collect();
        let mut on: Vec<&'static str> = want.difference(&self.joy_keys).copied().collect();
        off.sort(); on.sort();
        for k in off { self.release(&Binding::Key(k.into()), out); }
        for k in on { self.press(&Binding::Key(k.into()), out); }
        self.joy_keys = want;
    }

    pub fn handle(&mut self, t: TouchEvent) -> Vec<InputEvent> {
        let mut out = Vec::new();
        match t.phase {
            TouchPhase::Down => {
                if self.captures.contains_key(&t.pointer) { return out; }
                let Some(idx) = self.hit(t.x, t.y) else { return out };
                let el = self.layout.elements[idx].clone();
                match el.kind {
                    ElementKind::Button { binding, mode } => {
                        match mode {
                            Mode::Hold => self.press(&binding, &mut out),
                            Mode::Toggle => {
                                if self.toggled.remove(&el.id) { self.release(&binding, &mut out); }
                                else { self.toggled.insert(el.id.clone()); self.press(&binding, &mut out); }
                            }
                        }
                        self.captures.insert(t.pointer, Capture::Button(idx));
                    }
                    ElementKind::Joystick { .. } => { self.captures.insert(t.pointer, Capture::Joystick(idx)); self.update_joystick(idx, t.x, t.y, &mut out); }
                    ElementKind::Camera { .. } => { self.captures.insert(t.pointer, Capture::Camera { idx, last: (t.x, t.y) }); }
                    ElementKind::Hotbar { slots } => {
                        let slot = (((t.x - el.rect.x) / el.rect.w) * slots as f32).floor().clamp(0.0, slots as f32 - 1.0) as u32 + 1;
                        let key = Binding::Key(slot.to_string());
                        self.press(&key, &mut out);
                        self.release(&key, &mut out);
                    }
                }
            }
            TouchPhase::Move => {
                match self.captures.get_mut(&t.pointer) {
                    Some(Capture::Camera { idx, last }) => {
                        let sens = match self.layout.elements[*idx].kind { ElementKind::Camera { sensitivity } => sensitivity, _ => 1.0 };
                        let dx = (t.x - last.0) * self.aspect * CAMERA_BASE * sens;
                        let dy = (t.y - last.1) * CAMERA_BASE * sens;
                        *last = (t.x, t.y);
                        if dx != 0.0 || dy != 0.0 { out.push(InputEvent::MouseMove { dx, dy }); }
                    }
                    Some(Capture::Joystick(idx)) => { let i = *idx; self.update_joystick(i, t.x, t.y, &mut out); }
                    _ => {}
                }
            }
            TouchPhase::Up | TouchPhase::Cancel => {
                match self.captures.remove(&t.pointer) {
                    Some(Capture::Button(idx)) => {
                        if let ElementKind::Button { binding, mode: Mode::Hold } = self.layout.elements[idx].kind.clone() { self.release(&binding, &mut out); }
                    }
                    Some(Capture::Joystick(_)) => self.sync_joy(HashSet::new(), &mut out),
                    _ => {}
                }
            }
        }
        out
    }

    /// Release everything that is held, including toggles. Call on pause, focus loss, layout change
    /// and activity stop so no key/button can stay stuck.
    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        let mut held: Vec<Binding> = self.held.iter().cloned().collect();
        held.sort_by_key(|b| format!("{b:?}"));
        for b in held { self.release(&b, &mut out); }
        self.captures.clear(); self.toggled.clear(); self.joy_keys.clear();
        out
    }

    pub fn set_layout(&mut self, layout: Layout) -> Vec<InputEvent> {
        let out = self.release_all();
        self.layout = layout;
        out
    }
}

// ------------------------------------------------------------------ controller

/// Gamepad mapping: left stick = move, right stick = look, face/shoulder buttons = actions.
pub struct GamepadRouter { router_keys: HashSet<&'static str>, held: HashSet<Binding>, pub look_sensitivity: f32, pub deadzone: f32 }

impl GamepadRouter {
    pub fn new() -> Self { Self { router_keys: HashSet::new(), held: HashSet::new(), look_sensitivity: 1.0, deadzone: 0.2 } }

    pub fn button_binding(name: &str) -> Option<Binding> {
        Some(match name {
            "A" => Binding::Key("SPACE".into()), "B" => Binding::Key("LSHIFT".into()),
            "X" => Binding::Key("F".into()), "Y" => Binding::Key("E".into()),
            "RT" => Binding::Mouse(MouseButton::Left), "LT" => Binding::Mouse(MouseButton::Right),
            "RB" => Binding::ScrollUp, "LB" => Binding::ScrollDown,
            "START" => Binding::Key("ESCAPE".into()), "L3" => Binding::Key("LCTRL".into()),
            "DPAD_UP" => Binding::Key("F5".into()), "DPAD_DOWN" => Binding::Key("Q".into()),
            _ => return None,
        })
    }

    pub fn button(&mut self, name: &str, pressed: bool) -> Vec<InputEvent> {
        let mut out = Vec::new();
        let Some(b) = Self::button_binding(name) else { return out };
        match (&b, pressed) {
            (Binding::ScrollUp, true) => out.push(InputEvent::Scroll(1)),
            (Binding::ScrollDown, true) => out.push(InputEvent::Scroll(-1)),
            (Binding::ScrollUp, false) | (Binding::ScrollDown, false) => {}
            (_, true) => if self.held.insert(b.clone()) { out.push(match &b { Binding::Key(k) => InputEvent::KeyDown(k.clone()), Binding::Mouse(m) => InputEvent::MouseDown(*m), _ => unreachable!() }); },
            (_, false) => if self.held.remove(&b) { out.push(match &b { Binding::Key(k) => InputEvent::KeyUp(k.clone()), Binding::Mouse(m) => InputEvent::MouseUp(*m), _ => unreachable!() }); },
        }
        out
    }

    /// Left stick in [-1,1], y down positive.
    pub fn left_stick(&mut self, x: f32, y: f32) -> Vec<InputEvent> {
        let mut want: HashSet<&'static str> = HashSet::new();
        if y < -self.deadzone { want.insert("W"); }
        if y > self.deadzone { want.insert("S"); }
        if x < -self.deadzone { want.insert("A"); }
        if x > self.deadzone { want.insert("D"); }
        let mut out = Vec::new();
        for k in self.router_keys.difference(&want).copied().collect::<Vec<_>>() { out.push(InputEvent::KeyUp(k.into())); }
        for k in want.difference(&self.router_keys).copied().collect::<Vec<_>>() { out.push(InputEvent::KeyDown(k.into())); }
        self.router_keys = want;
        out
    }

    /// Right stick scaled by elapsed seconds; returns a mouse delta in virtual pixels.
    pub fn right_stick(&self, x: f32, y: f32, dt: f32) -> Option<InputEvent> {
        let (x, y) = (if x.abs() < self.deadzone { 0.0 } else { x }, if y.abs() < self.deadzone { 0.0 } else { y });
        if x == 0.0 && y == 0.0 { return None; }
        let speed = 900.0 * self.look_sensitivity * dt;
        Some(InputEvent::MouseMove { dx: x * speed, dy: y * speed })
    }

    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        for k in self.router_keys.drain() { out.push(InputEvent::KeyUp(k.into())); }
        for b in self.held.drain() {
            match b { Binding::Key(k) => out.push(InputEvent::KeyUp(k)), Binding::Mouse(m) => out.push(InputEvent::MouseUp(m)), _ => {} }
        }
        out
    }
}

// ------------------------------------------------------------------ layout store

pub fn layout_slug(name: &str) -> String {
    let s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "layout".into() } else { s }
}

pub fn save_layout(dirs: &Dirs, l: &Layout) -> Result<String> {
    let slug = layout_slug(&l.name);
    crate::store::write_atomic(&dirs.layouts().join(format!("{slug}.json")), l.to_json()?.as_bytes())?;
    Ok(slug)
}
pub fn load_layout(dirs: &Dirs, slug: &str) -> Result<Layout> {
    let slug = crate::paths::safe_file_name(slug).ok_or_else(|| Error::invalid("Invalid layout id."))?;
    Layout::from_json(&std::fs::read_to_string(dirs.layouts().join(format!("{slug}.json")))?)
}
pub fn list_layouts(dirs: &Dirs) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dirs.layouts()).map(|rd| rd.flatten().filter_map(|e| {
        let n = e.file_name().to_string_lossy().to_string();
        n.strip_suffix(".json").map(str::to_string)
    }).collect()).unwrap_or_default();
    v.sort(); v
}
pub fn delete_layout(dirs: &Dirs, slug: &str) -> Result<()> {
    let slug = crate::paths::safe_file_name(slug).ok_or_else(|| Error::invalid("Invalid layout id."))?;
    Ok(std::fs::remove_file(dirs.layouts().join(format!("{slug}.json")))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(p: u32, ph: TouchPhase, x: f32, y: f32) -> TouchEvent { TouchEvent { pointer: p, phase: ph, x, y } }
    fn center(l: &Layout, id: &str) -> (f32, f32) { let r = l.elements.iter().find(|e| e.id == id).unwrap().rect; (r.cx(), r.cy()) }

    #[test]
    fn default_layout_is_valid_and_roundtrips() {
        let l = default_layout();
        assert!(l.validate().is_empty(), "{:?}", l.validate());
        let l2 = Layout::from_json(&l.to_json().unwrap()).unwrap();
        assert_eq!(l2.elements.len(), l.elements.len());
        assert!(Layout::from_json("{\"schema\":\"other\",\"version\":1,\"name\":\"x\",\"elements\":[]}").is_err());
    }

    #[test]
    fn hold_button_and_multitouch() {
        let l = default_layout();
        let (jx, jy) = center(&l, "jump"); let (ax, ay) = center(&l, "attack");
        let mut r = InputRouter::new(l, 2.2);
        assert_eq!(r.handle(t(1, TouchPhase::Down, jx, jy)), vec![InputEvent::KeyDown("SPACE".into())]);
        assert_eq!(r.handle(t(2, TouchPhase::Down, ax, ay)), vec![InputEvent::MouseDown(MouseButton::Left)]);
        assert_eq!(r.handle(t(1, TouchPhase::Up, jx, jy)), vec![InputEvent::KeyUp("SPACE".into())]);
        assert_eq!(r.handle(t(2, TouchPhase::Cancel, ax, ay)), vec![InputEvent::MouseUp(MouseButton::Left)]);
    }

    #[test]
    fn toggle_then_release_all_clears_everything() {
        let l = default_layout();
        let (sx, sy) = center(&l, "sneak");
        let mut r = InputRouter::new(l, 2.2);
        assert_eq!(r.handle(t(1, TouchPhase::Down, sx, sy)), vec![InputEvent::KeyDown("LSHIFT".into())]);
        assert!(r.handle(t(1, TouchPhase::Up, sx, sy)).is_empty()); // toggle stays on
        assert_eq!(r.release_all(), vec![InputEvent::KeyUp("LSHIFT".into())]);
        assert!(r.release_all().is_empty());
    }

    #[test]
    fn joystick_forward_sprints_and_releases() {
        let l = default_layout();
        let jr = l.elements.iter().find(|e| e.id == "move").unwrap().rect;
        let mut r = InputRouter::new(l, 2.2);
        let ev = r.handle(t(1, TouchPhase::Down, jr.cx(), jr.y)); // top edge = full forward
        assert!(ev.contains(&InputEvent::KeyDown("W".into())));
        assert!(ev.contains(&InputEvent::KeyDown("LCTRL".into())));
        let up = r.handle(t(1, TouchPhase::Up, jr.cx(), jr.y));
        assert!(up.contains(&InputEvent::KeyUp("W".into())) && up.contains(&InputEvent::KeyUp("LCTRL".into())));
    }

    #[test]
    fn camera_emits_scaled_deltas() {
        let mut r = InputRouter::new(default_layout(), 2.0);
        r.handle(t(1, TouchPhase::Down, 0.6, 0.5));
        let ev = r.handle(t(1, TouchPhase::Move, 0.61, 0.5));
        match ev.as_slice() { [InputEvent::MouseMove { dx, dy }] => { assert!((dx - 20.0).abs() < 0.01); assert_eq!(*dy, 0.0); } o => panic!("{o:?}") }
    }

    #[test]
    fn hotbar_taps_slot() {
        let l = default_layout();
        let hb = l.elements.iter().find(|e| e.id == "hotbar").unwrap().rect;
        let mut r = InputRouter::new(l, 2.2);
        let ev = r.handle(t(1, TouchPhase::Down, hb.x + 0.001, hb.cy()));
        assert_eq!(ev, vec![InputEvent::KeyDown("1".into()), InputEvent::KeyUp("1".into())]);
    }

    #[test]
    fn snapping_finds_guides() {
        let (r, g) = snap(Rect { x: 0.012, y: 0.4, w: 0.1, h: 0.1 }, &[], 0.02);
        assert_eq!(r.x, 0.0);
        assert!(g.iter().any(|g| g.vertical && g.pos == 0.0));
    }

    #[test]
    fn gamepad_stick_and_release() {
        let mut g = GamepadRouter::new();
        assert_eq!(g.left_stick(0.0, -1.0), vec![InputEvent::KeyDown("W".into())]);
        assert_eq!(g.release_all(), vec![InputEvent::KeyUp("W".into())]);
    }
}
