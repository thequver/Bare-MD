//! Event handling: keyboard, mouse, IME and text input routing.

use std::time::{Duration, Instant};

use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

use crate::app::{App, ConfirmAction, Dialog, Focus, Hit, MenuKind, PickerKind, ToolbarAction};
use crate::gpu::Renderer;
use crate::markdown::Block;

/// Pixels one wheel *notch* scrolls the page.
///
/// Three lines of text on a 24 px line: the old 60 px was two, which felt like the
/// wheel was dragging. A `LineDelta` of 1 is one notch on every platform, so this
/// is the only number that has to change to make the wheel faster.
pub const WHEEL_STEP: f32 = 120.0;
/// How far a mouse-move while dragging a tab scrolls the strip at its edge, in pixels.
pub const TAB_EDGE_SCROLL: f32 = 18.0;
/// Pixels one wheel notch scrolls *sideways* (Shift+wheel) - a little less, because
/// a line of text is much wider than it is tall.
pub const WHEEL_STEP_X: f32 = 72.0;
#[derive(Default)]
pub struct InputState {
    pub modifiers: ModifiersState,
    pub last_key_text_at: Option<Instant>,
}

/// The screen-`y` range the handle's top may occupy: the text, top to bottom.
///
/// Everything about a minimap drag is measured in this one space - the painter draws
/// the handle at the `y` the drag holds, and the page's scroll is read from that same
/// `y` - so the handle cannot be drawn somewhere the drag does not know about.
fn handle_limits(map: crate::ui::Rect, geom: &crate::ui::MinimapGeom, total: f32, text_h: f32) -> (f32, f32) {
    let lo = map.y + geom.track_top();
    let hi = lo + (geom.track_h() - geom.view_h_px(total, text_h)).max(0.0);
    (lo, hi.max(lo))
}

/// The character a shortcut should be matched against, independent of the
/// keyboard layout.
///
/// `Ctrl+S` arrives as `Key::Character("s")` on an English layout and as
/// `Key::Character("ы")` on a Russian one - the *logical* key is whatever that
/// key types, and every shortcut in the editor silently stopped working as soon
/// as the user switched layout. A shortcut is about the **key**, not about the
/// letter printed on it, so this reads the physical position and maps it to the
/// character an English layout puts there (which is what the menus and the tool
/// tips show). Logical keys are still the fallback, for the platforms and keys
/// where no physical code is reported.
pub fn shortcut_char(physical: Option<PhysicalKey>, logical: &Key) -> Option<String> {
    if let Some(PhysicalKey::Code(code)) = physical {
        // Letters and digits are positional; the punctuation keys are matched by
        // what they type on a US layout.
        let ch = match code {
            KeyCode::KeyA => Some("a"),
            KeyCode::KeyB => Some("b"),
            KeyCode::KeyC => Some("c"),
            KeyCode::KeyD => Some("d"),
            KeyCode::KeyE => Some("e"),
            KeyCode::KeyF => Some("f"),
            KeyCode::KeyG => Some("g"),
            KeyCode::KeyH => Some("h"),
            KeyCode::KeyI => Some("i"),
            KeyCode::KeyJ => Some("j"),
            KeyCode::KeyK => Some("k"),
            KeyCode::KeyL => Some("l"),
            KeyCode::KeyM => Some("m"),
            KeyCode::KeyN => Some("n"),
            KeyCode::KeyO => Some("o"),
            KeyCode::KeyP => Some("p"),
            KeyCode::KeyQ => Some("q"),
            KeyCode::KeyR => Some("r"),
            KeyCode::KeyS => Some("s"),
            KeyCode::KeyT => Some("t"),
            KeyCode::KeyU => Some("u"),
            KeyCode::KeyV => Some("v"),
            KeyCode::KeyW => Some("w"),
            KeyCode::KeyX => Some("x"),
            KeyCode::KeyY => Some("y"),
            KeyCode::KeyZ => Some("z"),
            KeyCode::Digit0 => Some("0"),
            KeyCode::Digit1 => Some("1"),
            KeyCode::Digit2 => Some("2"),
            KeyCode::Digit3 => Some("3"),
            KeyCode::Digit4 => Some("4"),
            KeyCode::Digit5 => Some("5"),
            KeyCode::Digit6 => Some("6"),
            KeyCode::Digit7 => Some("7"),
            KeyCode::Digit8 => Some("8"),
            KeyCode::Digit9 => Some("9"),
            KeyCode::Minus => Some("-"),
            KeyCode::Equal => Some("="),
            KeyCode::BracketLeft => Some("["),
            KeyCode::BracketRight => Some("]"),
            KeyCode::Semicolon => Some(";"),
            KeyCode::Quote => Some("'"),
            KeyCode::Comma => Some(","),
            KeyCode::Period => Some("."),
            KeyCode::Slash => Some("/"),
            KeyCode::Backslash => Some("\\"),
            KeyCode::Backquote => Some("`"),
            _ => None,
        };
        if let Some(ch) = ch {
            return Some(ch.to_string());
        }
    }
    match logical {
        Key::Character(ch) => Some(ch.to_lowercase()),
        _ => None,
    }
}

impl App {
    pub fn handle_event(&mut self, ev: &WindowEvent, r: &mut Renderer, input: &mut InputState) -> bool {
        match ev {
            WindowEvent::ModifiersChanged(m) => {
                input.modifiers = m.state();
                self.alt_down = input.modifiers.alt_key();
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32 / r.scale, position.y as f32 / r.scale);
                let pos = self.mouse;
                if self.about_drag.is_some() {
                    // A drag that started on the About box's scrollbar belongs to it
                    // to the end, even when the pointer strays off the track.
                    self.about_drag_to(pos.1);
                    return true;
                }
                if self.dragging_minimap {
                    // The map owns the pointer from the press to the release.
                    self.minimap_scroll(pos, r);
                    return true;
                }
                if self.drag_update(pos) {
                    return true;
                }
                if let Some((part, bg)) = self.dialog.as_ref().and_then(|d| match d {
                    Dialog::Colour(cp) => cp.drag.map(|p| (p, cp.active_bg)),
                    _ => None,
                }) {
                    match part {
                        crate::app::ColourPart::Sv => self.colour_sv_at(bg, pos, true),
                        crate::app::ColourPart::Hue => self.colour_hue_at(bg, pos, true),
                    }
                    return true;
                }
                if self.tree_drag_update(pos) {
                    return true;
                }
                if self.ws_drag_update(pos) {
                    return true;
                }
                if self.tab_drag_update(pos) {
                    return true;
                }
                if self.pan_active {
                    self.pan_to(pos);
                    return true;
                }
                if let Some(f) = self.find_drag {
                    let caret = self.find_caret_at(f, pos.0, r);
                    self.tab_mut().find.place_caret(f, caret, true);
                    return true;
                }
                if let Some(hit) = self.field_drag.clone() {
                    // Dragging across a dialog field selects it, the way it does in
                    // every other text field in the app.
                    self.place_field_caret(hit, pos.0, r, true);
                    return true;
                }
                if self.dragging_selection {
                    self.drag_select_to(pos, r);
                    return true;
                }
                if self.dragging_scrollbar {
                    self.drag_scroll(pos, r);
                    return true;
                }
                if self.dragging_minimap {
                    self.minimap_scroll(pos, r);
                    return true;
                }
                true
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if *button == MouseButton::Right {
                    // A context menu is opened on press and *kept* through the
                    // release: closing it on release made it flash for a single
                    // frame, which is exactly what "the right-click menu closes
                    // instantly" looked like. Only a second press toggles it.
                    if *state == ElementState::Pressed {
                        // Right-click is *not* close. A right-click on a tab used to
                        // close it outright, which loses work for anybody who right-clicks
                        // to see what a tab offers - closing is the tab's own ×, or a
                        // middle click.
                        //
                        // The document's own menu belongs to the document: the
                        // side panels, the tab strip and the status bar get
                        // nothing (a menu of editing commands over the file
                        // tree was never what the click was about).
                        match self.hits.at(self.mouse) {
                            // A tab offers the tab commands (close, close others,
                            // close all, rename) - and never closes on the click
                            // itself, which is what right-click used to do.
                            Some(Hit::Tab(i)) | Some(Hit::TabClose(i)) => {
                                self.open_tab_menu(i, self.mouse)
                            }
                            // A row of the workspace tree: open, rename, duplicate,
                            // copy, paste, delete.
                            Some(Hit::SidebarEntry(i)) => {
                                if let Some(entry) = self.ws.entries().get(i).cloned() {
                                    self.open_tree_menu(entry.path, self.mouse);
                                }
                            }
                            // The empty part of the panel below the last row: the
                            // menu is about the folder the panel is showing, so "New
                            // file" makes the file there.
                            Some(Hit::SidebarBlank) => {
                                let root = self.ws.root.clone();
                                self.open_tree_menu(root, self.mouse);
                            }
                            _ if self.editor_rect.contains(self.mouse) => {
                                self.open_context_menu(self.mouse)
                            }
                            _ => {}
                        }
                    }
                    return true;
                }
                if *button == MouseButton::Middle {
                    // Middle click on a tab closes it (a browser habit, and the
                    // fastest way to get rid of a scratch tab); anywhere else the
                    // middle button still pans.
                    if *state == ElementState::Pressed {
                        if let Some(Hit::Tab(i)) = self.hits.at(self.mouse) {
                            let i = i.min(self.tabs.len().saturating_sub(1));
                            self.close_tab(i);
                            return true;
                        }
                        // Middle click on the empty part of the strip: a new
                        // document, the way a browser opens a new tab.
                        if let Some(Hit::WorkspaceTab(i)) = self.hits.at(self.mouse) {
                            self.close_workspace(i);
                            return true;
                        }
                        if self.hits.at(self.mouse).is_none() && self.tabbar_rect.contains(self.mouse) {
                            self.new_tab();
                            return true;
                        }
                    }
                    match state {
                        ElementState::Pressed => self.pan_begin(),
                        ElementState::Released => self.pan_end(),
                    }
                    return true;
                }
                if *button != MouseButton::Left {
                    return false;
                }
                match state {
                    ElementState::Pressed => {
                        self.on_mouse_down(r);
                        true
                    }
                    ElementState::Released => {
                        self.mouse_up(r);
                        true
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // One notch is [`WHEEL_STEP`] pixels of document, and a touchpad -
                // which reports real *pixels* - moves the page by exactly the
                // distance the fingers travelled.
                //
                // Both used to be converted to "lines" by dividing by 40 and then
                // multiplied by 60, so a notch moved the page 60 px (two lines,
                // and it felt like the wheel was dragging) and a touchpad moved it
                // 1.5 times the finger - neither the same speed nor the right one.
                let (dpx, dpy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x * WHEEL_STEP_X, *y * WHEEL_STEP),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                if input.modifiers.control_key() {
                    self.zoom(if dpy > 0.0 { 1.0 } else { -1.0 });
                } else if let Some(Dialog::Picker(_)) = &self.dialog {
                    // Whichever column the pointer is over gets the wheel: the tree
                    // scrolls itself, the listing moves its selection.
                    let over_tree = self.dialog_tree_rect.contains(self.mouse);
                    let rows = ((self.dialog_tree_rect.h - 8.0) / 22.0).max(1.0) as usize;
                    let step = if dpy < 0.0 { 1 } else { -1 };
                    if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                        if over_tree {
                            pk.scroll_tree(step as i32 * 3, rows);
                        } else {
                            // Scroll the listing, not the selection: the wheel is a
                            // scroll wheel everywhere else in the app, and moving
                            // the selection here silently changed what Open would
                            // open.
                            let list_rows = (self.dialog_list_rect.h / 24.0).max(1.0) as usize;
                            pk.scroll_list(step as i32 * 2, list_rows);
                        }
                    }
                } else if matches!(self.dialog, Some(Dialog::About(_))) {
                    // The wheel scrolls the notice; three lines a notch, the way the
                    // document scrolls a readable amount.
                    let step = if dpy < 0.0 { 3 } else { -3 };
                    if let Some(Dialog::About(a)) = &mut self.dialog {
                        a.scroll_by(step);
                    }
                } else if self.tabbar_rect.contains(self.mouse) {
                    // The wheel over the tab strip scrolls the *strip*. With more tabs
                    // than fit, the newest ones are reached here - the wheel is what
                    // the hand tries first, and it used to scroll the document
                    // underneath a strip the pointer was not even over.
                    let step = if dpy != 0.0 { -dpy.signum() } else { -dpx.signum() };
                    let amount = if dpy.abs() > 0.5 || dpx.abs() > 0.5 {
                        step * crate::app::App::TAB_SCROLL_STEP
                    } else {
                        0.0
                    };
                    self.scroll_tabs_by(amount);
                    return true;
                } else if input.modifiers.shift_key() {
                    self.tab_mut().scroll_x = (self.tab().scroll_x - dpx).max(0.0);
                } else if self.minimap_rect.contains(self.mouse) && self.minimap_wheel(dpy) {
                    // The wheel over the minimap scrolls the map itself.
                } else {
                    // Clamped here as well as at draw time: a wheel that runs past
                    // the end must stop, not bank up a scroll debt.
                    let view = self.text_rect.h;
                    let tab = self.tab_mut();
                    let max = (tab.layout.total_height - view).max(0.0);
                    tab.scroll = (tab.scroll - dpy).clamp(0.0, max);
                }
                true
            }
            WindowEvent::Ime(ime) => {
                use winit::event::Ime;
                if let Ime::Commit(text) = ime {
                    // Suppress the commit when the same key already delivered text
                    // through the key event (avoids doubled characters).
                    if let Some(t) = input.last_key_text_at {
                        if t.elapsed() < Duration::from_millis(60) {
                            return false;
                        }
                    }
                    self.route_text(text, r);
                    return true;
                }
                false
            }
            WindowEvent::KeyboardInput { event, .. } => self.on_key(event, r, input),
            WindowEvent::Focused(focused) => {
                if *focused {
                    self.focus_editor_if_no_overlay();
                }
                true
            }
            _ => false,
        }
    }

    fn focus_editor_if_no_overlay(&mut self) {
        if self.dialog.is_none() && !self.tab().find.open {
            self.focus = Focus::Editor;
        }
    }

    // ---------------- keyboard ----------------

    fn on_key(&mut self, event: &KeyEvent, r: &mut Renderer, input: &mut InputState) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }
        if self.menu.is_some() && !matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
            self.close_menu();
        }
        let mods = input.modifiers;
        let ctrl = mods.control_key();
        let shift = mods.shift_key();
        let alt = mods.alt_key();

        // Dialog / find routing for navigation keys first.
        match event.logical_key.clone() {
            Key::Named(NamedKey::Escape) => {
                if let Some(Dialog::External(ec)) = self.dialog.clone() {
                    // Escape keeps what is open (and stops asking about *this*
                    // change).
                    self.dialog = None;
                    self.note_disk_state(ec.tab);
                    self.focus = Focus::Editor;
                    return true;
                }
                if self.menu.is_some() {
                    self.close_menu();
                    return true;
                }
                if self.dialog.is_some() {
                    self.dialog = None;
                    self.focus = Focus::Editor;
                } else if self.tab().find.open {
                    self.close_find();
                }
                return true;
            }
            Key::Named(NamedKey::Enter) => {
                // Enter in the picker's address bar navigates; anywhere else in a
                // dialog it presses the default button.
                if self.picker_path_enter() {
                    return true;
                }
                if self.dialog.is_some() {
                    self.dialog_confirm(r);
                    return true;
                }
                if self.tab().find.open {
                    self.find_step(!shift);
                    return true;
                }
                if self.focus == Focus::Editor {
                    // Shift+Enter ends a quote/list instead of continuing it.
                    self.newline_ex(shift);
                    self.after_edit(r);
                    return true;
                }
                return false;
            }
            Key::Named(NamedKey::Tab) => {
                if ctrl {
                    self.next_tab(!shift);
                    return true;
                }
                if self.tab().find.open && self.focus != Focus::Editor {
                    self.toggle_find_focus(shift);
                    return true;
                }
                if self.dialog.is_some() {
                    return false;
                }
                if self.focus == Focus::Editor {
                    self.indent(shift);
                    self.after_edit(r);
                    return true;
                }
                return false;
            }
            Key::Named(NamedKey::ArrowLeft)
            | Key::Named(NamedKey::ArrowRight)
            | Key::Named(NamedKey::Home)
            | Key::Named(NamedKey::End)
                if self.focus == Focus::Dialog && self.dialog_field().is_some() =>
            {
                // A dialog field moves its caret exactly like the find fields do.
                let Some(f) = self.dialog_field_mut() else { return false };
                let sel = f.selection();
                match event.logical_key {
                    Key::Named(NamedKey::Home) => f.home(shift),
                    Key::Named(NamedKey::End) => f.end(shift),
                    Key::Named(NamedKey::ArrowLeft) => match (shift, sel) {
                        (false, Some((a, _))) => f.place_caret(a, false),
                        (true, _) => {
                            let at = f.caret;
                            f.place_caret(at - 1, true);
                        }
                        _ => f.move_caret(-1, false),
                    },
                    _ => match (shift, sel) {
                        (false, Some((_, b))) => f.place_caret(b, false),
                        (true, _) => {
                            let at = f.caret;
                            f.place_caret(at + 1, true);
                        }
                        _ => f.move_caret(1, false),
                    },
                }
                return true;
            }
            Key::Named(NamedKey::ArrowLeft)
            | Key::Named(NamedKey::ArrowRight)
            | Key::Named(NamedKey::Home)
            | Key::Named(NamedKey::End)
                if self.focus == Focus::FindQuery || self.focus == Focus::FindReplace =>
            {
                let f = match self.focus {
                    Focus::FindReplace => crate::search::FindFocus::Replace,
                    _ => crate::search::FindFocus::Query,
                };
                let find = &mut self.tab_mut().find;
                let len = find.field(f).chars().count() as i32;
                let (caret, _) = find.caret(f);
                let to = match event.logical_key {
                    Key::Named(NamedKey::Home) => 0,
                    Key::Named(NamedKey::End) => len as usize,
                    Key::Named(NamedKey::ArrowLeft) => (caret as i32 - 1).clamp(0, len) as usize,
                    _ => (caret as i32 + 1).clamp(0, len) as usize,
                };
                find.place_caret(f, to, shift);
                // Left/right over a selection collapses to its edge first.
                if !shift {
                    if let Some((a, b)) = find.selection(f) {
                        let target = match event.logical_key {
                            Key::Named(NamedKey::ArrowLeft) => a,
                            Key::Named(NamedKey::ArrowRight) => b,
                            _ => to,
                        };
                        find.place_caret(f, target, false);
                    }
                }
                return true;
            }
            Key::Named(NamedKey::ArrowUp) | Key::Named(NamedKey::ArrowDown) => {
                let down = matches!(event.logical_key, Key::Named(NamedKey::ArrowDown));
                if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                    let n = pk.entries.len();
                    if n > 0 {
                        let step = if down { 1 } else { -1 };
                        pk.selected = (pk.selected as isize + step).clamp(0, n as isize - 1) as usize;
                    }
                    return true;
                }
                if self.dialog.is_some() {
                    return true;
                }
                if self.focus == Focus::Editor {
                    if alt {
                        self.move_line(down);
                        self.after_edit(r);
                        return true;
                    }
                    self.move_vertical(down, shift);
                    self.after_navigation(r);
                    return true;
                }
                return false;
            }
            _ => {}
        }

        // Shortcuts. Matched on the *physical* key, so they keep working on a
        // non-English layout (see `shortcut_char`).
        if ctrl || (mods.super_key()) {
            if let Key::Named(NamedKey::Tab) = event.logical_key {
                self.next_tab(!shift);
                return true;
            }
            if let Some(c) = shortcut_char(Some(event.physical_key), &event.logical_key) {
                // Ctrl+Z undoes, Ctrl+Y and Ctrl+Shift+Z redo.
                if self.handle_shortcut(&c, shift, r) {
                    return true;
                }
            }
        }

        if alt {
            if let Some(c) = shortcut_char(Some(event.physical_key), &event.logical_key) {
                match c.as_str() {
                    "m" => {
                        self.settings.minimap_open = !self.settings.minimap_open;
                        return true;
                    }
                    "t" => {
                        self.toggle_theme();
                        return true;
                    }
                    "w" => {
                        let on = self.toggle_wrap();
                        self.set_status(if on { "Soft wrap on" } else { "Soft wrap off" });
                        self.after_navigation(r);
                        return true;
                    }
                    "b" => {
                        let on = self.toggle_autoclose();
                        self.set_status(if on {
                            "Auto-close brackets on"
                        } else {
                            "Auto-close brackets off"
                        });
                        return true;
                    }
                    _ => {}
                }
            }
        }

        // F1 is the one key that is about the program rather than the document, and
        // it is what people press to find out what they are running.
        if matches!(event.logical_key, Key::Named(NamedKey::F1)) {
            self.open_about();
            return true;
        }

        // Navigation / editing keys
        match event.logical_key.clone() {
            Key::Named(NamedKey::Backspace) => {
                if self.route_backspace() {
                    self.after_edit(r);
                    return true;
                }
                if self.focus == Focus::Editor {
                    if ctrl {
                        self.delete_word(false);
                    } else {
                        self.backspace();
                    }
                    self.after_edit(r);
                    return true;
                }
                false
            }
            Key::Named(NamedKey::Delete) => {
                if self.focus == Focus::Editor {
                    if ctrl {
                        self.delete_word(true);
                    } else {
                        self.delete_forward();
                    }
                    self.after_edit(r);
                    return true;
                }
                if self.delete_forward_in_field() {
                    return true;
                }
                false
            }
            Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::ArrowRight) => {
                if self.focus != Focus::Editor {
                    return false;
                }
                let forward = matches!(event.logical_key, Key::Named(NamedKey::ArrowRight));
                if ctrl {
                    self.move_word(forward, shift);
                } else {
                    self.move_horizontal(forward, shift);
                }
                self.after_navigation(r);
                true
            }
            Key::Named(NamedKey::Home) | Key::Named(NamedKey::End) => {
                if self.focus != Focus::Editor {
                    return false;
                }
                let end = matches!(event.logical_key, Key::Named(NamedKey::End));
                if ctrl {
                    self.move_doc_boundary(end, shift);
                } else {
                    self.move_line_boundary(end, shift);
                }
                self.after_navigation(r);
                true
            }
            Key::Named(NamedKey::PageUp)
            | Key::Named(NamedKey::PageDown)
            | Key::Named(NamedKey::ArrowUp)
            | Key::Named(NamedKey::ArrowDown)
                if matches!(self.dialog, Some(Dialog::About(_))) =>
            {
                let page = {
                    let visible = match &self.dialog {
                        Some(Dialog::About(a)) => a.visible as i32,
                        _ => 10,
                    };
                    match event.logical_key {
                        Key::Named(NamedKey::PageUp) => -visible,
                        Key::Named(NamedKey::PageDown) => visible,
                        Key::Named(NamedKey::ArrowUp) => -1,
                        _ => 1,
                    }
                };
                if let Some(Dialog::About(a)) = &mut self.dialog {
                    a.scroll_by(page);
                }
                return true;
            }
            Key::Named(NamedKey::PageUp) | Key::Named(NamedKey::PageDown) => {
                if self.focus != Focus::Editor {
                    return false;
                }
                let down = matches!(event.logical_key, Key::Named(NamedKey::PageDown));
                self.page(down, shift);
                self.after_navigation(r);
                true
            }
            Key::Character(text) if !ctrl && !mods.super_key() => {
                if self.focus == Focus::Editor {
                    input.last_key_text_at = Some(Instant::now());
                    self.insert_typed(&text);
                    self.after_edit(r);
                    return true;
                }
                input.last_key_text_at = Some(Instant::now());
                self.route_text(&text, r);
                true
            }
            Key::Named(NamedKey::Space) if !ctrl => {
                input.last_key_text_at = Some(Instant::now());
                if self.focus == Focus::Editor {
                    self.insert_text(" ");
                    self.after_edit(r);
                } else {
                    self.route_text(" ", r);
                }
                true
            }
            _ => false,
        }
    }

    fn toggle_find_focus(&mut self, shift: bool) {
        let (show_replace, focus) = {
            let t = self.tab();
            (t.find.show_replace, t.find.focus)
        };
        let next = match focus {
            crate::search::FindFocus::Query if show_replace => crate::search::FindFocus::Replace,
            _ => crate::search::FindFocus::Query,
        };
        let _ = shift;
        self.tab_mut().find.focus = next;
        self.focus = match next {
            crate::search::FindFocus::Query => Focus::FindQuery,
            crate::search::FindFocus::Replace => Focus::FindReplace,
        };
    }

    /// Text input into find fields or dialog fields. Returns true when consumed.
    ///
    /// Public so a test can drive the same door the window does.
    pub fn route_text(&mut self, text: &str, _r: &mut Renderer) -> bool {
        match self.focus {
            Focus::FindQuery => {
                let t = self.tab_mut();
                t.find.insert(crate::search::FindFocus::Query, text);
                true
            }
            Focus::FindReplace => {
                let t = self.tab_mut();
                t.find.insert(crate::search::FindFocus::Replace, text);
                true
            }
            Focus::Dialog => {
                if let Some(Dialog::Colour(cp)) = &mut self.dialog {
                    // The hex box is a normal text field: it takes the characters
                    // at its caret, replacing any selection, and it keeps working
                    // once it holds six of them (select-all, backspace, delete).
                    let bg = cp.active_bg;
                    let slot = cp.slot_mut(bg);
                    if slot.hex.insert(text) {
                        slot.commit_hex();
                    }
                    return true;
                }
                if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                    pk.insert_text(text);
                    return true;
                }
                // Every other dialog that owns a text field - the rename box - types
                // through the same `TextField` accessor the caret, select-all, cut,
                // copy and paste already use. Listing the boxes one by one is what
                // left the rename box open, focused and deaf.
                if let Some(f) = self.dialog_field_mut() {
                    f.insert(text);
                    return true;
                }
                false
            }
            Focus::Editor => false,
        }
    }

    /// Forward delete inside a find field. Returns true when consumed.
    pub fn delete_forward_in_field(&mut self) -> bool {
        match self.focus {
            Focus::FindQuery => {
                self.tab_mut().find.delete_forward(crate::search::FindFocus::Query);
                true
            }
            Focus::FindReplace => {
                self.tab_mut().find.delete_forward(crate::search::FindFocus::Replace);
                true
            }
            Focus::Dialog => {
                if let Some(Dialog::Colour(cp)) = &mut self.dialog {
                    let bg = cp.active_bg;
                    let slot = cp.slot_mut(bg);
                    if slot.hex.delete_forward() {
                        slot.commit_hex();
                    }
                    return true;
                }
                if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                    pk.delete_forward();
                    return true;
                }
                if let Some(f) = self.dialog_field_mut() {
                    f.delete_forward();
                    return true;
                }
                false
            }
            Focus::Editor => false,
        }
    }

    /// Backspace inside a text field. Returns true when consumed.
    pub fn route_backspace(&mut self) -> bool {
        match self.focus {
            Focus::FindQuery => {
                let t = self.tab_mut();
                t.find.backspace(crate::search::FindFocus::Query);
                true
            }
            Focus::FindReplace => {
                let t = self.tab_mut();
                t.find.backspace(crate::search::FindFocus::Replace);
                true
            }
            Focus::Dialog => {
                if let Some(Dialog::Colour(cp)) = &mut self.dialog {
                    let bg = cp.active_bg;
                    let slot = cp.slot_mut(bg);
                    if slot.hex.backspace() {
                        slot.commit_hex();
                    }
                    return true;
                }
                if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                    pk.backspace();
                    return true;
                }
                if let Some(f) = self.dialog_field_mut() {
                    f.backspace();
                    return true;
                }
                false
            }
            Focus::Editor => false,
        }
    }

    /// Scroll the caret into view and keep the layout fresh.
    pub fn after_edit(&mut self, r: &mut Renderer) {
        self.refresh_layout(r);
        self.ensure_caret_visible();
    }

    pub fn after_navigation(&mut self, r: &mut Renderer) {
        self.refresh_layout(r);
        self.ensure_caret_visible();
    }

    fn refresh_layout(&mut self, r: &mut Renderer) {
        let theme = self.theme;
        let width = self.text_rect.w.max(80.0);
        let i = self.active.min(self.tabs.len() - 1);
        let te = &mut r.text;
        self.tabs[i].ensure_layout(te, &theme, width, self.mode, self.settings.wrap);
    }

    // ---------------- mouse ----------------

    /// Left button released: finish a grip drag, otherwise fall back to the
    /// click behaviour of whatever is under the pointer.
    pub fn mouse_up(&mut self, r: &mut Renderer) {
        if let Some(d) = self.tree_drag.take() {
            if d.moved {
                // A drag that actually moved a file is not a click on its row.
                if let Some(dir) = d.over {
                    self.move_into_folder(&d.path, &dir);
                }
                self.dragging_selection = false;
                self.mouse_down = false;
                return;
            }
        }
        if let Some(d) = self.ws_drag.take() {
            if d.moved {
                // A drag that moved a chip is not a click on it.
                self.dragging_selection = false;
                self.mouse_down = false;
                return;
            }
        }
        let tab_drag = self.tab_drag.take();
        if let Some(d) = tab_drag {
            if d.moved {
                // A drag that actually moved a tab is not a click on it.
                self.dragging_selection = false;
                self.mouse_down = false;
                return;
            }
        }
        let was_grip = self.drag.map(|d| d.moved).unwrap_or(false);
        let had_grip = self.drag.is_some();
        let ended = self.drag_end(r);
        self.dragging_selection = false;
        self.dragging_scrollbar = false;
        self.dragging_minimap = false;
        self.about_drag = None;
        self.map_drag = None;
        self.map_handle = None;
        self.find_drag = None;
        self.field_drag = None;
        if let Some(Dialog::Colour(cp)) = &mut self.dialog {
            cp.drag = None;
        }
        self.mouse_down = false;
        if ended || was_grip {
            return;
        }
        if had_grip {
            // A grip that was pressed but never moved behaves like a click: it
            // opens the row / column menu it belongs to.
            if let Some(hit) = self.hits.at(self.mouse) {
                match hit {
                    Hit::RowGrip(first, row) => {
                        self.open_row_menu(first, row);
                        return;
                    }
                    Hit::ColGrip(first, col) => {
                        self.open_col_menu(first, col);
                        return;
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn on_mouse_down(&mut self, r: &mut Renderer) {
        let pos = self.mouse;
        self.mouse_down = true;
        // click counting for double click
        let now = Instant::now();
        self.click_count = match self.last_click {
            Some((t, p)) if now.duration_since(t) < Duration::from_millis(400) && (p.0 - pos.0).abs() < 6.0 && (p.1 - pos.1).abs() < 6.0 => self.click_count + 1,
            _ => 1,
        };
        self.last_click = Some((now, pos));

        // The map column is a drag track: a press anywhere inside it grabs the map,
        // and that is decided by geometry so a stale or crowded hit table cannot take
        // the press away from it.
        if self.dialog.is_none()
            && self.menu.is_none()
            && !self.minimap_rect.is_empty()
            && self.minimap_rect.contains(pos)
        {
            self.dragging_minimap = true;
            self.minimap_grab(pos);
            return;
        }

        let hit = self.hits.at(pos);
        // A press on a grip starts a drag; its menu only opens if the pointer
        // never actually moved.
        if let Some(h) = &hit {
            if matches!(h, Hit::RowGrip(..) | Hit::ColGrip(..) | Hit::OutlineGrip(_))
                && self.drag_begin(h, pos)
            {
                self.close_menu();
                return;
            }
        }
        if self.menu.is_some() {
            match &hit {
                Some(Hit::MenuItem(_)) | Some(Hit::MenuButton(_)) => {}
                _ => {
                    self.close_menu();
                }
            }
        }
        // The two right-click menus behave the same way: a press on one of their
        // rows runs it (the match below), their own background only dismisses them,
        // and a press anywhere else dismisses them and then falls through to
        // whatever was clicked.
        if self.tab_menu.is_some() || self.tree_menu.is_some() {
            match &hit {
                Some(Hit::TabMenu(_)) | Some(Hit::TreeMenu(_)) => {}
                Some(Hit::MenuButton(MenuKind::Context)) => {
                    self.tab_menu = None;
                    self.tree_menu = None;
                    return;
                }
                _ => {
                    self.tab_menu = None;
                    self.tree_menu = None;
                }
            }
        }
        // The About box's scrollbar: a press grabs it, a move drags, a release lets go.
        if matches!(hit, Some(Hit::AboutScroll)) {
            self.about_drag_begin(pos);
            return;
        }

        // Dragging inside the colour picker.
        if let Some((part, bg)) = match &hit {
            Some(Hit::ColourSv(bg)) => Some((crate::app::ColourPart::Sv, *bg)),
            Some(Hit::ColourHue(bg)) => Some((crate::app::ColourPart::Hue, *bg)),
            _ => None,
        } {
            if let Some(Dialog::Colour(cp)) = &mut self.dialog {
                cp.drag = Some(part);
                cp.active_bg = bg;
            }
        }

        match hit {
            // Handled above, before the press got this far: the drag owns it.
            Some(Hit::AboutScroll) => {}
            Some(Hit::Editor) => {
                self.focus = Focus::Editor;
                if self.tab().find.open {
                    // Clicking the document leaves the find bar (but keeps it open).
                    self.focus = Focus::Editor;
                }
                // A single click on a link follows it, the way every editor with
                // rendered links does - in a cell exactly as in a paragraph.
                //
                // A table used to be the other way round (a plain click placed the
                // caret, Alt+click followed), which meant a link in a cell simply
                // did not open when you clicked it - the one place a writer is most
                // likely to click a filename. Nothing is lost by making the two the
                // same: Alt+click (or a double click, which selects the word) still
                // puts the caret into the label to edit it, in a table as well.
                let ch = self.char_at_point(pos, r);
                if self.click_count == 1 && !self.alt_down {
                    let target = ch.and_then(|ch| self.link_target_at(ch));
                    if let Some(url) = target {
                        self.open_link(&url);
                        self.after_navigation(r);
                        return;
                    }
                }
                self.place_caret_from_point(pos, r, true);
                self.dragging_selection = true;
            }
            Some(Hit::Task(ch)) => {
                self.toggle_task_at(ch);
                self.after_edit(r);
            }
            Some(Hit::Tab(i)) => {
                self.active = i.min(self.tabs.len() - 1);
                self.focus = Focus::Editor;
                // …and remember that this press may turn into a reorder.
                self.tab_drag = Some(crate::app::TabDrag {
                    index: self.active,
                    moved: false,
                    start: pos,
                    hovered: Some(self.active),
                });
            }
            Some(Hit::TabClose(i)) => {
                self.close_tab(i);
            }
            Some(Hit::NewTab) => self.new_tab(),
            Some(Hit::TabScrollLeft) => {
                self.scroll_tabs_by(-crate::app::App::TAB_SCROLL_STEP);
            }
            Some(Hit::TabScrollRight) => {
                self.scroll_tabs_by(crate::app::App::TAB_SCROLL_STEP);
            }
            Some(Hit::Toolbar(action)) => {
                self.run_toolbar(action, r);
            }
            Some(Hit::SidebarEntry(i)) => {
                // …and remember that this press may turn into a drag onto a folder.
                if let Some(e) = self.ws.entries().get(i).cloned() {
                    self.tree_drag = Some(crate::app::TreeDrag {
                        path: e.path,
                        start: pos,
                        moved: false,
                        over: None,
                    });
                }
                // Two clicks on the same row inside the double-click window are one
                // double click. On a *file* that means rename - the file is already
                // open from the first click, so the second has nothing else to do; on
                // a folder it means "open/collapse it" once, not twice (a toggle run
                // twice is a no-op, which looked like the row was dead).
                let entry = self.ws.entries().get(i).cloned();
                let mut double = false;
                if let Some(e) = &entry {
                    let now = std::time::Instant::now();
                    let near = self
                        .tree_click
                        .as_ref()
                        .map(|(p, t)| {
                            *p == e.path
                                && now.duration_since(*t).as_millis()
                                    < crate::app::DOUBLE_CLICK_MS as u128
                        })
                        .unwrap_or(false);
                    self.tree_click = if near { None } else { Some((e.path.clone(), now)) };
                    double = near;
                }
                if let Some(e) = entry {
                    if double && !e.is_dir {
                        self.open_rename(crate::app::RenameTarget::Entry(e.path.clone()));
                        return;
                    }
                    if e.is_dir {
                        self.ws.toggle(&e.path);
                    } else {
                        if !double {
                            self.open_path(&e.path);
                        }
                        self.focus = Focus::Editor;
                    }
                }
            }
            Some(Hit::StatusPath) => {
                // Clicking the path copies it.
                let path = self.tab().doc.path.clone();
                if let Some(p) = path {
                    let text = p.display().to_string();
                    let copied = self.clipboard().map(|cb| cb.set_text(text).is_ok()).unwrap_or(false);
                    self.set_status(if copied { "Copied!" } else { "Clipboard unavailable" });
                }
            }
            Some(Hit::WorkspaceTab(i)) => {
                self.switch_workspace(i);
                // …and remember that this press may turn into a reorder.
                self.ws_drag = Some(crate::app::WorkspaceDrag {
                    index: self.ws_active,
                    moved: false,
                    start: pos,
                });
            }
            Some(Hit::WorkspaceTabClose(i)) => self.close_workspace(i),
            Some(Hit::WorkspaceAdd) => {
                self.open_picker(PickerKind::OpenFolder);
            }
            Some(Hit::ExternalReload) => {
                if let Some(Dialog::External(ec)) = self.dialog.clone() {
                    self.dialog = None;
                    self.active = ec.tab.min(self.tabs.len().saturating_sub(1));
                    self.reload_file();
                    self.focus = Focus::Editor;
                }
            }
            Some(Hit::ExternalKeep) => {
                // Keep the buffer as it is, but remember the file's new stamp so
                // the question is not asked again for the same change.
                if let Some(Dialog::External(ec)) = self.dialog.clone() {
                    self.dialog = None;
                    self.note_disk_state(ec.tab);
                    self.focus = Focus::Editor;
                    self.set_status("kept the open version");
                }
            }
            Some(Hit::ExternalAuto) => {
                if let Some(Dialog::External(ec)) = self.dialog.clone() {
                    self.dialog = None;
                    self.auto_reload = true;
                    self.active = ec.tab.min(self.tabs.len().saturating_sub(1));
                    self.reload_file();
                    self.focus = Focus::Editor;
                    self.set_status("auto-reload on for this session");
                }
            }
            Some(Hit::DialogTreeItem(i)) => {
                let dir = match &self.dialog {
                    Some(Dialog::Picker(pk)) => pk.tree.entry_dir(i),
                    _ => None,
                };
                if let Some(dir) = dir {
                    if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                        pk.select_dir(dir);
                    }
                    self.focus = Focus::Dialog;
                }
            }
            Some(Hit::SidebarOpenFolder) => {
                // A dialog, with the filesystem tree on the left and the folder's
                // contents on the right - the shape every folder selector has.
                self.open_picker(PickerKind::OpenFolder);
            }
            Some(Hit::DialogTreeChevron(i)) => {
                if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                    pk.toggle_tree(i);
                }
                self.focus = Focus::Dialog;
            }
            Some(Hit::SidebarUp) => {
                let parent = self.ws.root.parent().map(|p| p.to_path_buf());
                if let Some(p) = parent {
                    self.ws.set_root(p);
                }
            }
            Some(Hit::CopyCode(a, b)) => {
                // The card's copy button: exactly the lines the card shows, no
                // fence markers.
                let code = self.code_text(a, b);
                let copied = self.clipboard().map(|cb| cb.set_text(code).is_ok()).unwrap_or(false);
                self.focus = Focus::Editor;
                self.set_status(if copied { "Code copied" } else { "Could not reach the clipboard" });
            }
            Some(Hit::OutlineItem(i)) => {
                let target = self.tab().layout.headings.get(i).map(|(line, _, _)| *line);
                if let Some(line) = target {
                    let start = self.tab().doc.line_start(line);
                    self.set_caret(start, false);
                    self.scroll_to_line(line);
                    self.focus = Focus::Editor;
                }
            }
            Some(Hit::ScrollV) => {
                self.dragging_scrollbar = true;
                self.scroll_start = (self.tab().scroll, 0.0);
                self.drag_scroll_anchor = pos;
            }
            Some(Hit::ScrollH) => {
                self.dragging_scrollbar = true;
                self.scroll_start = (self.tab().scroll_x, 0.0);
                self.drag_scroll_anchor = pos;
            }
            Some(Hit::Minimap) => {
                self.dragging_minimap = true;
                self.minimap_grab(pos);
            }
            Some(Hit::FindQuery) => {
                self.focus = Focus::FindQuery;
                let f = crate::search::FindFocus::Query;
                self.tab_mut().find.focus = f;
                self.tab_mut().find.open = true;
                self.find_field_press(f, pos, r);
            }
            Some(Hit::FindReplace) => {
                self.focus = Focus::FindReplace;
                let f = crate::search::FindFocus::Replace;
                self.tab_mut().find.focus = f;
                self.find_field_press(f, pos, r);
            }
            Some(Hit::FindNext) => self.find_step(true),
            Some(Hit::FindPrev) => self.find_step(false),
            Some(Hit::FindToggleCase) => {
                let t = self.tab_mut();
                t.find.case_sensitive = !t.find.case_sensitive;
                t.find.version = u64::MAX;
            }
            Some(Hit::FindToggleWord) => {
                let t = self.tab_mut();
                t.find.whole_word = !t.find.whole_word;
                t.find.version = u64::MAX;
            }
            Some(Hit::FindReplaceOne) => {
                self.find_replace_one();
                self.after_edit(r);
            }
            Some(Hit::FindReplaceAll) => {
                self.find_replace_all();
                self.after_edit(r);
            }
            Some(Hit::FindClose) => self.close_find(),
            Some(Hit::StatusSave) => {
                self.save();
            }
            Some(Hit::DialogItem(i)) => {
                if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                    pk.selected = i;
                    let entry = pk.selected_entry().map(|e| (e.path.clone(), e.is_dir));
                    if let Some((path, is_dir)) = entry {
                        if is_dir && self.click_count > 1 {
                            pk.navigate(&path.display().to_string());
                        } else if !is_dir {
                            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                            pk.name.set(&name);
                        }
                    }
                }
            }
            Some(Hit::DialogUp) => {
                let parent = match &self.dialog {
                    Some(Dialog::Picker(pk)) => pk
                        .dir
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| pk.dir.clone())
                        .display()
                        .to_string(),
                    _ => String::new(),
                };
                if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                    pk.navigate(&parent);
                }
            }
            Some(Hit::ColourSv(bg)) => {
                self.colour_sv_at(bg, pos, false);
            }
            Some(Hit::ColourHue(bg)) => {
                self.colour_hue_at(bg, pos, false);
            }
            Some(Hit::ColourHex(bg)) => {
                if let Some(Dialog::Colour(cp)) = &mut self.dialog {
                    cp.active_bg = bg;
                }
                self.focus = Focus::Dialog;
                self.place_field_caret(Hit::ColourHex(bg), pos.0, r, false);
                self.field_drag = Some(Hit::ColourHex(bg));
            }
            Some(Hit::ColourPreset(bg, i)) => {
                let value = crate::app::COLOUR_PRESETS[i % crate::app::COLOUR_PRESETS.len()];
                if let Some(Dialog::Colour(cp)) = &mut self.dialog {
                    cp.active_bg = bg;
                    cp.slot_mut(bg).set_rgb(value);
                }
            }
            Some(Hit::ColourClear(bg)) => {
                if let Some(Dialog::Colour(cp)) = &mut self.dialog {
                    cp.active_bg = bg;
                    let slot = cp.slot_mut(bg);
                    // Toggle between "this column applies a colour" and "this
                    // column clears the colour".
                    slot.on = !slot.on;
                    slot.touched = true;
                }
            }
            Some(Hit::ColourOk) => {
                if let Some(Dialog::Colour(cp)) = self.dialog.take() {
                    // Both columns at once: whichever ones were touched.
                    let text = cp.text.clone();
                    let back = cp.background.clone();
                    let (apply_text, apply_back) = cp.columns_to_apply();
                    if apply_text {
                        self.set_text_color(false, if text.on { Some(text.rgb()) } else { None });
                    }
                    if apply_back {
                        self.set_text_color(true, if back.on { Some(back.rgb()) } else { None });
                        if back.on {
                            self.last_colour = Some(back.rgb());
                        }
                    }
                    if text.on && apply_text {
                        self.last_colour = Some(text.rgb());
                    }
                    self.focus = Focus::Editor;
                    self.after_edit(r);
                }
            }
            Some(Hit::ColourCancel) => {
                self.dialog = None;
                self.focus = Focus::Editor;
            }
            Some(Hit::AboutClose) => {
                self.dialog = None;
                self.focus = Focus::Editor;
            }
            Some(Hit::AboutSave) => {
                // Written next to the executable when that is writable, otherwise in
                // the data folder; either way the user is told where it went.
                match self.save_licences() {
                    Ok(path) => self.set_status(format!("Saved {}", path.display())),
                    Err(e) => self.set_status(format!("Could not save the licences: {e}")),
                }
            }
            Some(Hit::DialogRenameField) => {
                self.focus = Focus::Dialog;
                self.place_field_caret(Hit::DialogRenameField, pos.0, r, false);
                self.field_drag = Some(Hit::DialogRenameField);
            }
            Some(Hit::DialogRenameOk) => self.commit_rename(),
            // The action has to be read **before** the menu is closed: the menus are
            // their own storage, so clearing one and then asking it for the entry that
            // was clicked gives nothing back, and every row of both menus did nothing
            // at all. (`run_tab_menu` / `run_tree_menu` take the menu themselves, which
            // is what closes it.)
            Some(Hit::TabMenu(i)) => {
                if let Some(action) = self.tab_menu_entries().get(i).map(|(_, a)| *a) {
                    self.run_tab_menu(action);
                }
            }
            Some(Hit::TreeMenu(i)) => {
                if let Some(action) = self.tree_menu_entries().get(i).map(|(_, a)| *a) {
                    self.run_tree_menu(action);
                }
            }
            Some(Hit::DialogConfirm) => self.dialog_confirm(r),
            Some(Hit::DialogCancel) => {
                self.dialog = None;
                // Cancelling one question stops "close others" / "close all" here
                // rather than skipping the tab and carrying on.
                self.pending_close = None;
                self.focus = Focus::Editor;
            }
            Some(Hit::DialogNameField) => {
                self.focus = Focus::Dialog;
                if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                    pk.field = crate::app::PickField::Name;
                }
                self.place_field_caret(Hit::DialogNameField, pos.0, r, false);
                self.field_drag = Some(Hit::DialogNameField);
            }
            Some(Hit::DialogPathField) => {
                self.focus = Focus::Dialog;
                if let Some(Dialog::Picker(pk)) = &mut self.dialog {
                    pk.field = crate::app::PickField::Path;
                }
                self.place_field_caret(Hit::DialogPathField, pos.0, r, false);
                self.field_drag = Some(Hit::DialogPathField);
            }
            Some(Hit::MenuButton(kind)) => {
                let anchor = self.hits.find(|h| matches!(h, Hit::MenuButton(k) if *k == kind));
                if let Some(rect) = anchor {
                    if self.menu.as_ref().map(|m| m.kind) == Some(kind) {
                        self.close_menu();
                    } else {
                        self.open_menu(kind, rect);
                    }
                }
            }
            Some(Hit::MenuItem(i)) => {
                let action = self.menu.as_ref().and_then(|m| m.entries.get(i)).map(|e| e.action.clone());
                if let Some(action) = action {
                    self.run_menu_action(action, r);
                }
            }
            Some(Hit::TableZone(action, line)) => {
                // The widgets know which row they belong to, and the "+" on the
                // right edge means "append a column", so the caret goes to the
                // last cell first.
                self.focus = Focus::Editor;
                self.caret_to_table_line(line);
                if action == crate::app::TableAction::InsertColRight {
                    self.caret_to_last_cell(line);
                }
                self.run_table_action(action);
                self.after_edit(r);
            }
            // Grips are handled on press (drag) and on release (menu); see above.
            Some(Hit::RowGrip(_, _)) | Some(Hit::ColGrip(_, _)) | Some(Hit::OutlineGrip(_)) => {}
            // The panel's empty part only means something to a right-click, which is
            // handled before the left button ever gets here.
            Some(Hit::SidebarBlank) => {}
            Some(Hit::DialogPlaces(_)) | None => {}
        }
    }

    pub fn dispatch_toolbar(&mut self, action: ToolbarAction, r: &mut Renderer) {
        self.run_toolbar(action, r);
    }

    fn run_toolbar(&mut self, action: ToolbarAction, r: &mut Renderer) {
        use ToolbarAction::*;
        match action {
            New => self.new_tab(),
            Open => self.open_picker(PickerKind::OpenFile),
            OpenFolder => self.open_picker(PickerKind::OpenFolder),
            Save => {
                self.save();
            }
            SaveAs => self.open_picker(PickerKind::SaveAs),
            Undo => {
                self.undo();
                self.after_edit(r);
            }
            Redo => {
                self.redo();
                self.after_edit(r);
            }
            Bold => {
                self.toggle_inline("**", "**");
                self.after_edit(r);
            }
            Italic => {
                self.toggle_inline("*", "*");
                self.after_edit(r);
            }
            Underline => {
                self.toggle_inline("++", "++");
                self.after_edit(r);
            }
            Highlight => {
                self.toggle_inline("==", "==");
                self.after_edit(r);
            }
            Sup => {
                self.toggle_inline("^", "^");
                self.after_edit(r);
            }
            Sub => {
                self.toggle_inline("~", "~");
                self.after_edit(r);
            }
            ClearFormat => {
                self.clear_formatting();
                self.after_edit(r);
            }
            AlignLeft => {
                self.set_alignment(0);
                self.after_edit(r);
            }
            AlignCenter => {
                self.set_alignment(1);
                self.after_edit(r);
            }
            AlignRight => {
                self.set_alignment(2);
                self.after_edit(r);
            }
            Renumber => {
                self.renumber_list();
                self.after_edit(r);
            }
            MoveUp => {
                self.move_line(false);
                self.after_edit(r);
            }
            MoveDown => {
                self.move_line(true);
                self.after_edit(r);
            }
            AttachFile => {
                self.open_picker(PickerKind::Attach);
            }
            Strike => {
                self.toggle_inline("~~", "~~");
                self.after_edit(r);
            }
            Code => {
                // A multi-line selection becomes a fenced block, a short one gets
                // inline back-ticks.
                self.toggle_code_block();
                self.after_edit(r);
            }
            H1 => {
                self.toggle_heading(1);
                self.after_edit(r);
            }
            H2 => {
                self.toggle_heading(2);
                self.after_edit(r);
            }
            H3 => {
                self.toggle_heading(3);
                self.after_edit(r);
            }
            Bullet => {
                self.toggle_list(Block::Bullet);
                self.after_edit(r);
            }
            Ordered => {
                self.toggle_list(Block::Ordered);
                self.after_edit(r);
            }
            Task => {
                self.toggle_list(Block::Task(false));
                self.after_edit(r);
            }
            Quote => {
                self.toggle_quote();
                self.after_edit(r);
            }
            Link => {
                self.insert_link();
                self.after_edit(r);
            }
            Table => {
                self.insert_block("| Column | Column |\n| ------ | ------ |\n| Cell   | Cell   |\n");
                self.after_edit(r);
            }
            Hr => {
                self.insert_block("---\n");
                self.after_edit(r);
            }
            ReloadFile => {
                self.reload_file();
                self.after_navigation(r);
            }
            ToggleRaw => {
                self.cycle_view_mode();
                self.after_navigation(r);
            }
            ToggleSidebar => {
                self.settings.sidebar_open = !self.settings.sidebar_open;
                self.after_navigation(r);
            }
            ToggleOutline => {
                self.settings.outline_open = !self.settings.outline_open;
                self.after_navigation(r);
            }
            ToggleMinimap => {
                self.settings.minimap_open = !self.settings.minimap_open;
            }
            ToggleAutoclose => {
                let on = self.toggle_autoclose();
                self.set_status(if on {
                    "Auto-close brackets on"
                } else {
                    "Auto-close brackets off"
                });
            }
            ToggleWrap => {
                let on = self.toggle_wrap();
                self.set_status(if on { "Soft wrap on" } else { "Soft wrap off" });
            }
            RevealInWorkspace => {
                match self.tab().doc.path.clone() {
                    Some(p) => {
                        self.attach_folder_of(&p);
                        self.after_navigation(r);
                    }
                    None => self.set_status("This document has not been saved yet"),
                }
            }
            ToggleTheme => {
                self.toggle_theme();
            }
            FindReplace => {
                self.open_find(true);
            }
        }
    }

    pub fn dialog_confirm(&mut self, r: &mut Renderer) {
        let Some(dialog) = self.dialog.take() else { return };
        match dialog {
            Dialog::Confirm { on_yes, .. } => match on_yes {
                ConfirmAction::CloseTab(i) => {
                    self.force_close_tab(i);
                    // A "close others" / "close all" that was waiting on this answer
                    // carries on with the tabs it had left. The list is ascending and
                    // is worked from the back, so the indices below the one just
                    // removed are still the same tabs.
                    if let Some(rest) = self.pending_close.take() {
                        self.resume_close(rest);
                    }
                    self.focus = Focus::Editor;
                }
                ConfirmAction::DeletePath(p) => {
                    self.delete_path(&p);
                    self.focus = Focus::Editor;
                }
                ConfirmAction::Quit => {
                    self.should_close = true;
                }
            },
            Dialog::External(ec) => {
                // Enter means "show me what is on disk", Escape is handled by the
                // key layer as "keep mine".
                self.active = ec.tab.min(self.tabs.len().saturating_sub(1));
                self.reload_file();
                self.focus = Focus::Editor;
            }
            Dialog::Rename(box_) => {
                // Enter in the rename box renames - the box stays open on an error,
                // so put it back before committing.
                self.dialog = Some(Dialog::Rename(box_));
                self.commit_rename();
            }
            Dialog::About(_) => {
                // Enter closes the box, like Escape does.
                self.focus = Focus::Editor;
            }
            Dialog::Colour(cp) => {
                // Enter in the dialog applies both columns, like Apply.
                let text = cp.text.clone();
                let back = cp.background.clone();
                let (apply_text, apply_back) = cp.columns_to_apply();
                if apply_text {
                    self.set_text_color(false, if text.on { Some(text.rgb()) } else { None });
                }
                if apply_back {
                    self.set_text_color(true, if back.on { Some(back.rgb()) } else { None });
                }
                self.focus = Focus::Editor;
            }
            Dialog::Picker(mut pk) => {
                let target = pk.selected_entry().map(|e| (e.path.clone(), e.is_dir));
                match pk.kind {
                    PickerKind::OpenFile => {
                        if let Some((path, is_dir)) = target {
                            if is_dir {
                                pk.dir = path;
                                pk.selected = 0;
                                pk.refresh();
                                self.dialog = Some(Dialog::Picker(pk));
                                self.focus = Focus::Dialog;
                            } else {
                                self.open_path(&path);
                                self.focus = Focus::Editor;
                            }
                        }
                    }
                    PickerKind::OpenFolder => {
                        let path = match target {
                            Some((path, true)) => path,
                            Some((path, false)) => path.parent().map(|p| p.to_path_buf()).unwrap_or(path),
                            None => pk.dir.clone(),
                        };
                        self.add_workspace(path);
                        self.focus = Focus::Editor;
                    }
                    PickerKind::Attach => {
                        // A folder in the list is a way to walk to the file you
                        // want to attach, exactly like it is in the open dialog;
                        // only a file is something to attach.
                        match target {
                            Some((path, true)) => {
                                pk.dir = path;
                                pk.selected = 0;
                                pk.refresh();
                                self.dialog = Some(Dialog::Picker(pk));
                                self.focus = Focus::Dialog;
                                return;
                            }
                            Some((path, false)) => {
                                self.attach_file(&path);
                                self.after_edit(r);
                            }
                            None => {}
                        }
                        self.focus = Focus::Editor;
                    }
                    PickerKind::SaveAs => {
                        // Save writes into the folder the dialog is showing - the one in
                        // the address bar - and nowhere else. It used to follow the
                        // listing's selection: with a folder highlighted (and "..", the
                        // parent, is always the first row, so a brand-new document always
                        // has it highlighted) pressing Save walked the dialog into that
                        // folder instead of writing the file, and the document landed
                        // somewhere the user never chose. Walking folders is what the
                        // listing is for: click twice, or use the tree.
                        let mut name = pk.name.as_str().trim().to_string();
                        if name.is_empty() {
                            // Nothing typed: an existing file under the cursor names it.
                            if let Some((path, false)) = &target {
                                name = path
                                    .file_stem()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                            }
                        }
                        if name.is_empty() {
                            pk.error = Some("Enter a file name".into());
                            self.dialog = Some(Dialog::Picker(pk));
                            self.focus = Focus::Dialog;
                            return;
                        }
                        if !name.contains('.') {
                            name.push_str(".md");
                        }
                        let path = pk.dir.join(name);
                        self.save_as(&path);
                        self.focus = Focus::Editor;
                    }
                }
            }
        }
    }

    /// Document char offset under a window position, or `None` when the pointer
    /// is not over a laid-out line.
    ///
    /// Table rows are hit-tested **per cell**: a cell can be empty (a column that
    /// was just added), and an empty cell has no run to measure, so the generic
    /// "walk the runs" mapping dropped the caret at the end of the row — outside
    /// the table — where typing split the table in two.
    pub fn char_at_point(&mut self, pos: (f32, f32), r: &mut Renderer) -> Option<usize> {
        let text = self.text_rect;
        let i = self.active;
        let theme = self.theme;
        let width = text.w.max(80.0);
        self.tabs[i].ensure_layout(&mut r.text, &theme, width, self.mode, self.settings.wrap);
        let tab = &self.tabs[i];
        let y = pos.1 - text.y + tab.scroll;
        let line_idx = tab.layout.line_at_y(y);
        let line = tab.layout.lines.get(line_idx)?;
        // Clicking a folded mermaid block opens it for editing: the caret goes to
        // the first line of its source, where the typing will actually go.
        if line.folded {
            if let Some((open, close)) = tab.layout.mermaid_block_at(line_idx) {
                let target = (open + 1).min(close);
                return Some(tab.layout.lines.get(target).map(|l| l.start_char).unwrap_or(line.start_char));
            }
        }
        // A click maps through the variant that is **on screen right now**: the
        // band *and* the x have to come from the same one. A table row shows its
        // inline markers while the caret is on it and hides them otherwise, and
        // mapping a click through the revealed variant while the rendered one is
        // painted put the caret one character to the right of the pointer as soon
        // as a cell held anything with markers in it (`**bold**`, a link). The row
        // only *becomes* revealed once the caret has landed in it, so the reveal
        // must not be anticipated here.
        let src = tab.line_is_raw(self.mode, line_idx) || tab.table_shows_markers(self.mode, line_idx);
        let ri = line.row_of_y_in(y, src);
        // ---- the empty strip right of the text ----
        // The text column is narrower than the editor, and the gap between it and
        // the minimap (or the window edge) is a big, obvious target: a click there
        // means "the end of the line I am level with". It used to hit nothing at
        // all, because only the wrapped text box was a click surface. It now also
        // *is* the answer directly - the last position of the row that was clicked
        // - instead of being mapped through the row's text, which on a wrapped line
        // could walk into the row the long word flowed onto.
        if pos.0 > text.right() + 0.5 {
            return Some(line.start_char + line.row(ri).end.min(line.chars.len()));
        }
        // The text of a row starts at its own x: an indented block, a centred line,
        // and - new with bidirectional text - a right-to-left paragraph, which is
        // flush with the right margin. Leaving the shift out of this put every click
        // that far to the left of the pointer, which on an RTL line meant every
        // click landed at the start of the text.
        let x = pos.0 - text.x + tab.scroll_x - line.indent - line.align_offset_row(width, ri, src);
        if line.block() == Block::Table && !line.table_cols.is_empty() {
            // The row's cells wrap inside their own columns, so the wrapped band
            // is what knows where a click lands - including in an empty cell.
            if !line.bands(src).is_empty() {
                let local = line.char_at_row_x(&mut r.text, ri, x, src);
                return Some(line.start_char + local.min(line.chars.len()));
            }
            // Which column is the pointer in?
            let mut ci = 0usize;
            for (k, &cx) in line.table_cols.iter().enumerate() {
                if x >= cx - crate::layout::TABLE_CELL_PAD {
                    ci = k;
                }
            }
            if let Some(&(a, b)) = line.info.cells.get(ci) {
                if b <= a {
                    // Empty cell: the caret goes where typing belongs, just
                    // inside the cell.
                    return Some(line.start_char + a.min(line.chars.len()));
                }
                let local = line.char_at_row_x(&mut r.text, ri, x, src);
                return Some(line.start_char + local.clamp(a, b));
            }
            return Some(line.start_char);
        }
        // Clicking a line reveals its markers, so the caret has to be placed in
        // the geometry that line is *about* to have - otherwise a click on a
        // fence row lands at column 0 of the (empty) rendered text.
        let local = line.char_at_row_x(&mut r.text, ri, x, true);
        Some(line.start_char + local)
    }

    fn place_caret_from_point(&mut self, pos: (f32, f32), r: &mut Renderer, extend_from_click: bool) {
        let Some(ch) = self.char_at_point(pos, r) else { return };
        let extend = self.click_count > 1;
        if self.click_count > 1 {
            self.select_word_at(ch);
        } else {
            self.set_caret(ch, extend && !extend_from_click);
        }
    }

    fn drag_select_to(&mut self, pos: (f32, f32), r: &mut Renderer) {
        let text = self.text_rect;
        if pos.1 < text.y {
            let tab = self.tab_mut();
            tab.scroll = (tab.scroll - 12.0).max(0.0);
        } else if pos.1 > text.bottom() {
            let tab = self.tab_mut();
            tab.scroll += 12.0;
        }
        let Some(ch) = self.char_at_point(pos, r) else { return };
        let t = self.tab_mut();
        t.caret = ch.min(t.doc.len_chars());
        t.goal_x = None;
    }

    fn drag_scroll(&mut self, pos: (f32, f32), _r: &mut Renderer) {
        let text = self.text_rect;
        let i = self.active;
        let total = self.tabs[i].layout.total_height.max(1.0);
        let max_scroll = (total - text.h).max(0.0);
        let max_x = (self.tabs[i].layout.max_width - text.w).max(0.0);
        let dy = pos.1 - self.drag_scroll_anchor.1;
        let dx = pos.0 - self.drag_scroll_anchor.0;
        let vertical = self.tabs[i].layout.total_height > text.h;
        let start0 = self.scroll_start.0;
        if vertical {
            let visible_frac = (text.h / total).clamp(0.06, 1.0);
            let thumb_h = (text.h * visible_frac).max(24.0);
            let travel = (text.h - thumb_h).max(1.0);
            let delta = dy / travel * max_scroll;
            let tab = self.tab_mut();
            tab.scroll = (start0 + delta).clamp(0.0, max_scroll);
        }
        if max_x > 0.0 {
            let frac = (text.w / self.tabs[i].layout.max_width).clamp(0.08, 1.0);
            let thumb_w = (text.w * frac).max(30.0);
            let travel = (text.w - thumb_w).max(1.0);
            let delta = dx / travel * max_x;
            let tab = self.tab_mut();
            tab.scroll_x = (start0 + delta).clamp(0.0, max_x);
        }
    }

    /// Scroll so that the document position under the pointer sits where the
    /// pointer is inside the viewport. The map draws on a spacing-free "virtual"
    /// scale, so the click has to be converted through the very same geometry the
    /// painter used -- a plain `y / height * total` lands somewhere else entirely.
    /// Caret placement inside a find field from a click, plus drag-selection
    /// setup: the field is a real text field now, not a place where the caret is
    /// always glued to the end.
    fn find_field_press(&mut self, f: crate::search::FindFocus, pos: (f32, f32), r: &mut Renderer) {
        let caret = self.find_caret_at(f, pos.0, r);
        let shift = false;
        self.tab_mut().find.place_caret(f, caret, shift);
        self.find_drag = Some(f);
    }

    /// Char offset in a find field under an x coordinate.
    fn find_caret_at(&mut self, f: crate::search::FindFocus, x: f32, r: &mut Renderer) -> usize {
        let Some(rect) = self.hits.find(|h| match (h, f) {
            (Hit::FindQuery, crate::search::FindFocus::Query) => true,
            (Hit::FindReplace, crate::search::FindFocus::Replace) => true,
            _ => false,
        }) else {
            return self.tab().find.field(f).chars().count();
        };
        let size = self.theme.ui_size();
        let text = self.tab().find.field(f).chars().collect::<String>();
        let mut best = 0usize;
        let mut best_d = f32::MAX;
        let mut prefix = String::new();
        let mut measure = 0.0f32;
        for (i, ch) in text.chars().enumerate() {
            let w = r.text_width(&ch.to_string(), crate::text::Face::Regular, size);
            let centre = rect.x + 8.0 + measure + w * 0.5;
            let d = (centre - x).abs();
            if d < best_d {
                best_d = d;
                best = i;
            }
            measure += w;
            prefix.push(ch);
        }
        let _ = prefix;
        // Past the last glyph: end of the field.
        let end_x = rect.x + 8.0 + measure;
        if x > end_x {
            text.chars().count()
        } else {
            best
        }
    }

    /// Move a file out of the tree and over the folders in it while it is dragged.
    ///
    /// The drag says which folder would take it, and the drop is what moves it: a
    /// drag that ends anywhere else (or a stray release) moves nothing. Rows are
    /// read from the painter's published rects, so what lights up is exactly the row
    /// under the pointer.
    pub fn tree_drag_update(&mut self, pos: (f32, f32)) -> bool {
        let Some(mut d) = self.tree_drag.clone() else { return false };
        if !self.mouse_down {
            self.tree_drag = None;
            return false;
        }
        let (dx, dy) = (pos.0 - d.start.0, pos.1 - d.start.1);
        if !d.moved && dx.abs() < 5.0 && dy.abs() < 5.0 {
            return false;
        }
        d.moved = true;
        // The folder under the pointer: a row, the root's own row (the address chip),
        // or the blank part of the panel - which is how a file gets back out of a
        // folder and into the root.
        d.over = self.drop_target_at(pos, &d.path);
        self.tree_drag = Some(d);
        true
    }

    /// Move a workspace tab along the sidebar strip while it is being dragged.
    pub fn ws_drag_update(&mut self, pos: (f32, f32)) -> bool {
        let Some(mut d) = self.ws_drag else { return false };
        if !self.mouse_down {
            self.ws_drag = None;
            return false;
        }
        let (dx, dy) = (pos.0 - d.start.0, pos.1 - d.start.1);
        if !d.moved && dx.abs() < 5.0 && dy.abs() < 5.0 {
            return false;
        }
        d.moved = true;
        let target = self
            .ws_tab_rects
            .iter()
            .position(|r| pos.0 >= r.x && pos.0 <= r.right())
            .unwrap_or_else(|| {
                if self.ws_tab_rects.last().map(|r| pos.0 > r.right()).unwrap_or(false) {
                    self.ws_tab_rects.len().saturating_sub(1)
                } else {
                    0
                }
            })
            .min(self.workspaces.len().saturating_sub(1));
        if target != d.index {
            self.move_workspace(d.index, target);
            d.index = target;
        }
        self.ws_drag = Some(d);
        true
    }

    /// Move a tab along the strip while it is being dragged.
    pub fn tab_drag_update(&mut self, pos: (f32, f32)) -> bool {
        let Some(mut d) = self.tab_drag else { return false };
        if !self.mouse_down {
            self.tab_drag = None;
            return false;
        }
        let (dx, dy) = (pos.0 - d.start.0, pos.1 - d.start.1);
        if !d.moved && dx.abs() < 5.0 && dy.abs() < 5.0 {
            return false;
        }
        d.moved = true;
        // Dragging along the strip scrolls it as well as reordering: with more tabs
        // than fit, a drag towards the edge has to bring the ones that are off screen
        // into reach, or a tab could never be dragged onto a neighbour it cannot see.
        if pos.0 > self.tab_strip_right - 24.0 {
            self.scroll_tabs_by(TAB_EDGE_SCROLL);
        } else if pos.0 < self.text_rect.x + 24.0 {
            self.scroll_tabs_by(-TAB_EDGE_SCROLL);
        }
        // Which tab is the pointer over now?
        let target = self
            .tab_rects
            .iter()
            .position(|r| pos.0 >= r.x && pos.0 <= r.right())
            .unwrap_or_else(|| {
                // Past the last chip *drawn*: only the view controls are out there,
                // so the drag means "all the way to the end" only when the strip was
                // cut short by the controls. Otherwise it is the empty part of the
                // strip and the tab goes back to the front.
                let past_last = self.tab_rects.last().map(|r| pos.0 > r.right()).unwrap_or(false);
                let cut_short = self
                    .tab_rects
                    .last()
                    .map(|r| r.right() >= self.tab_strip_right - 1.0)
                    .unwrap_or(false);
                if past_last && cut_short {
                    self.tab_rects.len().saturating_sub(1)
                } else if past_last {
                    0
                } else {
                    0
                }
            })
            .min(self.tabs.len().saturating_sub(1));
        d.hovered = Some(target);
        if target != d.index && d.index < self.tabs.len() {
            let tab = self.tabs.remove(d.index);
            self.tabs.insert(target, tab);
            self.active = target;
            d.index = target;
            self.invalidate_layouts();
        }
        self.tab_drag = Some(d);
        true
    }

    /// Saturation/value square: the pointer picks sat (x) and value (y).
    fn colour_sv_at(&mut self, bg: bool, pos: (f32, f32), dragging: bool) {
        let Some(rect) = self.hits.find(|h| matches!(h, Hit::ColourSv(b) if *b == bg)) else { return };
        if let Some(Dialog::Colour(cp)) = &mut self.dialog {
            cp.active_bg = bg;
            if dragging && cp.drag.is_none() {
                cp.drag = Some(crate::app::ColourPart::Sv);
            }
            let slot = cp.slot_mut(bg);
            slot.sat = ((pos.0 - rect.x) / rect.w.max(1.0)).clamp(0.0, 1.0);
            slot.val = (1.0 - (pos.1 - rect.y) / rect.h.max(1.0)).clamp(0.0, 1.0);
            slot.sync_hex();
        }
    }

    /// Hue strip: the pointer picks the hue.
    fn colour_hue_at(&mut self, bg: bool, pos: (f32, f32), dragging: bool) {
        let Some(rect) = self.hits.find(|h| matches!(h, Hit::ColourHue(b) if *b == bg)) else { return };
        if let Some(Dialog::Colour(cp)) = &mut self.dialog {
            cp.active_bg = bg;
            if dragging && cp.drag.is_none() {
                cp.drag = Some(crate::app::ColourPart::Hue);
            }
            let slot = cp.slot_mut(bg);
            slot.hue = (((pos.1 - rect.y) / rect.h.max(1.0)).clamp(0.0, 1.0) * 360.0).min(359.9);
            slot.sync_hex();
        }
    }

    /// Put a dialog field's caret where it was clicked.
    ///
    /// The field's rect comes from the hit table the painter filled, so the caret
    /// cannot drift from the box that was drawn: there is one piece of geometry,
    /// not a copy in the input layer.
    fn place_field_caret(&mut self, hit: Hit, x: f32, r: &mut Renderer, extend: bool) {
        let Some(rect) = self.hits.find(|h| *h == hit) else { return };
        let size = self.theme.ui_size();
        let Some(f) = self.dialog_field_mut() else { return };
        let local = x - (rect.x + 8.0);
        let idx = f.index_at(r.text_ref(), crate::text::Face::Regular, size, local);
        f.place_caret(idx, extend);
    }

    /// Grab the map: the handle comes to the pointer, wherever in the column it is.
    ///
    /// One rule for the whole map. Press high and the document's top comes into view,
    /// press low and its end does; the press always *moves* the handle, so the map
    /// answers the very first press with something visible, and the drag then has room
    /// in both directions. (Two earlier versions failed on exactly this: a press below
    /// the bars either did nothing, or grabbed a handle with no travel left, and in
    /// both cases the map looked frozen even though the page moved.)
    fn minimap_grab(&mut self, pos: (f32, f32)) {
        let map = self.minimap_rect;
        if map.is_empty() {
            return;
        }
        let (geom, _) = self.minimap_geometry();
        let text_h = self.text_rect.h.max(1.0);
        let total = self.tab().layout.total_height.max(1.0);
        let view = geom.view_h_px(total, text_h);
        let (lo, hi) = handle_limits(map, &geom, total, text_h);
        let top = (pos.1 - view * 0.5).clamp(lo, hi);
        self.map_drag = Some(crate::app::MapDrag {
            pointer_y0: pos.1,
            top0: top,
        });
        self.map_handle = Some(top);
        let scroll = geom.scroll_at_handle_top(top - (map.y + geom.track_top()), total, text_h);
        self.tab_mut().scroll = scroll;
    }

    /// Move the handle with the pointer: one map pixel per pointer pixel.
    ///
    /// The move is measured from the press, not from where the pointer is in the strip:
    /// pressing below the text parks the handle at the end of the document, and from there
    /// a small drag upwards has to move it *now*, not once the pointer has climbed back
    /// into the bars (an absolute mapping does exactly that, and reads as a dead handle).
    fn minimap_scroll(&mut self, pos: (f32, f32), _r: &mut Renderer) {
        let Some(drag) = self.map_drag.clone() else { return };
        let geom = self.minimap_geom_only();
        let text_h = self.text_rect.h.max(1.0);
        let total = self.tab().layout.total_height.max(1.0);
        let (lo, hi) = handle_limits(self.minimap_rect, &geom, total, text_h);
        let top = (drag.top0 + (pos.1 - drag.pointer_y0)).clamp(lo, hi);
        self.map_handle = Some(top);
        let scroll = geom.scroll_at_handle_top(
            top - (self.minimap_rect.y + geom.track_top()),
            total,
            text_h,
        );
        self.tab_mut().scroll = scroll;
    }

    /// One keyboard shortcut, by key (`"s"`, `"z"`, `"1"` …) and Shift state.
    ///
    /// Public so the shortcut table can be driven directly (the alternative is
    /// synthesising a `KeyEvent`, which carries platform-private fields).
    pub fn handle_shortcut(&mut self, c: &str, shift: bool, r: &mut Renderer) -> bool {
        match (c, shift) {

                    ("s", false) => {
                        self.save();
                        true
                    }
                    ("s", true) => {
                        self.open_picker(PickerKind::SaveAs);
                        true
                    }
                    ("o", false) => {
                        self.open_picker(PickerKind::OpenFile);
                        true
                    }
                    ("o", true) => {
                        self.open_picker(PickerKind::OpenFolder);
                        true
                    }
                    ("n", false) => {
                        self.new_tab();
                        true
                    }
                    ("t", false) => {
                        self.new_tab();
                        true
                    }
                    ("w", false) => {
                        let i = self.active;
                        self.close_tab(i);
                        true
                    }
                    ("f", false) => {
                        self.open_find(false);
                        true
                    }
                    ("h", false) => {
                        self.open_find(true);
                        true
                    }
                    ("b", false) => {
                        self.toggle_inline("**", "**");
                        self.after_edit(r);
                        true
                    }
                    ("i", false) => {
                        self.toggle_inline("*", "*");
                        self.after_edit(r);
                        true
                    }
                    ("x", true) => {
                        self.toggle_inline("~~", "~~");
                        self.after_edit(r);
                        true
                    }
                    ("e", false) => {
                        self.toggle_inline("`", "`");
                        self.after_edit(r);
                        true
                    }
                    ("k", false) => {
                        self.insert_link();
                        self.after_edit(r);
                        true
                    }
                    ("a", false) => {
                        if self.focus == Focus::Dialog && self.dialog_select_all() {
                            return true;
                        }
                        match self.focus {
                            Focus::FindQuery => {
                                self.tab_mut().find.select_all(crate::search::FindFocus::Query);
                                true
                            }
                            Focus::FindReplace => {
                                self.tab_mut().find.select_all(crate::search::FindFocus::Replace);
                                true
                            }
                            _ => {
                                self.select_all();
                                true
                            }
                        }
                    }
                    ("c", false) => {
                        if self.focus == Focus::Dialog && self.dialog_copy() {
                            return true;
                        }
                        self.copy();
                        true
                    }
                    ("x", false) => {
                        if self.focus == Focus::Dialog && self.dialog_cut() {
                            return true;
                        }
                        self.cut();
                        self.after_edit(r);
                        true
                    }
                    ("v", false) => {
                        if self.focus == Focus::Dialog && self.dialog_paste() {
                            return true;
                        }
                        self.paste();
                        self.after_edit(r);
                        true
                    }
                    ("z", false) => {
                        self.undo();
                        self.after_edit(r);
                        true
                    }
                    ("z", true) | ("y", false) => {
                        self.redo();
                        self.after_edit(r);
                        true
                    }
                    ("e", true) => {
                        self.settings.sidebar_open = !self.settings.sidebar_open;
                        true
                    }
                    ("m", true) => {
                        self.cycle_view_mode();
                        true
                    }
                    ("u", false) => {
                        self.toggle_inline("++", "++");
                        self.after_edit(r);
                        true
                    }
                    ("h", true) => {
                        self.toggle_inline("==", "==");
                        self.after_edit(r);
                        true
                    }
                    ("1", false) => { self.toggle_heading(1); self.after_edit(r); true }
                    ("2", false) => { self.toggle_heading(2); self.after_edit(r); true }
                    ("3", false) => { self.toggle_heading(3); self.after_edit(r); true }
                    ("4", false) => { self.toggle_heading(4); self.after_edit(r); true }
                    ("5", false) => { self.toggle_heading(5); self.after_edit(r); true }
                    ("6", false) => { self.toggle_heading(6); self.after_edit(r); true }
                    ("t", true) => {
                        // Tools menu from the keyboard.
                        let anchor = self.toolbar_rect;
                        self.open_menu(MenuKind::Tools, anchor);
                        true
                    }
            _ => false,
        }
    }

    /// Drag the minimap to `pos` (only when a drag on the map is running).
    ///
    /// Public because the moved-cursor path is otherwise reachable only through a
    /// synthetic `CursorMoved` event; the drag itself is what has to be tested.
    pub fn minimap_drag(&mut self, pos: (f32, f32), r: &mut Renderer) -> bool {
        if !self.dragging_minimap {
            return false;
        }
        self.minimap_scroll(pos, r);
        true
    }

    /// Enter in the picker's address bar: navigate to what was typed.
    ///
    /// There is no "Go" button any more; the address bar commits the path the
    /// way a file manager's does. Returns whether the key was consumed.
    pub fn picker_path_enter(&mut self) -> bool {
        let Some(Dialog::Picker(pk)) = &self.dialog else { return false };
        if pk.field != crate::app::PickField::Path {
            return false;
        }
        let path = pk.path.as_str().to_string();
        if let Some(Dialog::Picker(pk)) = &mut self.dialog {
            pk.navigate(&path);
            self.focus = Focus::Dialog;
        }
        true
    }

    /// The wheel over the minimap scrolls the document.
    ///
    /// The map has no scroll position of its own any more: it shows the whole
    /// document, so a wheel over it means exactly what a wheel over the text means.
    /// Wheel over the minimap: scrolls the page, in document pixels.
    pub fn minimap_wheel(&mut self, dpy: f32) -> bool {
        if self.minimap_rect.is_empty() || !self.settings.minimap_open {
            return false;
        }
        let view = self.text_rect.h.max(1.0);
        let max = (self.tab().layout.total_height - view).max(0.0);
        let tab = self.tab_mut();
        tab.scroll = (tab.scroll - dpy).clamp(0.0, max);
        true
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    /// A shortcut is about the *key*, not the letter on it.
    ///
    /// With a Russian (or Greek, or Dvorak…) layout the logical key for `Ctrl+Z`
    /// is `Key::Character("я")`, so matching on it made every shortcut in the
    /// editor silently stop working. The physical position is what the shortcut
    /// table is about, and it is the only thing that is the same on every layout.
    #[test]
    fn shortcut_char_ignores_the_keyboard_layout() {
        let cyrillic = Key::Character("я".into());
        assert_eq!(
            shortcut_char(Some(PhysicalKey::Code(KeyCode::KeyZ)), &cyrillic).as_deref(),
            Some("z")
        );
        // The same for the rest of the table: undo, redo, save, bold, headings.
        for (code, want) in [
            (KeyCode::KeyY, "y"),
            (KeyCode::KeyS, "s"),
            (KeyCode::KeyB, "b"),
            (KeyCode::KeyF, "f"),
            (KeyCode::Digit1, "1"),
        ] {
            assert_eq!(
                shortcut_char(Some(PhysicalKey::Code(code)), &Key::Character("ф".into())).as_deref(),
                Some(want)
            );
        }
        // Without a physical code the logical key is still the fallback.
        assert_eq!(shortcut_char(None, &Key::Character("Z".into())).as_deref(), Some("z"));
        assert_eq!(shortcut_char(None, &Key::Named(NamedKey::Escape)), None);
    }
}
