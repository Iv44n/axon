//! The settings screen: a modal glass panel over the grid, opened with the gear in the title bar or
//! `Ctrl+,`. Sliders, a font picker and a switch change the look live; Save writes the file
//! (`config::save`, comments kept), Cancel puts back what was there when the screen opened.
//!
//! Like the chrome, nothing here is kept between frames: the panel is a pure function of the
//! `Settings` state and the window size, painted over the finished grid on every redraw while it
//! is open. Its text is rasterized when drawn, with an own small `Fonts` that exists only while
//! the screen is open (so it costs no memory the rest of the time) and is independent of the
//! font being edited, so the panel does not move while its own sliders do.
//!
//! Coordinates are window pixels (the origin is the window's top-left corner, not the shadow's).
use std::path::PathBuf;

use crate::{
    chrome::{self, TITLE_H},
    config::{self, FontCfg},
    font::{self, Fonts},
    input::sym,
    xkb::Key,
};

// ----------------------------------------------------------------- layout --

pub const PANEL_W: i32 = 460;
const PAD: i32 = 24;
const HEAD: i32 = 56;
const ROW: i32 = 38;
const FOOT: i32 = 66;
/// Left edge of the controls, and the width of a slider track.
const CTRL_X: i32 = 150;
const TRACK_W: i32 = 214;
const ARROW_W: i32 = 30;
/// Where a slider row's number sits: its box starts here and ends at the panel's padding.
const VALUE_X: i32 = CTRL_X + TRACK_W + 14;
const BTN_W: i32 = 88;
const BTN_H: i32 = 30;
const PILL: (i32, i32) = (40, 22);

pub const ROWS: usize = 7;
const PANEL_H: i32 = HEAD + ROWS as i32 * ROW + FOOT;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    Family,
    Size,
    Weight,
    Bold,
    Spacing,
    Line,
    Opacity,
}

const ALL: [Row; ROWS] = [Row::Family, Row::Size, Row::Weight, Row::Bold, Row::Spacing, Row::Line, Row::Opacity];

impl Row {
    fn label(self) -> &'static str {
        match self {
            Row::Family => "Font",
            Row::Size => "Size",
            Row::Weight => "Weight",
            Row::Bold => "Bold text",
            Row::Spacing => "Letter spacing",
            Row::Line => "Line height",
            Row::Opacity => "Opacity",
        }
    }

    /// `(min, max, step)` of a slider row, in the unit the setting is written in. The screen's own
    /// ranges are tighter than the file's (`config.rs` clamps those): what is useful to drag.
    fn range(self) -> (f32, f32, f32) {
        match self {
            Row::Size => (8.0, 40.0, 1.0),
            Row::Weight => (100.0, 900.0, 10.0),
            Row::Spacing => (-4.0, 16.0, 0.5),
            Row::Line => (0.8, 2.0, 0.05),
            Row::Opacity => (0.4, 1.0, 0.02),
            Row::Family | Row::Bold => (0.0, 0.0, 0.0),
        }
    }

    /// What may be typed into the row's box: the file's own limits, wider than the slider's.
    fn limits(self) -> (f32, f32) {
        match self {
            Row::Size => config::SIZE,
            Row::Weight => config::WEIGHT,
            Row::Spacing => config::SPACING,
            Row::Line => config::LINE_HEIGHT,
            Row::Opacity => config::OPACITY,
            Row::Family | Row::Bold => (0.0, 0.0),
        }
    }

    fn index(self) -> usize {
        ALL.iter().position(|&r| r == self).unwrap_or(0)
    }
}

/// What is under the pointer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    /// A slider, and where on its track (0..1).
    Slider(Row, f32),
    /// The font picker's arrows: previous (-1) and next (+1).
    Arrow(i32),
    /// The number on the right of a slider row: click to type it.
    Value(Row),
    Toggle,
    Defaults,
    Cancel,
    Save,
    /// On the panel, on nothing in particular.
    Panel,
    /// Off the panel.
    Outside,
}

/// Top-left corner of the panel in a `ww` x `wh` window: centred under the title bar.
fn origin(ww: i32, wh: i32) -> (i32, i32) {
    let room = wh - TITLE_H as i32;
    ((ww - PANEL_W) / 2, (TITLE_H as i32 + (room - PANEL_H) / 2).max(TITLE_H as i32 + 8))
}

fn row_top(i: usize) -> i32 {
    HEAD + i as i32 * ROW
}

fn button_rect(which: Hit) -> (i32, i32, i32, i32) {
    let y = PANEL_H - FOOT + (FOOT - BTN_H) / 2;
    let x = match which {
        Hit::Defaults => PAD,
        Hit::Cancel => PANEL_W - PAD - 2 * BTN_W - 10,
        _ => PANEL_W - PAD - BTN_W,
    };
    (x, y, if which == Hit::Defaults { 104 } else { BTN_W }, BTN_H)
}

fn inside(x: i32, y: i32, (rx, ry, rw, rh): (i32, i32, i32, i32)) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

// ------------------------------------------------------------------ state --

/// The values being edited.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub font: FontCfg,
    pub opacity: f32,
}

impl Draft {
    fn defaults() -> Draft {
        Draft { font: FontCfg::default(), opacity: chrome::OPACITY }
    }

    fn get(&self, row: Row) -> f32 {
        match row {
            Row::Size => self.font.size,
            Row::Weight => self.font.weight as f32,
            Row::Spacing => self.font.letter_spacing,
            Row::Line => self.font.line_height,
            Row::Opacity => self.opacity,
            Row::Family | Row::Bold => 0.0,
        }
    }

    /// Sets a slider row, snapped to its step and kept inside the screen's range.
    fn set(&mut self, row: Row, v: f32) {
        let (lo, hi, step) = row.range();
        self.store(row, ((v.clamp(lo, hi) - lo) / step).round() * step + lo);
    }

    /// Sets a row to a typed number: only the file's limits apply (and weights are tens).
    fn set_exact(&mut self, row: Row, v: f32) {
        let (lo, hi) = row.limits();
        let v = v.clamp(lo, hi);
        self.store(row, if row == Row::Weight { (v / 10.0).round() * 10.0 } else { v });
    }

    fn store(&mut self, row: Row, v: f32) {
        let v = (v * 1000.0).round() / 1000.0; // 0.88, not 0.8800001
        match row {
            Row::Size => self.font.size = v,
            Row::Weight => self.font.weight = v as u16,
            Row::Spacing => self.font.letter_spacing = v,
            Row::Line => self.font.line_height = v,
            Row::Opacity => self.opacity = v,
            Row::Family | Row::Bold => {}
        }
    }

    fn text(&self, row: Row) -> String {
        let v = self.get(row);
        match row {
            Row::Size | Row::Weight => format!("{v:.0}"),
            Row::Spacing => format!("{v:+.1}").replace("+0.0", "0").replace("-0.0", "0"),
            Row::Line | Row::Opacity => format!("{v:.2}"),
            Row::Family | Row::Bold => String::new(),
        }
    }
}

/// A number being typed into a row's box.
#[derive(Clone, Debug, PartialEq)]
struct Edit {
    row: Row,
    text: String,
    /// Opened by a click: the shown number is selected, so the first key replaces it.
    selected: bool,
}

const EDIT_MAX: usize = 7;

impl Edit {
    /// Adds a typed character if it can be part of a number; the first one replaces a selection.
    fn push(&mut self, c: char) {
        let c = if c == ',' { '.' } else { c };
        if !(c.is_ascii_digit() || c == '.' || c == '-') {
            return;
        }
        if std::mem::take(&mut self.selected) {
            self.text.clear();
        }
        let ok = match c {
            '.' => !self.text.contains('.'),
            '-' => self.text.is_empty(),
            _ => true,
        };
        if ok && self.text.len() < EDIT_MAX {
            self.text.push(c);
        }
    }
}

/// What the caller has to do after an input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    None,
    /// Only the panel changed: repaint.
    Redraw,
    /// A value changed: apply `draft` to the window, then repaint.
    Apply,
    Save,
    Cancel,
}

pub struct Settings {
    pub draft: Draft,
    orig: Draft,
    /// Installed monospaced families `(name, file)`; the picker has one more stop before them: the default.
    families: Vec<(String, PathBuf)>,
    /// 0 = the default font, `n` = `families[n - 1]`.
    fam: usize,
    pub sel: usize,
    pub hover: Hit,
    drag: Option<Row>,
    /// A number being typed.
    edit: Option<Edit>,
    ui: Fonts,
    /// A line for the header instead of its hint (a save that failed).
    pub notice: Option<String>,
}

impl Settings {
    /// Opens the screen on the current look, listing the fonts installed.
    pub fn open(font: &FontCfg, opacity: f32) -> Settings {
        Settings::new(font, opacity, font::monospaced_families())
    }

    pub fn new(font: &FontCfg, opacity: f32, mut families: Vec<(String, PathBuf)>) -> Settings {
        let mut fam = 0;
        if let Some(f) = &font.family {
            let found = families.iter().position(|(n, p)| n.eq_ignore_ascii_case(f) || p == &font::expand_home(f));
            fam = match found {
                Some(i) => i + 1,
                None => {
                    // A font the file names that the scan did not list (a path, a proportional font).
                    families.push((f.rsplit('/').next().unwrap_or(f).to_string(), font::expand_home(f)));
                    families.len()
                }
            };
        }
        let draft = Draft { font: font.clone(), opacity };
        Settings {
            orig: draft.clone(),
            draft,
            families,
            fam,
            sel: 0,
            hover: Hit::Outside,
            drag: None,
            edit: None,
            // The default family at a fixed size: never the font being edited.
            ui: Fonts::new(&FontCfg::sized(13.0)),
            notice: None,
        }
    }

    /// The draft with any number still being typed taken in (before a save).
    pub fn commit(&mut self) -> &Draft {
        self.commit_edit();
        &self.draft
    }

    /// What the screen opened with, to put back on Cancel.
    pub fn original(&self) -> &Draft {
        &self.orig
    }

    /// The settings that differ from what the screen opened with, as `config::save` takes them.
    pub fn changes(&self) -> Vec<(&'static str, Option<String>)> {
        let (d, o) = (&self.draft, &self.orig);
        let mut out = Vec::new();
        if d.font.family != o.font.family {
            out.push(("font.family", d.font.family.as_ref().map(|f| format!("{f:?}"))));
        }
        let mut num = |key, a: f32, b: f32, text: String| {
            if a != b {
                out.push((key, Some(text)));
            }
        };
        num("font.size", d.font.size, o.font.size, format!("{}", d.font.size));
        num("font.weight", d.font.weight as f32, o.font.weight as f32, format!("{}", d.font.weight));
        num("font.letter_spacing", d.font.letter_spacing, o.font.letter_spacing, format!("{}", d.font.letter_spacing));
        num("font.line_height", d.font.line_height, o.font.line_height, format!("{}", d.font.line_height));
        num("window.opacity", d.opacity, o.opacity, format!("{}", d.opacity));
        if d.font.bold != o.font.bold {
            out.push(("font.bold", Some(d.font.bold.to_string())));
        }
        out
    }

    /// After a successful save the draft is what the file says.
    pub fn saved(&mut self) {
        self.orig = self.draft.clone();
        self.notice = None;
    }

    fn family_label(&self) -> &str {
        match self.fam {
            0 => "Default (JetBrains Mono)",
            n => &self.families[n - 1].0,
        }
    }

    /// Moves the picker `dir` stops, wrapping, and points the draft's `family` at the choice.
    fn pick(&mut self, dir: i32) {
        let stops = self.families.len() as i32 + 1;
        self.fam = (self.fam as i32 + dir).rem_euclid(stops) as usize;
        self.draft.font.family = match self.fam {
            0 => None,
            n => {
                let (name, path) = &self.families[n - 1];
                Some(font::family_setting(name, path))
            }
        };
    }

    fn defaults(&mut self) {
        self.draft = Draft::defaults();
        self.fam = 0;
    }

    // ---------------------------------------------------------- pointer --

    /// What is at window pixel `(x, y)`.
    pub fn hit(&self, x: f32, y: f32, ww: u32, wh: u32) -> Hit {
        let (ox, oy) = origin(ww as i32, wh as i32);
        let (x, y) = (x.floor() as i32 - ox, y.floor() as i32 - oy);
        if x < 0 || y < 0 || x >= PANEL_W || y >= PANEL_H {
            return Hit::Outside;
        }
        for b in [Hit::Defaults, Hit::Cancel, Hit::Save] {
            if inside(x, y, button_rect(b)) {
                return b;
            }
        }
        for (i, &row) in ALL.iter().enumerate() {
            if y < row_top(i) || y >= row_top(i) + ROW {
                continue;
            }
            return match row {
                Row::Family if (CTRL_X..CTRL_X + ARROW_W).contains(&x) => Hit::Arrow(-1),
                Row::Family if (PANEL_W - PAD - ARROW_W..PANEL_W - PAD).contains(&x) => Hit::Arrow(1),
                Row::Bold if x < CTRL_X + PILL.0 + 8 => Hit::Toggle,
                Row::Family | Row::Bold => Hit::Panel,
                _ if (CTRL_X - 8..CTRL_X + TRACK_W + 8).contains(&x) => {
                    Hit::Slider(row, ((x - CTRL_X) as f32 / TRACK_W as f32).clamp(0.0, 1.0))
                }
                _ if x >= VALUE_X - 4 && x < PANEL_W - PAD + 6 => Hit::Value(row),
                _ => Hit::Panel,
            };
        }
        Hit::Panel
    }

    /// The pointer moved to `(x, y)`. Returns whether anything on the panel changed.
    pub fn pointer(&mut self, x: f32, y: f32, ww: u32, wh: u32) -> Effect {
        if let Some(row) = self.drag {
            let (ox, _) = origin(ww as i32, wh as i32);
            let t = ((x - (ox + CTRL_X) as f32) / TRACK_W as f32).clamp(0.0, 1.0);
            let (lo, hi, _) = row.range();
            let before = self.draft.clone();
            self.draft.set(row, lo + t * (hi - lo));
            self.hover = Hit::Slider(row, t);
            return if self.draft != before { Effect::Apply } else { Effect::None };
        }
        let now = self.hit(x, y, ww, wh);
        // A slider's hover does not depend on where along the track the pointer is.
        let same = |a: Hit, b: Hit| match (a, b) {
            (Hit::Slider(r, _), Hit::Slider(s, _)) => r == s,
            _ => a == b,
        };
        let changed = !same(now, self.hover);
        self.hover = now;
        if changed { Effect::Redraw } else { Effect::None }
    }

    /// The primary button went down at `(x, y)`.
    pub fn press(&mut self, x: f32, y: f32, ww: u32, wh: u32) -> Effect {
        let hit = self.hit(x, y, ww, wh);
        self.hover = hit;
        // Any press ends a number being typed, keeping it; clicking the same box again starts over.
        let committed = self.commit_edit();
        let again = matches!((hit, committed), (Hit::Value(r), Some((c, _))) if r == c);
        let e = self.press_on(hit, again);
        if e == Effect::None && committed.is_some_and(|(_, changed)| changed) { Effect::Apply } else { e }
    }

    fn press_on(&mut self, hit: Hit, skip_edit: bool) -> Effect {
        match hit {
            Hit::Value(row) => {
                self.sel = row.index();
                if !skip_edit {
                    self.start_edit(row, None);
                }
                Effect::Redraw
            }
            Hit::Slider(row, t) => {
                self.sel = row.index();
                self.drag = Some(row);
                let (lo, hi, _) = row.range();
                let before = self.draft.clone();
                self.draft.set(row, lo + t * (hi - lo));
                if self.draft != before { Effect::Apply } else { Effect::Redraw }
            }
            Hit::Arrow(dir) => {
                self.sel = 0;
                self.pick(dir);
                Effect::Apply
            }
            Hit::Toggle => {
                self.sel = Row::Bold.index();
                self.draft.font.bold = !self.draft.font.bold;
                Effect::Apply
            }
            Hit::Defaults => {
                self.defaults();
                Effect::Apply
            }
            Hit::Cancel => Effect::Cancel,
            Hit::Save => Effect::Save,
            Hit::Panel | Hit::Outside => Effect::None,
        }
    }

    pub fn release(&mut self) {
        self.drag = None;
    }

    // --------------------------------------------------------- keyboard --

    /// A key while the screen is open: it never reaches the shell.
    pub fn key(&mut self, k: &Key) -> Effect {
        if self.edit.is_some() {
            return self.edit_key(k);
        }
        // Typing a number on a slider row starts typing it.
        let row = ALL[self.sel];
        if !matches!(row, Row::Family | Row::Bold) && !k.mods.ctrl && !k.mods.alt {
            if k.text.chars().next().is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-')) {
                self.start_edit(row, Some(&k.text));
                return Effect::Redraw;
            }
        }
        let big = if k.mods.shift { 5.0 } else { 1.0 };
        match k.sym {
            sym::ESCAPE => Effect::Cancel,
            sym::RETURN => Effect::Save,
            sym::TAB if k.mods.shift => self.move_sel(-1),
            sym::TAB | sym::DOWN => self.move_sel(1),
            sym::UP => self.move_sel(-1),
            sym::LEFT => self.adjust(-big),
            sym::RIGHT => self.adjust(big),
            0x20 if ALL[self.sel] == Row::Bold => self.adjust(1.0),
            _ => Effect::None,
        }
    }

    /// Opens the row's box: with the keys typed so far, or with the current number selected.
    fn start_edit(&mut self, row: Row, typed: Option<&str>) {
        let mut e = Edit { row, text: String::new(), selected: typed.is_none() };
        match typed {
            Some(t) => t.chars().for_each(|c| e.push(c)),
            None => e.text = format!("{}", self.draft.get(row)),
        }
        self.edit = Some(e);
    }

    /// Ends the box, keeping a valid number. Returns the row and whether the draft changed.
    fn commit_edit(&mut self) -> Option<(Row, bool)> {
        let e = self.edit.take()?;
        let before = self.draft.get(e.row);
        if let Ok(v) = e.text.replace(',', ".").parse::<f32>() {
            if v.is_finite() {
                self.draft.set_exact(e.row, v);
            }
        }
        Some((e.row, self.draft.get(e.row) != before))
    }

    fn edit_key(&mut self, k: &Key) -> Effect {
        match k.sym {
            sym::ESCAPE => {
                self.edit = None;
                Effect::Redraw
            }
            sym::RETURN => self.finish_edit(0),
            sym::TAB if k.mods.shift => self.finish_edit(-1),
            sym::TAB | sym::DOWN => self.finish_edit(1),
            sym::UP => self.finish_edit(-1),
            sym::BACKSPACE => {
                if let Some(e) = self.edit.as_mut() {
                    if std::mem::take(&mut e.selected) {
                        e.text.clear();
                    } else {
                        e.text.pop();
                    }
                }
                Effect::Redraw
            }
            _ => {
                if let Some(e) = self.edit.as_mut().filter(|_| !k.mods.ctrl && !k.mods.alt) {
                    k.text.chars().for_each(|c| e.push(c));
                }
                Effect::Redraw
            }
        }
    }

    /// Commits the box and moves the selection by `d` rows (0 stays).
    fn finish_edit(&mut self, d: i32) -> Effect {
        let changed = self.commit_edit().is_some_and(|(_, c)| c);
        if d != 0 {
            self.sel = (self.sel as i32 + d).rem_euclid(ROWS as i32) as usize;
        }
        if changed { Effect::Apply } else { Effect::Redraw }
    }

    fn move_sel(&mut self, d: i32) -> Effect {
        self.sel = (self.sel as i32 + d).rem_euclid(ROWS as i32) as usize;
        Effect::Redraw
    }

    /// Left and right on the selected row: `n` steps of its slider, the next font, the switch.
    fn adjust(&mut self, n: f32) -> Effect {
        let row = ALL[self.sel];
        match row {
            Row::Family => self.pick(n.signum() as i32),
            Row::Bold => self.draft.font.bold = !self.draft.font.bold,
            _ => {
                let (_, _, step) = row.range();
                let v = self.draft.get(row) + n * step;
                self.draft.set(row, v);
            }
        }
        Effect::Apply
    }

    // ------------------------------------------------------------- paint --

    /// Paints the dimmed grid and the panel over a finished frame. `(ox, oy)` is where the window
    /// starts in the `w` x `h` framebuffer; `square` is a maximized, fullscreen or tiled window.
    pub fn draw(&self, fb: &mut [u32], w: usize, h: usize, (ox, oy): (usize, usize), (ww, wh): (u32, u32), square: bool) {
        if fb.len() < w * h {
            return;
        }
        let mut p = Paint { fb, w, h, ox: ox as i32, oy: oy as i32 };
        let (ww, wh) = (ww as i32, wh as i32);

        // The grid, dimmed; the title bar and its buttons stay as they are.
        let r = if square { 0.0 } else { chrome::RADIUS };
        p.shape(0, TITLE_H as i32, ww, wh, BLACK, 0.5, &|x, y| {
            if y < wh as f32 - r { 1.0 } else { (0.5 - chrome::rounded_rect(x, y, ww as f32, wh as f32, r)).clamp(0.0, 1.0) }
        });

        let (px, py) = origin(ww, wh);
        let (pw, ph) = (PANEL_W as f32, PANEL_H as f32);
        // Shadow, body, outline.
        p.shape(px - 30, py - 22, px + PANEL_W + 30, py + PANEL_H + 40, BLACK, 0.45, &|x, y| {
            let d = chrome::rounded_rect(x - px as f32, y - py as f32 - 8.0, pw, ph, 14.0);
            let t = ((d + 6.0) / 30.0).clamp(0.0, 1.0);
            (1.0 - t).powi(3) * (0.5 + 0.5 * (d / 6.0).clamp(0.0, 1.0)).min(1.0)
        });
        let body = |x: f32, y: f32| (0.5 - chrome::rounded_rect(x - px as f32, y - py as f32, pw, ph, 14.0)).clamp(0.0, 1.0);
        p.shape(px, py, px + PANEL_W, py + PANEL_H, [0.09, 0.09, 0.10], 1.0, &body);
        p.shape(px, py, px + PANEL_W, py + PANEL_H, WHITE, 0.10, &|x, y| {
            let d = chrome::rounded_rect(x - px as f32, y - py as f32, pw, ph, 14.0);
            (0.5 - d).clamp(0.0, 1.0) * (1.5 + d).clamp(0.0, 1.0)
        });

        let t = |p: &mut Paint, x: i32, cy: i32, s: &str, a: f32, bold: bool| self.text(p, px + x, py + cy, s, WHITE, a, bold);
        t(&mut p, PAD, 28, "Settings", 0.96, true);
        let (head, head_a) = match &self.notice {
            Some(n) => (n.as_str(), 0.95),
            None => ("drag, or click a number to type", 0.40),
        };
        let n = head.chars().count().min(34);
        self.text(&mut p, px + PANEL_W - PAD - n as i32 * self.ui.cell_w as i32, py + 28, &head.chars().take(n).collect::<String>(), WHITE, head_a, false);

        for (i, &row) in ALL.iter().enumerate() {
            let top = py + row_top(i);
            let cy = top + ROW / 2;
            let hovered = matches!(self.hover, Hit::Slider(r, _) if r == row)
                || matches!(self.hover, Hit::Value(r) if r == row)
                || (row == Row::Family && matches!(self.hover, Hit::Arrow(_)))
                || (row == Row::Bold && self.hover == Hit::Toggle);
            if i == self.sel || hovered {
                let a = if i == self.sel { 0.07 } else { 0.04 };
                p.rrect(px + 8, top + 1, PANEL_W - 16, ROW - 2, 8.0, WHITE, a);
            }
            self.text(&mut p, px + PAD, cy, row.label(), WHITE, 0.62, false);
            let cx = px + CTRL_X;
            match row {
                Row::Family => {
                    let right = px + PANEL_W - PAD - ARROW_W;
                    for (x, dir) in [(cx, -1), (right, 1)] {
                        let lit = self.hover == Hit::Arrow(dir);
                        p.rrect(x, cy - 12, ARROW_W, 24, 7.0, WHITE, if lit { 0.20 } else { 0.08 });
                        p.chevron(x + ARROW_W / 2, cy, dir, WHITE, if lit { 1.0 } else { 0.75 });
                    }
                    let room = ((right - cx - ARROW_W - 12) / self.ui.cell_w as i32).max(1) as usize;
                    let label = self.family_label();
                    let shown: String = if label.chars().count() > room {
                        label.chars().take(room - 1).chain(['\u{2026}']).collect()
                    } else {
                        label.to_string()
                    };
                    let tx = cx + ARROW_W + (right - cx - ARROW_W - shown.chars().count() as i32 * self.ui.cell_w as i32) / 2;
                    self.text(&mut p, tx, cy, &shown, WHITE, 0.95, false);
                }
                Row::Bold => {
                    let on = self.draft.font.bold;
                    let (pw, ph) = PILL;
                    p.rrect(cx, cy - ph / 2, pw, ph, ph as f32 / 2.0, WHITE, if on { 0.88 } else { 0.18 });
                    let kx = if on { cx + pw - ph / 2 } else { cx + ph / 2 };
                    p.disc(kx as f32, cy as f32, ph as f32 / 2.0 - 3.0, if on { [0.1, 0.1, 0.11] } else { WHITE }, if on { 0.95 } else { 0.8 });
                    self.text(&mut p, cx + pw + 14, cy, if on { "on" } else { "off" }, WHITE, 0.55, false);
                }
                _ => {
                    let (lo, hi, _) = row.range();
                    let t = ((self.draft.get(row) - lo) / (hi - lo)).clamp(0.0, 1.0);
                    let kx = cx as f32 + t * TRACK_W as f32;
                    let lit = matches!(self.hover, Hit::Slider(r, _) if r == row) || self.drag == Some(row);
                    p.rrect(cx, cy - 2, TRACK_W, 4, 2.0, WHITE, 0.16);
                    p.rrect(cx, cy - 2, (kx - cx as f32).round() as i32, 4, 2.0, WHITE, 0.72);
                    p.disc(kx, cy as f32, if lit { 8.0 } else { 7.0 }, BLACK, 0.35);
                    p.disc(kx, cy as f32, if lit { 6.8 } else { 5.8 }, WHITE, 0.96);
                    let right = px + PANEL_W - PAD;
                    let cw = self.ui.cell_w as i32;
                    match self.edit.as_ref().filter(|e| e.row == row) {
                        // The box being typed into: a lit field, the text, a caret after it.
                        Some(e) => {
                            let bw = EDIT_MAX as i32 * cw + 14;
                            p.rrect(right - bw + 4, cy - 13, bw, 26, 7.0, WHITE, 0.14);
                            p.rrect(right - bw + 5, cy - 12, bw - 2, 24, 6.0, [0.03, 0.03, 0.04], 1.0);
                            let tx = right - e.text.len() as i32 * cw - 3;
                            self.text(&mut p, tx, cy, &e.text, WHITE, 1.0, false);
                            if e.selected {
                                p.rrect(tx - 2, cy - 9, e.text.len() as i32 * cw + 4, 18, 3.0, WHITE, 0.22);
                            } else {
                                p.rrect(right - 2, cy - 8, 2, 16, 1.0, WHITE, 0.95);
                            }
                        }
                        None => {
                            let v = self.draft.text(row);
                            if matches!(self.hover, Hit::Value(r) if r == row) {
                                let bw = v.chars().count() as i32 * cw + 12;
                                p.rrect(right - bw + 3, cy - 12, bw, 24, 6.0, WHITE, 0.10);
                            }
                            self.text(&mut p, right - v.chars().count() as i32 * cw, cy, &v, WHITE, 0.90, false);
                        }
                    }
                }
            }
        }

        for (b, label) in [(Hit::Defaults, "Defaults"), (Hit::Cancel, "Cancel"), (Hit::Save, "Save")] {
            let (x, y, bw, bh) = button_rect(b);
            let lit = self.hover == b;
            let primary = b == Hit::Save;
            let (fill, a, ink) = match (primary, lit) {
                (true, false) => (WHITE, 0.90, [0.06, 0.06, 0.07]),
                (true, true) => (WHITE, 1.0, [0.0, 0.0, 0.0]),
                (false, false) => (WHITE, 0.10, WHITE),
                (false, true) => (WHITE, 0.20, WHITE),
            };
            p.rrect(px + x, py + y, bw, bh, 8.0, fill, a);
            let tw = label.chars().count() as i32 * self.ui.cell_w as i32;
            self.text(&mut p, px + x + (bw - tw) / 2, py + y + bh / 2, label, ink, if primary { 1.0 } else { 0.9 }, primary);
        }
    }

    /// `s` with its left edge at `x` and its vertical centre at `cy` (window pixels).
    fn text(&self, p: &mut Paint, x: i32, cy: i32, s: &str, c: [f32; 3], a: f32, bold: bool) {
        let (cw, ch) = (self.ui.cell_w as i32, self.ui.cell_h as i32);
        for (n, c0) in s.chars().enumerate() {
            if c0 == ' ' {
                continue;
            }
            let bmp = self.ui.raster(c0, bold, false);
            let (gx, gy) = (x + n as i32 * cw, cy - ch / 2);
            for yy in 0..ch {
                for xx in 0..cw {
                    let cov = bmp[(yy * cw + xx) as usize];
                    if cov != 0 {
                        p.put(gx + xx, gy + yy, c, a * cov as f32 / 255.0);
                    }
                }
            }
        }
    }
}

// --------------------------------------------------------------- painting --

const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

/// A framebuffer of premultiplied `Argb8888` and the window's origin in it.
struct Paint<'a> {
    fb: &'a mut [u32],
    w: usize,
    h: usize,
    ox: i32,
    oy: i32,
}

impl Paint<'_> {
    /// Colour `c` at alpha `a` over window pixel `(x, y)`; outside the framebuffer is ignored.
    fn put(&mut self, x: i32, y: i32, c: [f32; 3], a: f32) {
        let (fx, fy) = (x + self.ox, y + self.oy);
        if a <= 0.0 || fx < 0 || fy < 0 || fx as usize >= self.w || fy as usize >= self.h {
            return;
        }
        let dst = &mut self.fb[fy as usize * self.w + fx as usize];
        let k = 1.0 - a.min(1.0);
        let ch = |shift: u32, src: f32| {
            let d = (*dst >> shift & 255) as f32;
            ((src * a * 255.0 + d * k + 0.5) as u32).min(255)
        };
        let alpha = ((a * 255.0 + (*dst >> 24) as f32 * k + 0.5) as u32).min(255);
        *dst = alpha << 24 | ch(16, c[0]) << 16 | ch(8, c[1]) << 8 | ch(0, c[2]);
    }

    /// Fills the box `x0..x1` x `y0..y1` with `c` at `a` times `cov(x, y)`, sampled at pixel centres.
    fn shape(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: [f32; 3], a: f32, cov: &dyn Fn(f32, f32) -> f32) {
        for y in y0..y1 {
            for x in x0..x1 {
                let k = cov(x as f32 + 0.5, y as f32 + 0.5);
                if k > 0.0 {
                    self.put(x, y, c, a * k);
                }
            }
        }
    }

    fn rrect(&mut self, x: i32, y: i32, w: i32, h: i32, r: f32, c: [f32; 3], a: f32) {
        let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
        self.shape(x, y, x + w, y + h, c, a, &|px, py| (0.5 - chrome::rounded_rect(px - fx, py - fy, fw, fh, r.min(fh / 2.0))).clamp(0.0, 1.0));
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32, c: [f32; 3], a: f32) {
        let (x0, y0, x1, y1) = ((cx - r - 1.0) as i32, (cy - r - 1.0) as i32, (cx + r + 2.0) as i32, (cy + r + 2.0) as i32);
        self.shape(x0, y0, x1, y1, c, a, &|px, py| (0.5 - ((px - cx).hypot(py - cy) - r)).clamp(0.0, 1.0));
    }

    /// A `<` (`dir` -1) or `>` (+1) about 10 px tall, 1.4 px stroke, centred on `(cx, cy)`.
    fn chevron(&mut self, cx: i32, cy: i32, dir: i32, c: [f32; 3], a: f32) {
        let (cx, cy, d) = (cx as f32, cy as f32, dir as f32);
        let tip = (cx + 2.0 * d, cy);
        let (top, bot) = ((cx - 2.5 * d, cy - 5.0), (cx - 2.5 * d, cy + 5.0));
        self.shape(cx as i32 - 7, cy as i32 - 8, cx as i32 + 8, cy as i32 + 9, c, a, &|px, py| {
            let dist = seg(px, py, top, tip).min(seg(px, py, tip, bot));
            (1.2 - dist).clamp(0.0, 1.0)
        });
    }
}

fn seg(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - a.0 - dx * t).hypot(py - a.1 - dy * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xkb::Mods;

    const WW: u32 = 900;
    const WH: u32 = 520;

    fn fams() -> Vec<(String, PathBuf)> {
        vec![("Alpha Mono".into(), "/fonts/Alpha.ttf".into()), ("Beta Mono".into(), "/fonts/Beta.ttf".into())]
    }

    fn open() -> Settings {
        Settings::new(&FontCfg::default(), 0.88, fams())
    }

    fn key(sym: u32, shift: bool) -> Key {
        Key { sym, text: String::new(), mods: Mods { shift, ctrl: false, alt: false } }
    }

    /// Window pixel of a point `(x, y)` inside the panel.
    fn at(x: i32, y: i32) -> (f32, f32) {
        let (ox, oy) = origin(WW as i32, WH as i32);
        ((ox + x) as f32 + 0.5, (oy + y) as f32 + 0.5)
    }

    #[test]
    fn the_panel_is_centred_under_the_title_bar_and_fits_the_default_window() {
        let (x, y) = origin(1000, 640);
        assert_eq!(x * 2 + PANEL_W, 1000);
        assert!(y >= TITLE_H as i32 && y + PANEL_H <= 640);
        // A window shorter than the panel keeps its top edge below the title bar.
        assert!(origin(900, 300).1 >= TITLE_H as i32 + 8);
    }

    #[test]
    fn hit_testing_finds_sliders_arrows_switch_and_buttons() {
        let s = open();
        let h = |x, y| {
            let (fx, fy) = at(x, y);
            s.hit(fx, fy, WW, WH)
        };
        let cy = |row: Row| row_top(row.index()) + ROW / 2;
        assert!(matches!(h(CTRL_X + TRACK_W / 2, cy(Row::Size)), Hit::Slider(Row::Size, t) if (t - 0.5).abs() < 0.01));
        assert!(matches!(h(CTRL_X - 4, cy(Row::Opacity)), Hit::Slider(Row::Opacity, t) if t == 0.0));
        assert_eq!(h(CTRL_X + 5, cy(Row::Family)), Hit::Arrow(-1));
        assert_eq!(h(PANEL_W - PAD - 5, cy(Row::Family)), Hit::Arrow(1));
        assert_eq!(h(CTRL_X + 10, cy(Row::Bold)), Hit::Toggle);
        assert_eq!(h(PAD + 5, button_rect(Hit::Defaults).1 + 5), Hit::Defaults);
        assert_eq!(h(PANEL_W - PAD - BTN_W - 10 - 5, button_rect(Hit::Cancel).1 + 5), Hit::Cancel);
        assert_eq!(h(PANEL_W - PAD - 5, button_rect(Hit::Save).1 + 5), Hit::Save);
        assert_eq!(h(PAD, 10), Hit::Panel);
        assert_eq!(h(-5, 100), Hit::Outside);
        assert_eq!(h(PANEL_W + 5, 100), Hit::Outside);
    }

    #[test]
    fn dragging_a_slider_sets_snapped_values_and_stops_on_release() {
        let mut s = open();
        let cy = row_top(Row::Size.index()) + ROW / 2;
        let (x, y) = at(CTRL_X, cy);
        assert_eq!(s.press(x, y, WW, WH), Effect::Apply); // 15 -> the track's left end: 8
        assert_eq!(s.draft.font.size, 8.0);
        let (x, y) = at(CTRL_X + TRACK_W, cy);
        assert_eq!(s.pointer(x + 300.0, y, WW, WH), Effect::Apply); // past the end: clamped
        assert_eq!(s.draft.font.size, 40.0);
        let (x, _) = at(CTRL_X + TRACK_W / 2, cy);
        s.pointer(x, y + 200.0, WW, WH); // the pointer may leave the row while dragging
        assert_eq!(s.draft.font.size, 24.0);
        s.release();
        let before = s.draft.clone();
        let (x, y) = at(CTRL_X, cy);
        assert_ne!(s.pointer(x, y, WW, WH), Effect::Apply);
        assert_eq!(s.draft, before);
    }

    #[test]
    fn values_snap_to_their_step_and_print_cleanly() {
        let mut d = Draft::defaults();
        d.set(Row::Opacity, 0.8765);
        assert_eq!(d.opacity, 0.88);
        d.set(Row::Weight, 447.0);
        assert_eq!(d.font.weight, 450);
        d.set(Row::Line, 1.234);
        assert_eq!(d.font.line_height, 1.25);
        d.set(Row::Spacing, 0.3);
        assert_eq!(d.text(Row::Spacing), "+0.5");
        d.set(Row::Spacing, 0.0);
        assert_eq!(d.text(Row::Spacing), "0");
        d.set(Row::Size, 1000.0);
        assert_eq!(d.font.size, 40.0);
        assert_eq!(d.text(Row::Opacity), "0.88");
    }

    #[test]
    fn the_keyboard_moves_between_rows_and_adjusts_values() {
        let mut s = open();
        assert_eq!(s.key(&key(sym::DOWN, false)), Effect::Redraw);
        assert_eq!(ALL[s.sel], Row::Size);
        assert_eq!(s.key(&key(sym::RIGHT, false)), Effect::Apply);
        assert_eq!(s.draft.font.size, 16.0);
        s.key(&key(sym::LEFT, true)); // shift: five steps
        assert_eq!(s.draft.font.size, 11.0);
        s.key(&key(sym::UP, false));
        s.key(&key(sym::UP, false)); // wraps to the last row, opacity
        assert_eq!(ALL[s.sel], Row::Opacity);
        s.key(&key(sym::LEFT, false));
        assert_eq!(s.draft.opacity, 0.86);
        // Space flips the switch only on its row.
        assert_eq!(s.key(&key(0x20, false)), Effect::None);
        s.sel = Row::Bold.index();
        s.key(&key(0x20, false));
        assert!(!s.draft.font.bold);
        assert_eq!(s.key(&key(sym::ESCAPE, false)), Effect::Cancel);
        assert_eq!(s.key(&key(sym::RETURN, false)), Effect::Save);
        assert_eq!(s.key(&key(0x61, false)), Effect::None);
    }

    #[test]
    fn the_font_picker_cycles_through_default_and_the_installed_families() {
        let mut s = open();
        assert_eq!(s.family_label(), "Default (JetBrains Mono)");
        s.pick(1);
        assert_eq!(s.family_label(), "Alpha Mono");
        assert!(s.draft.font.family.is_some());
        s.pick(1);
        s.pick(1);
        assert_eq!(s.family_label(), "Default (JetBrains Mono)", "wraps after the last family");
        assert_eq!(s.draft.font.family, None);
        s.pick(-1);
        assert_eq!(s.family_label(), "Beta Mono");
    }

    #[test]
    fn a_configured_font_the_scan_did_not_list_still_has_a_stop() {
        let cfg = FontCfg { family: Some("/home/me/fonts/Odd.ttf".into()), ..FontCfg::default() };
        let s = Settings::new(&cfg, 0.88, fams());
        assert_eq!(s.family_label(), "Odd.ttf");
        let cfg = FontCfg { family: Some("beta mono".into()), ..FontCfg::default() };
        assert_eq!(Settings::new(&cfg, 0.88, fams()).family_label(), "Beta Mono");
    }

    #[test]
    fn changes_lists_only_what_differs_and_defaults_resets_everything() {
        let mut s = open();
        assert!(s.changes().is_empty());
        s.draft.set(Row::Weight, 600.0);
        s.draft.font.bold = false;
        s.pick(2);
        let keys: Vec<&str> = s.changes().iter().map(|c| c.0).collect();
        assert_eq!(keys, ["font.family", "font.weight", "font.bold"]);
        assert_eq!(s.changes()[1].1.as_deref(), Some("600"));
        s.saved();
        assert!(s.changes().is_empty());
        let (x, y) = at(button_rect(Hit::Defaults).0 + 3, button_rect(Hit::Defaults).1 + 3);
        assert_eq!(s.press(x, y, WW, WH), Effect::Apply);
        assert_eq!(s.draft, Draft::defaults());
        assert_eq!(s.family_label(), "Default (JetBrains Mono)");
        assert!(s.changes().iter().any(|c| c.0 == "font.weight"), "reset values count as changes against the saved ones");
    }

    #[test]
    fn the_panel_is_painted_over_the_grid_and_leaves_the_title_bar_alone() {
        let (ml, mt, mb) = chrome::margins(false);
        let (w, h) = (WW as usize + 2 * ml, WH as usize + mt + mb);
        let glass = 0xe0101010u32;
        let mut fb = vec![glass; w * h];
        let s = open();
        s.draw(&mut fb, w, h, (ml, mt), (WW, WH), false);
        let px = |x: usize, y: usize| fb[(mt + y) * w + ml + x];
        assert_eq!(px(300, 10), glass, "the title bar is untouched");
        assert_eq!(fb[0], glass, "so is the shadow margin");
        assert_ne!(px(5, 300), glass, "the grid is dimmed");
        assert!(px(5, 300) >> 24 >= glass >> 24, "dimming never makes the window more transparent than it was");
        let (ox, oy) = origin(WW as i32, WH as i32);
        let body = px((ox + 6) as usize, (oy + 200) as usize);
        assert!(body >> 24 > 0xe0, "the panel is nearly opaque: {body:08x}");
        // Premultiplied: no channel above alpha, anywhere.
        for &p in &fb {
            assert!((p >> 16 & 255) <= (p >> 24) + 1 && (p >> 8 & 255) <= (p >> 24) + 1 && (p & 255) <= (p >> 24) + 1);
        }
    }

    fn typed(text: &str) -> Key {
        Key { sym: 0, text: text.into(), mods: Mods::default() }
    }

    fn type_all(s: &mut Settings, text: &str) {
        for c in text.chars() {
            s.key(&typed(&c.to_string()));
        }
    }

    fn value_point(row: Row) -> (f32, f32) {
        at(VALUE_X + 10, row_top(row.index()) + ROW / 2)
    }

    #[test]
    fn clicking_a_number_opens_its_box_with_the_value_selected() {
        let mut s = open();
        let (x, y) = value_point(Row::Size);
        assert!(matches!(s.hit(x, y, WW, WH), Hit::Value(Row::Size)));
        assert_eq!(s.press(x, y, WW, WH), Effect::Redraw);
        assert_eq!(s.edit, Some(Edit { row: Row::Size, text: "15".into(), selected: true }));
        assert_eq!(s.sel, Row::Size.index());
        // The first key replaces the selected number; later ones add to it.
        type_all(&mut s, "2");
        assert_eq!(s.edit.as_ref().unwrap().text, "2");
        type_all(&mut s, "4");
        assert_eq!(s.key(&key(sym::RETURN, false)), Effect::Apply, "Enter commits the box, it does not save");
        assert_eq!(s.draft.font.size, 24.0);
        assert!(s.edit.is_none());
    }

    #[test]
    fn typed_numbers_may_go_past_the_slider_but_not_the_files_limits() {
        let mut s = open();
        s.sel = Row::Size.index();
        type_all(&mut s, "60"); // the slider ends at 40, the file allows 96
        s.key(&key(sym::RETURN, false));
        assert_eq!(s.draft.font.size, 60.0);
        s.sel = Row::Size.index();
        type_all(&mut s, "500");
        s.key(&key(sym::RETURN, false));
        assert_eq!(s.draft.font.size, 96.0);
        // The knob is pinned at the end of its track while the number is honest.
        assert_eq!(s.draft.text(Row::Size), "96");
    }

    #[test]
    fn decimals_signs_and_weights_are_read_as_written() {
        let mut s = open();
        let enter = |s: &mut Settings, row: Row, text: &str| {
            s.sel = row.index();
            type_all(s, text);
            s.key(&key(sym::RETURN, false));
        };
        enter(&mut s, Row::Size, "17.25");
        assert_eq!(s.draft.font.size, 17.25);
        enter(&mut s, Row::Spacing, "-1,5"); // a decimal comma is fine
        assert_eq!(s.draft.font.letter_spacing, -1.5);
        enter(&mut s, Row::Spacing, "-9");
        assert_eq!(s.draft.font.letter_spacing, -8.0, "clamped to the file's limit");
        enter(&mut s, Row::Weight, "447");
        assert_eq!(s.draft.font.weight, 450);
        enter(&mut s, Row::Line, "1.337");
        assert_eq!(s.draft.font.line_height, 1.337);
        enter(&mut s, Row::Opacity, ".5");
        assert_eq!(s.draft.opacity, 0.5);
        enter(&mut s, Row::Opacity, "0");
        assert_eq!(s.draft.opacity, 0.3);
    }

    #[test]
    fn junk_is_ignored_while_typing_and_an_unreadable_box_changes_nothing() {
        let mut s = open();
        s.sel = Row::Size.index();
        type_all(&mut s, "1a2.3.4-5");
        assert_eq!(s.edit.as_ref().unwrap().text, "12.345", "letters, a second dot and a late minus are dropped");
        type_all(&mut s, "6789");
        assert_eq!(s.edit.as_ref().unwrap().text.len(), EDIT_MAX);
        s.key(&key(sym::BACKSPACE, false));
        assert_eq!(s.edit.as_ref().unwrap().text, "12.345", "the seventh character was kept, Backspace removed it");
        let before = s.draft.clone();
        s.edit = None;
        for bad in ["-", ".", ""] {
            s.sel = Row::Size.index();
            s.start_edit(Row::Size, None);
            s.edit.as_mut().unwrap().text = bad.into();
            assert_eq!(s.key(&key(sym::RETURN, false)), Effect::Redraw, "{bad:?}");
            assert_eq!(s.draft, before);
        }
    }

    #[test]
    fn escape_ends_the_box_not_the_screen_and_other_ways_out_keep_the_number() {
        let mut s = open();
        s.sel = Row::Weight.index();
        type_all(&mut s, "700");
        assert_eq!(s.key(&key(sym::ESCAPE, false)), Effect::Redraw);
        assert!(s.edit.is_none());
        assert_eq!(s.draft.font.weight, 400, "Escape discards what was typed");
        assert_eq!(s.key(&key(sym::ESCAPE, false)), Effect::Cancel, "with no box open it cancels the screen");
        // Tab commits and moves on; so does a click anywhere; so does Save.
        type_all(&mut s, "700");
        assert_eq!(s.key(&key(sym::TAB, false)), Effect::Apply);
        assert_eq!((s.draft.font.weight, ALL[s.sel]), (700, Row::Bold));
        s.sel = Row::Line.index();
        type_all(&mut s, "1.5");
        let (x, y) = at(PAD, 10);
        assert_eq!(s.press(x, y, WW, WH), Effect::Apply, "a click elsewhere keeps the number");
        assert_eq!(s.draft.font.line_height, 1.5);
        s.sel = Row::Size.index();
        type_all(&mut s, "20");
        assert_eq!(s.commit().font.size, 20.0, "and so does saving");
    }

    #[test]
    fn digits_do_nothing_on_the_font_and_switch_rows_and_keys_with_ctrl_are_not_typing() {
        let mut s = open();
        assert_eq!(s.key(&typed("5")), Effect::None); // the font row
        s.sel = Row::Bold.index();
        assert_eq!(s.key(&typed("5")), Effect::None);
        s.sel = Row::Size.index();
        let ctrl = Key { sym: 0, text: "5".into(), mods: Mods { ctrl: true, shift: false, alt: false } };
        assert_eq!(s.key(&ctrl), Effect::None);
        assert!(s.edit.is_none());
        // Clicking the same box twice does not reopen it over the typed text.
        let (x, y) = value_point(Row::Size);
        s.press(x, y, WW, WH);
        type_all(&mut s, "33");
        s.press(x, y, WW, WH);
        assert_eq!(s.draft.font.size, 33.0);
        assert!(s.edit.is_none());
    }

    #[test]
    fn a_box_being_typed_is_painted() {
        let (ml, mt, mb) = chrome::margins(false);
        let (w, h) = (WW as usize + 2 * ml, WH as usize + mt + mb);
        let frame = |s: &Settings| {
            let mut fb = vec![0xe0101010u32; w * h];
            s.draw(&mut fb, w, h, (ml, mt), (WW, WH), false);
            fb
        };
        let mut s = open();
        let plain = frame(&s);
        s.sel = Row::Size.index();
        type_all(&mut s, "2");
        assert_ne!(frame(&s), plain);
    }
}
