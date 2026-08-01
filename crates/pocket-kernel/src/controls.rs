//! Built-in `BUTTON`, `EDIT` and `STATIC` window controls.
//!
//! On a real device these come from the OS: `CreateWindowExW` with a
//! system class name hands back a window whose procedure lives inside
//! `coredll`. That procedure owns the control's pixels, keeps its text,
//! tracks focus, and turns stylus taps into `WM_COMMAND` notifications
//! for the parent — the application never draws a button itself.
//!
//! PocketHLE had no such procedure, so every child collapsed onto the
//! application's own `HWND` and `WndProc`. That was not merely a
//! cosmetic gap: because the child reused the parent's handle it also
//! re-entered the parent's `WM_CREATE`, which creates the children, so
//! CERF BlankApp looped creating `BUTTON`/`BUTTON`/`EDIT` forever and
//! never reached its first `WM_PAINT`.
//!
//! This module models the controls instead. Each one gets its own
//! handle, its own rectangle in parent-client coordinates, and paints
//! itself after the parent's `WM_PAINT` — the same "control owns its
//! pixels" reasoning as [`crate::StatusBar`].

use crate::font;
use crate::gdi::Surface;

/// Which built-in window class a control was created from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlClass {
    Button,
    Edit,
    Static,
}

impl ControlClass {
    /// Match a `CreateWindowExW` class name against the built-in
    /// classes. Win32 class names are case-insensitive, and apps are
    /// inconsistent about it — CERF BlankApp passes `BUTTON` and
    /// `EDIT`, MFC-generated code tends to pass `Button` / `Edit`.
    pub fn from_class_name(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("BUTTON") {
            Some(Self::Button)
        } else if name.eq_ignore_ascii_case("EDIT") {
            Some(Self::Edit)
        } else if name.eq_ignore_ascii_case("STATIC") {
            Some(Self::Static)
        } else {
            None
        }
    }
}

/// `BS_*` button type, held in the low nibble of a `BUTTON`'s style.
const BS_TYPE_MASK: u32 = 0x000F;
const BS_DEFPUSHBUTTON: u32 = 0x0001;
const BS_CHECKBOX: u32 = 0x0002;
const BS_AUTOCHECKBOX: u32 = 0x0003;
const BS_RADIOBUTTON: u32 = 0x0004;
const BS_AUTORADIOBUTTON: u32 = 0x0009;

/// `WS_VISIBLE`.
const WS_VISIBLE: u32 = 0x1000_0000;

/// One built-in control owned by the HLE rather than by the guest.
#[derive(Debug, Clone)]
pub struct ChildWindow {
    /// Handle handed back to the guest from `CreateWindowExW`.
    pub hwnd: u32,
    /// Owning top-level window.
    pub parent: u32,
    pub class: ControlClass,
    /// Control id — the `hMenu` argument of `CreateWindowExW` for a
    /// child window. This is what `GetDlgItem` looks up and what the
    /// parent matches on in `LOWORD(wParam)` of `WM_COMMAND`.
    pub id: u32,
    pub style: u32,
    /// Current caption / edit contents.
    pub text: String,
    /// Rectangle in parent-client coordinates. Controls are routinely
    /// created `0x0` and positioned later from the parent's `WM_SIZE`
    /// handler via `MoveWindow`, which is exactly what BlankApp does.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub visible: bool,
    /// Depressed while the stylus is held down inside a push button.
    pub pressed: bool,
    /// `BM_SETCHECK` state of a check box or radio button.
    pub checked: bool,
}

impl ChildWindow {
    /// Face colour — `COLOR_BTNFACE`, the same light grey the status
    /// bar uses.
    const FACE: u16 = 0xC618;
    /// `COLOR_BTNSHADOW`.
    const SHADOW: u16 = 0x8410;
    /// `COLOR_BTNHIGHLIGHT`.
    const LIGHT: u16 = 0xFFFF;
    /// `COLOR_BTNTEXT` / `COLOR_WINDOWTEXT`.
    const TEXT: u16 = 0x0000;
    /// `COLOR_WINDOW` — the edit control's background.
    const WINDOW: u16 = 0xFFFF;

    /// Side of the square glyph box drawn for a check box / radio.
    const CHECK_BOX: i32 = 12;

    /// Is `(px, py)`, in parent-client coordinates, inside this control?
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    /// `BS_*` type of a button, or `None` for the other classes.
    fn button_type(&self) -> Option<u32> {
        (self.class == ControlClass::Button).then_some(self.style & BS_TYPE_MASK)
    }

    /// A push button — the kind that sends `BN_CLICKED` and pops back
    /// up, as opposed to a check box or radio button that latches.
    pub fn is_push_button(&self) -> bool {
        matches!(self.button_type(), Some(t) if t != BS_CHECKBOX
            && t != BS_AUTOCHECKBOX
            && t != BS_RADIOBUTTON
            && t != BS_AUTORADIOBUTTON)
    }

    /// A button that latches its check state when clicked. Only the
    /// `AUTO` variants toggle themselves; the plain ones leave it to
    /// the application's `WM_COMMAND` handler.
    pub fn is_auto_check(&self) -> bool {
        matches!(
            self.button_type(),
            Some(BS_AUTOCHECKBOX) | Some(BS_AUTORADIOBUTTON)
        )
    }

    /// Any latching button, auto or not — these draw a box/dot rather
    /// than a raised face.
    fn is_check_like(&self) -> bool {
        matches!(
            self.button_type(),
            Some(BS_CHECKBOX)
                | Some(BS_AUTOCHECKBOX)
                | Some(BS_RADIOBUTTON)
                | Some(BS_AUTORADIOBUTTON)
        )
    }

    /// `BS_DEFPUSHBUTTON` — drawn with the extra black outline that
    /// marks the button `VK_RETURN` would activate.
    fn is_default(&self) -> bool {
        self.button_type() == Some(BS_DEFPUSHBUTTON)
    }

    /// Can this control take the input focus? Statics never do, which
    /// is what keeps a tap on a label from stealing the caret from an
    /// edit field.
    pub fn is_focusable(&self) -> bool {
        self.class != ControlClass::Static
    }

    /// Paint the control. `focused` drives the focus rectangle and the
    /// edit caret; the caller knows which control holds the focus.
    pub fn render(&self, surf: &mut Surface<'_>, focused: bool) {
        if !self.visible || self.w <= 0 || self.h <= 0 {
            return;
        }
        match self.class {
            ControlClass::Button => self.render_button(surf, focused),
            ControlClass::Edit => self.render_edit(surf, focused),
            ControlClass::Static => self.render_static(surf),
        }
    }

    fn render_button(&self, surf: &mut Surface<'_>, focused: bool) {
        if self.is_check_like() {
            return self.render_check(surf, focused);
        }
        let (mut x, mut y, mut w, mut h) = (self.x, self.y, self.w, self.h);
        if self.is_default() {
            // The default button's outline sits *outside* the raised
            // face, so the face shrinks by a pixel on every side.
            stroke(surf, x, y, w, h, Self::TEXT);
            x += 1;
            y += 1;
            w -= 2;
            h -= 2;
            if w <= 0 || h <= 0 {
                return;
            }
        }
        surf.fill_rect(x, y, w, h, Self::FACE);
        // Held down: the light and shadow edges swap so the face reads
        // as sunken, and the caption shifts a pixel down-right with it.
        let (tl, br) = if self.pressed {
            (Self::SHADOW, Self::LIGHT)
        } else {
            (Self::LIGHT, Self::SHADOW)
        };
        draw_edge(surf, x, y, w, h, tl, br);

        let nudge = i32::from(self.pressed);
        let text_w = font::str_width(&self.text);
        let tx = x + ((w - text_w) / 2).max(1) + nudge;
        let ty = y + ((h - font::GLYPH_H) / 2).max(0) + nudge;
        draw_clipped(surf, tx, ty, &self.text, Self::TEXT, x + w - 2);

        if focused {
            dotted_rect(surf, x + 3, y + 3, w - 6, h - 6, Self::TEXT);
        }
    }

    fn render_check(&self, surf: &mut Surface<'_>, focused: bool) {
        let box_y = self.y + ((self.h - Self::CHECK_BOX) / 2).max(0);
        let side = Self::CHECK_BOX.min(self.h);
        surf.fill_rect(self.x, box_y, side, side, Self::WINDOW);
        draw_edge(surf, self.x, box_y, side, side, Self::SHADOW, Self::LIGHT);
        if self.checked {
            // A filled core rather than a literal tick: at 12 px with a
            // 6x8 font there is no room for a convincing check glyph.
            surf.fill_rect(self.x + 3, box_y + 3, side - 6, side - 6, Self::TEXT);
        }
        let tx = self.x + side + 4;
        let ty = self.y + ((self.h - font::GLYPH_H) / 2).max(0);
        draw_clipped(surf, tx, ty, &self.text, Self::TEXT, self.x + self.w);
        if focused {
            dotted_rect(surf, tx - 2, self.y, self.w - side - 2, self.h, Self::TEXT);
        }
    }

    fn render_edit(&self, surf: &mut Surface<'_>, focused: bool) {
        surf.fill_rect(self.x, self.y, self.w, self.h, Self::WINDOW);
        // Sunken: shadow along the top/left, highlight along the
        // bottom/right — the inverse of a raised button.
        draw_edge(
            surf,
            self.x,
            self.y,
            self.w,
            self.h,
            Self::SHADOW,
            Self::LIGHT,
        );
        let inner_w = self.w - 6;
        if inner_w <= 0 {
            return;
        }
        // `ES_AUTOHSCROLL` behaviour: once the text outruns the field
        // the caret stays visible, so it is the *tail* that shows.
        let max_chars = (inner_w / font::GLYPH_W).max(0) as usize;
        let len = self.text.chars().count();
        let shown: String = if len > max_chars {
            self.text.chars().skip(len - max_chars).collect()
        } else {
            self.text.clone()
        };
        let ty = self.y + ((self.h - font::GLYPH_H) / 2).max(1);
        font::draw_str(surf, self.x + 3, ty, &shown, Self::TEXT);
        if focused {
            let caret_x = self.x + 3 + font::str_width(&shown);
            if caret_x < self.x + self.w - 1 {
                surf.fill_rect(caret_x, ty, 1, font::GLYPH_H, Self::TEXT);
            }
        }
    }

    fn render_static(&self, surf: &mut Surface<'_>) {
        let ty = self.y + ((self.h - font::GLYPH_H) / 2).max(0);
        draw_clipped(surf, self.x, ty, &self.text, Self::TEXT, self.x + self.w);
    }
}

/// Draw `text` but stop before `right`, so a caption never bleeds out
/// of its control.
fn draw_clipped(surf: &mut Surface<'_>, x: i32, y: i32, text: &str, color: u16, right: i32) {
    let room = (right - x).max(0);
    let max_chars = (room / font::GLYPH_W) as usize;
    if max_chars == 0 {
        return;
    }
    let shown: String = text.chars().take(max_chars).collect();
    font::draw_str(surf, x, y, &shown, color);
}

/// The classic Win32 3D edge: `tl` down the top and left, `br` up the
/// bottom and right.
fn draw_edge(surf: &mut Surface<'_>, x: i32, y: i32, w: i32, h: i32, tl: u16, br: u16) {
    if w <= 0 || h <= 0 {
        return;
    }
    surf.fill_rect(x, y, w, 1, tl);
    surf.fill_rect(x, y, 1, h, tl);
    surf.fill_rect(x, y + h - 1, w, 1, br);
    surf.fill_rect(x + w - 1, y, 1, h, br);
}

/// Solid one-pixel outline.
fn stroke(surf: &mut Surface<'_>, x: i32, y: i32, w: i32, h: i32, color: u16) {
    draw_edge(surf, x, y, w, h, color, color);
}

/// The alternating-pixel focus rectangle Windows draws inside a button
/// that holds the keyboard focus.
fn dotted_rect(surf: &mut Surface<'_>, x: i32, y: i32, w: i32, h: i32, color: u16) {
    if w <= 0 || h <= 0 {
        return;
    }
    for i in 0..w {
        if (x + i + y) % 2 == 0 {
            surf.put_pixel(x + i, y, color);
        }
        if (x + i + y + h - 1) % 2 == 0 {
            surf.put_pixel(x + i, y + h - 1, color);
        }
    }
    for j in 0..h {
        if (x + y + j) % 2 == 0 {
            surf.put_pixel(x, y + j, color);
        }
        if (x + w - 1 + y + j) % 2 == 0 {
            surf.put_pixel(x + w - 1, y + j, color);
        }
    }
}

/// What a stylus or key event did to the control set, so the caller can
/// synthesise the window message a real control would have sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    /// A push button was released inside its own rectangle, or a check
    /// box latched: the parent must receive
    /// `WM_COMMAND(MAKEWPARAM(id, BN_CLICKED), hwndChild)`.
    Clicked { parent: u32, id: u32, hwnd: u32 },
    /// The event was consumed by a control (focus change, a character
    /// typed into an edit) but produces no notification.
    Consumed,
}

/// Every built-in control the guest has created, plus the focus.
#[derive(Debug, Clone)]
pub struct Controls {
    children: Vec<ChildWindow>,
    /// Handle of the control holding the input focus, or `0`.
    pub focus: u32,
    next_hwnd: u32,
    /// Push button the stylus went down on. Win32 captures the mouse
    /// here: releasing outside the button cancels the click instead of
    /// notifying the parent.
    capture: u32,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            children: Vec::new(),
            focus: 0,
            next_hwnd: Self::HWND_BASE,
            capture: 0,
        }
    }
}

impl Controls {
    /// First handle handed out to a control. Kept in the same
    /// `0xDEAD_xxxx` space as the other synthetic window handles, in a
    /// range wide enough that [`Self::is_child_hwnd`] can recognise one
    /// without consulting the state.
    pub const HWND_BASE: u32 = 0xDEAD_0E00;
    /// One past the last control handle.
    pub const HWND_END: u32 = 0xDEAD_0F00;

    /// Does `hwnd` fall in the control handle range? Stateless so the
    /// `IsWindow` family can answer without borrowing the kernel.
    pub fn is_child_hwnd(hwnd: u32) -> bool {
        (Self::HWND_BASE..Self::HWND_END).contains(&hwnd)
    }

    /// Register a new control and return its handle, or `0` if the
    /// handle range is exhausted.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        parent: u32,
        class: ControlClass,
        id: u32,
        text: String,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> u32 {
        if self.next_hwnd >= Self::HWND_END {
            return 0;
        }
        let hwnd = self.next_hwnd;
        self.next_hwnd += 1;
        self.children.push(ChildWindow {
            hwnd,
            parent,
            class,
            id,
            style,
            text,
            x,
            y,
            w,
            h,
            visible: style & WS_VISIBLE != 0,
            pressed: false,
            checked: false,
        });
        hwnd
    }

    pub fn get(&self, hwnd: u32) -> Option<&ChildWindow> {
        self.children.iter().find(|c| c.hwnd == hwnd)
    }

    pub fn get_mut(&mut self, hwnd: u32) -> Option<&mut ChildWindow> {
        self.children.iter_mut().find(|c| c.hwnd == hwnd)
    }

    /// `GetDlgItem`: find a control by its parent and id.
    pub fn by_id(&self, parent: u32, id: u32) -> Option<&ChildWindow> {
        self.children
            .iter()
            .find(|c| c.parent == parent && c.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// Drop a control and anything that referred to it.
    pub fn destroy(&mut self, hwnd: u32) {
        self.children.retain(|c| c.hwnd != hwnd);
        if self.focus == hwnd {
            self.focus = 0;
        }
        if self.capture == hwnd {
            self.capture = 0;
        }
    }

    /// Drop every control owned by `parent` — a top-level window going
    /// away takes its children with it.
    pub fn destroy_children_of(&mut self, parent: u32) {
        let doomed: Vec<u32> = self
            .children
            .iter()
            .filter(|c| c.parent == parent)
            .map(|c| c.hwnd)
            .collect();
        for hwnd in doomed {
            self.destroy(hwnd);
        }
    }

    /// Topmost visible control under `(x, y)`, in parent-client
    /// coordinates. Later-created controls sit on top, matching the
    /// z-order a freshly built dialog has.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<u32> {
        self.children
            .iter()
            .rev()
            .find(|c| c.visible && c.contains(x, y))
            .map(|c| c.hwnd)
    }

    /// Stylus down. Takes the focus, and arms a push button.
    pub fn pointer_down(&mut self, x: i32, y: i32) -> Option<ControlAction> {
        let hwnd = self.hit_test(x, y)?;
        let child = self.get_mut(hwnd)?;
        if !child.is_focusable() {
            return Some(ControlAction::Consumed);
        }
        if child.class == ControlClass::Button {
            child.pressed = true;
            self.capture = hwnd;
        }
        self.focus = hwnd;
        Some(ControlAction::Consumed)
    }

    /// Stylus up. A button only notifies its parent when the release
    /// lands back inside the rectangle the press started in.
    pub fn pointer_up(&mut self, x: i32, y: i32) -> Option<ControlAction> {
        let captured = self.capture;
        if captured == 0 {
            // Not our press — but still swallow a release over a
            // control so the parent does not act on it.
            return self.hit_test(x, y).map(|_| ControlAction::Consumed);
        }
        self.capture = 0;
        let child = self.get_mut(captured)?;
        child.pressed = false;
        if !child.contains(x, y) {
            return Some(ControlAction::Consumed);
        }
        if child.is_auto_check() {
            child.checked = !child.checked;
        }
        Some(ControlAction::Clicked {
            parent: child.parent,
            id: child.id,
            hwnd: child.hwnd,
        })
    }

    /// Feed a virtual-key press to the focused control.
    ///
    /// Returns `None` when nothing has the focus or the key means
    /// nothing here, in which case the caller passes it to the
    /// application as usual.
    pub fn key_down(&mut self, vk: u16) -> Option<ControlAction> {
        const VK_BACK: u16 = 0x08;
        const VK_RETURN: u16 = 0x0D;
        const VK_SPACE: u16 = 0x20;

        let focus = self.focus;
        let child = self.get_mut(focus)?;
        match child.class {
            ControlClass::Edit => {
                if vk == VK_BACK {
                    child.text.pop();
                    return Some(ControlAction::Consumed);
                }
                let ch = vk_to_char(vk)?;
                child.text.push(ch);
                Some(ControlAction::Consumed)
            }
            ControlClass::Button if vk == VK_SPACE || vk == VK_RETURN => {
                if child.is_auto_check() {
                    child.checked = !child.checked;
                }
                Some(ControlAction::Clicked {
                    parent: child.parent,
                    id: child.id,
                    hwnd: child.hwnd,
                })
            }
            _ => None,
        }
    }

    /// Paint every visible control over `surf`.
    ///
    /// Called once the application's own `WM_PAINT` has finished: on a
    /// device the controls are sibling windows that paint after the
    /// parent has filled its client area, and an app that blits over
    /// the whole client rect would otherwise erase them.
    pub fn render(&self, surf: &mut Surface<'_>) {
        for child in &self.children {
            child.render(surf, child.hwnd == self.focus);
        }
    }
}

/// Map a virtual-key code to the character an `EDIT` would receive.
///
/// The host frontends deliver key presses as virtual keys, not
/// `WM_CHAR`, so this is where typing into a field becomes text. Only
/// the unshifted layout is modelled — there is no shift state to
/// consult.
fn vk_to_char(vk: u16) -> Option<char> {
    match vk {
        0x30..=0x39 => Some((b'0' + (vk - 0x30) as u8) as char),
        0x41..=0x5A => Some((b'a' + (vk - 0x41) as u8) as char),
        0x20 => Some(' '),
        // VK_NUMPAD0..9
        0x60..=0x69 => Some((b'0' + (vk - 0x60) as u8) as char),
        0xBD => Some('-'),
        0xBE => Some('.'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gdi::Bitmap;

    const PUSH: u32 = WS_VISIBLE;
    const PARENT: u32 = 0xDEAD_0001;

    fn button(ctrls: &mut Controls, id: u32, x: i32, y: i32) -> u32 {
        ctrls.create(
            PARENT,
            ControlClass::Button,
            id,
            "MsgBox".into(),
            PUSH,
            x,
            y,
            120,
            26,
        )
    }

    #[test]
    fn controls_get_distinct_handles_from_the_parent() {
        let mut ctrls = Controls::default();
        let a = button(&mut ctrls, 201, 60, 72);
        let b = button(&mut ctrls, 202, 60, 108);
        assert_ne!(a, b);
        assert_ne!(a, PARENT);
        assert!(Controls::is_child_hwnd(a));
        assert!(Controls::is_child_hwnd(b));
        assert!(!Controls::is_child_hwnd(PARENT));
    }

    #[test]
    fn get_dlg_item_finds_a_control_by_id() {
        let mut ctrls = Controls::default();
        let a = button(&mut ctrls, 201, 60, 72);
        let b = button(&mut ctrls, 202, 60, 108);
        assert_eq!(ctrls.by_id(PARENT, 201).map(|c| c.hwnd), Some(a));
        assert_eq!(ctrls.by_id(PARENT, 202).map(|c| c.hwnd), Some(b));
        assert!(ctrls.by_id(PARENT, 999).is_none());
        // A different parent must not match.
        assert!(ctrls.by_id(0xDEAD_0002, 201).is_none());
    }

    #[test]
    fn a_press_and_release_inside_a_button_notifies_the_parent() {
        let mut ctrls = Controls::default();
        let hwnd = button(&mut ctrls, 201, 60, 72);
        assert_eq!(ctrls.pointer_down(70, 80), Some(ControlAction::Consumed));
        assert!(ctrls.get(hwnd).unwrap().pressed);
        assert_eq!(ctrls.focus, hwnd);
        assert_eq!(
            ctrls.pointer_up(70, 80),
            Some(ControlAction::Clicked {
                parent: PARENT,
                id: 201,
                hwnd
            })
        );
        assert!(!ctrls.get(hwnd).unwrap().pressed);
    }

    #[test]
    fn releasing_outside_the_button_cancels_the_click() {
        let mut ctrls = Controls::default();
        let hwnd = button(&mut ctrls, 201, 60, 72);
        ctrls.pointer_down(70, 80);
        // Slide off the button before lifting: Win32 captures the
        // mouse and drops the notification.
        assert_eq!(ctrls.pointer_up(70, 300), Some(ControlAction::Consumed));
        assert!(!ctrls.get(hwnd).unwrap().pressed);
    }

    #[test]
    fn a_tap_outside_every_control_is_left_to_the_application() {
        let mut ctrls = Controls::default();
        button(&mut ctrls, 201, 60, 72);
        assert_eq!(ctrls.pointer_down(10, 10), None);
        assert_eq!(ctrls.pointer_up(10, 10), None);
    }

    #[test]
    fn typing_into_a_focused_edit_accumulates_text() {
        let mut ctrls = Controls::default();
        let edit = ctrls.create(
            PARENT,
            ControlClass::Edit,
            203,
            String::new(),
            WS_VISIBLE,
            60,
            144,
            120,
            22,
        );
        ctrls.pointer_down(70, 150);
        assert_eq!(ctrls.focus, edit);
        for vk in [0x41u16, 0x42, 0x20, 0x39] {
            assert_eq!(ctrls.key_down(vk), Some(ControlAction::Consumed));
        }
        assert_eq!(ctrls.get(edit).unwrap().text, "ab 9");
        // VK_BACK rubs out the last character.
        assert_eq!(ctrls.key_down(0x08), Some(ControlAction::Consumed));
        assert_eq!(ctrls.get(edit).unwrap().text, "ab ");
    }

    #[test]
    fn keys_are_left_alone_when_no_control_has_the_focus() {
        let mut ctrls = Controls::default();
        ctrls.create(
            PARENT,
            ControlClass::Edit,
            203,
            String::new(),
            WS_VISIBLE,
            60,
            144,
            120,
            22,
        );
        // Never tapped, so nothing is focused: the game still gets its
        // D-pad presses.
        assert_eq!(ctrls.key_down(0x41), None);
    }

    #[test]
    fn an_auto_checkbox_latches_on_click() {
        let mut ctrls = Controls::default();
        let cb = ctrls.create(
            PARENT,
            ControlClass::Button,
            300,
            "Sound".into(),
            WS_VISIBLE | BS_AUTOCHECKBOX,
            10,
            10,
            100,
            16,
        );
        assert!(!ctrls.get(cb).unwrap().checked);
        ctrls.pointer_down(20, 15);
        ctrls.pointer_up(20, 15);
        assert!(ctrls.get(cb).unwrap().checked);
        ctrls.pointer_down(20, 15);
        ctrls.pointer_up(20, 15);
        assert!(!ctrls.get(cb).unwrap().checked);
    }

    #[test]
    fn a_static_label_never_takes_the_focus() {
        let mut ctrls = Controls::default();
        let label = ctrls.create(
            PARENT,
            ControlClass::Static,
            100,
            "Hello, world".into(),
            WS_VISIBLE,
            10,
            10,
            120,
            16,
        );
        assert_eq!(ctrls.pointer_down(20, 15), Some(ControlAction::Consumed));
        assert_ne!(ctrls.focus, label);
        assert_eq!(ctrls.focus, 0);
    }

    #[test]
    fn destroying_a_parent_takes_its_controls_with_it() {
        let mut ctrls = Controls::default();
        let a = button(&mut ctrls, 201, 60, 72);
        ctrls.focus = a;
        ctrls.destroy_children_of(PARENT);
        assert!(ctrls.is_empty());
        assert_eq!(ctrls.focus, 0);
    }

    #[test]
    fn controls_paint_inside_their_own_rectangles() {
        let mut bm = Bitmap::new(240, 320);
        let mut surf = Surface::Bitmap(&mut bm);
        surf.fill_rect(0, 0, 240, 320, 0x07E0); // green backdrop
        let mut ctrls = Controls::default();
        button(&mut ctrls, 201, 60, 72);
        ctrls.render(&mut surf);

        let pixel = |x: i32, y: i32| -> u16 {
            let i = (y as usize * 240 + x as usize) * 2;
            let px = surf.pixels();
            u16::from_le_bytes([px[i], px[i + 1]])
        };
        // Inside the button: repainted.
        assert_ne!(pixel(100, 80), 0x07E0);
        // One pixel outside every edge: untouched.
        assert_eq!(pixel(59, 80), 0x07E0);
        assert_eq!(pixel(180, 80), 0x07E0);
        assert_eq!(pixel(100, 71), 0x07E0);
        assert_eq!(pixel(100, 98), 0x07E0);
    }

    #[test]
    fn a_zero_sized_control_paints_nothing() {
        let mut bm = Bitmap::new(64, 64);
        let mut surf = Surface::Bitmap(&mut bm);
        surf.fill_rect(0, 0, 64, 64, 0x07E0);
        let mut ctrls = Controls::default();
        // Created 0x0, as BlankApp does before its WM_SIZE runs.
        ctrls.create(
            PARENT,
            ControlClass::Button,
            201,
            "MsgBox".into(),
            WS_VISIBLE,
            0,
            0,
            0,
            0,
        );
        ctrls.render(&mut surf);
        assert!(surf
            .pixels()
            .chunks_exact(2)
            .all(|p| u16::from_le_bytes([p[0], p[1]]) == 0x07E0));
    }

    #[test]
    fn class_names_are_matched_case_insensitively() {
        assert_eq!(
            ControlClass::from_class_name("BUTTON"),
            Some(ControlClass::Button)
        );
        assert_eq!(
            ControlClass::from_class_name("Edit"),
            Some(ControlClass::Edit)
        );
        assert_eq!(
            ControlClass::from_class_name("static"),
            Some(ControlClass::Static)
        );
        assert_eq!(ControlClass::from_class_name("CerfBlankApp"), None);
    }
}
