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
    bar,
    config::{self, Chip, FontCfg, InputCfg},
    i18n::{self, Str, tr},
    font::{self, Fonts, UiFont},
    input::sym,
    paint::{BLACK, Paint, WHITE},
    palette,
    xkb::Key,
};

// ----------------------------------------------------------------- layout --

pub const PANEL_W: i32 = 500;
const PAD: i32 = 24;
/// The live preview under the title row, then the strip of tabs (a rounded container with a
/// sliding marker), then the rows.
const PREVIEW_Y: i32 = 44;
const PREVIEW_H: i32 = 58;
const TAB_Y: i32 = 114;
const TAB_W: i32 = 80;
const TAB_H: i32 = 26;
const HEAD: i32 = 152;
/// How long the panel takes to appear, a hover to fade and the tab marker to slide, in seconds.
const FADE_IN: f32 = 0.16;
const FADE_HOVER: f32 = 0.12;
const SLIDE: f32 = 0.22;

fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Size of the labels' text, in pixels.
const UI_PX: f32 = 13.5;
const ROW: i32 = 38;
/// Room for the rows of the fullest page; the footer follows.
const BODY: i32 = 346;
const FOOT: i32 = 60;
/// Left edge of the controls, and the width of a slider track.
const CTRL_X: i32 = 160;
const TRACK_W: i32 = 240;
const ARROW_W: i32 = 30;
/// Where a slider row's number sits: its box starts here and ends at the panel's padding.
const VALUE_X: i32 = CTRL_X + TRACK_W + 14;
const BTN_W: i32 = 92;
const BTN_H: i32 = 34;
const PILL: (i32, i32) = (38, 22);

pub const ROWS: usize = 9;
const PANEL_H: i32 = HEAD + BODY + FOOT;
/// The input page: options side by side, and a grid of chips three rows tall.
const CHOICE_W: i32 = 150;
const CHIP_W: i32 = 101;
const CHIP_H: i32 = 28;
const CHIP_GAP: i32 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Font,
    Input,
}

impl Page {
    const ALL: [Page; 2] = [Page::Font, Page::Input];

    fn label(self) -> &'static str {
        match self {
            Page::Font => tr(Str::TabFont),
            Page::Input => tr(Str::TabInput),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    Family,
    Size,
    Weight,
    Bold,
    Spacing,
    Line,
    Opacity,
    /// Whether the compositor blurs what is behind the glass.
    Blur,
    /// The language of the interface.
    Language,
    /// Input type: the shell's own prompt, or the context bar.
    Mode,
    Clock,
    /// The shape of the cursor.
    Cursor,
    Chips,
}

const FONT_ROWS: [Row; ROWS] = [Row::Family, Row::Size, Row::Weight, Row::Bold, Row::Spacing, Row::Line, Row::Opacity, Row::Blur, Row::Language];
const INPUT_ROWS: [Row; 4] = [Row::Mode, Row::Clock, Row::Cursor, Row::Chips];

fn rows(page: Page) -> &'static [Row] {
    match page {
        Page::Font => &FONT_ROWS,
        Page::Input => &INPUT_ROWS,
    }
}

/// How tall a row is: the chip grid takes three.
fn row_h(row: Row) -> i32 {
    if row == Row::Chips { 3 * ROW } else { ROW }
}

fn row_top(page: Page, i: usize) -> i32 {
    HEAD + rows(page)[..i].iter().map(|&r| row_h(r)).sum::<i32>()
}

/// The options of a choice row, in order, and how many there are.
fn options(row: Row) -> ([&'static str; 3], usize) {
    match row {
        Row::Mode => ([tr(Str::OptShell), tr(Str::OptChips), ""], 2),
        Row::Cursor => ([tr(Str::OptBlock), tr(Str::OptBar), tr(Str::OptUnderline)], 3),
        Row::Language => ([tr(Str::OptAuto), tr(Str::OptEnglish), tr(Str::OptSpanish)], 3),
        _ => ([tr(Str::Opt12), tr(Str::Opt24), ""], 2),
    }
}

/// How wide an option of a choice row is (they sit side by side).
fn choice_w(row: Row) -> i32 {
    if matches!(row, Row::Cursor | Row::Language) { 100 } else { CHOICE_W }
}

impl Row {
    fn label(self) -> &'static str {
        tr(match self {
            Row::Family => Str::RowFont,
            Row::Size => Str::RowSize,
            Row::Weight => Str::RowWeight,
            Row::Bold => Str::RowBold,
            Row::Spacing => Str::RowSpacing,
            Row::Line => Str::RowLine,
            Row::Opacity => Str::RowOpacity,
            Row::Blur => Str::RowBlur,
            Row::Language => Str::RowLanguage,
            Row::Mode => Str::RowMode,
            Row::Clock => Str::RowClock,
            Row::Cursor => Str::RowCursor,
            Row::Chips => Str::RowChips,
        })
    }

    fn page(self) -> Page {
        if matches!(self, Row::Mode | Row::Clock | Row::Cursor | Row::Chips) { Page::Input } else { Page::Font }
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
            _ => (0.0, 0.0, 0.0),
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
            _ => (0.0, 0.0),
        }
    }

    /// Whether the row is a slider with a number (the others are a picker, a switch, choices, chips).
    fn is_number(self) -> bool {
        !matches!(self, Row::Family | Row::Bold | Row::Blur | Row::Language | Row::Mode | Row::Clock | Row::Cursor | Row::Chips)
    }

    /// Its place in its page.
    fn index(self) -> usize {
        rows(self.page()).iter().position(|&r| r == self).unwrap_or(0)
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
    /// A switch row's pill: bold text, blur.
    Toggle(Row),
    /// A page's tab.
    Tab(Page),
    /// Option `n` of a choice row.
    Choice(Row, usize),
    /// A chip's switch.
    Chip(Chip),
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
    let room = wh - TITLE_H as i32 - chrome::STATUS_H as i32;
    ((ww - PANEL_W) / 2, (TITLE_H as i32 + (room - PANEL_H) / 2).max(TITLE_H as i32 + 8))
}

fn tab_rect(page: Page) -> (i32, i32, i32, i32) {
    (PAD + 3 + Page::ALL.iter().position(|&p| p == page).unwrap_or(0) as i32 * TAB_W, TAB_Y + 3, TAB_W, TAB_H)
}

/// Where slot `i` of the chip grid is, relative to the panel, on the input page. The grid lists the
/// chips in the order the bar shows them, then the ones that are off.
fn slot_rect(i: usize) -> (i32, i32, i32, i32) {
    let (r, col) = (i as i32 / 3, i as i32 % 3);
    let top = row_top(Page::Input, 3) + (3 * ROW - (3 * CHIP_H + 2 * CHIP_GAP)) / 2;
    (CTRL_X + col * (CHIP_W + CHIP_GAP), top + r * (CHIP_H + CHIP_GAP), CHIP_W, CHIP_H)
}

fn button_rect(which: Hit) -> (i32, i32, i32, i32) {
    let y = PANEL_H - FOOT + (FOOT - BTN_H) / 2;
    let x = match which {
        Hit::Defaults => PAD,
        Hit::Cancel => PANEL_W - PAD - 2 * BTN_W - 10,
        _ => PANEL_W - PAD - BTN_W,
    };
    (x, y, if which == Hit::Defaults { 96 } else { BTN_W }, BTN_H)
}

fn inside(x: i32, y: i32, (rx, ry, rw, rh): (i32, i32, i32, i32)) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

/// The name of a chip in the grid, in the language in use (the file always says `chip.name()`).
fn chip_label(chip: Chip) -> &'static str {
    tr(match chip {
        Chip::Time => Str::ChipTime,
        Chip::User => Str::ChipUser,
        Chip::Env => Str::ChipEnv,
        Chip::Pyenv => Str::ChipPyenv,
        Chip::Host => Str::ChipHost,
        Chip::Dir => Str::ChipDir,
        Chip::Branch => Str::ChipBranch,
        Chip::Changes => Str::ChipChanges,
        Chip::Kube => Str::ChipKube,
    })
}

// ------------------------------------------------------------------ state --

/// A chip held by the pointer: pressed on it, and moved far enough to be a drag rather than a click.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ChipDrag {
    chip: Chip,
    /// Where it was pressed and where the pointer is, relative to the panel.
    from: (i32, i32),
    at: (i32, i32),
    moved: bool,
}

/// How far the pointer must go before a press on a chip is a drag and not a click.
const DRAG_SLOP: i32 = 5;

/// The values being edited.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub font: FontCfg,
    pub opacity: f32,
    pub input: InputCfg,
    pub blur: bool,
    pub lang: i18n::Setting,
}

impl Draft {
    fn defaults() -> Draft {
        Draft { font: FontCfg::default(), opacity: chrome::OPACITY, input: InputCfg::default(), blur: false, lang: i18n::Setting::Auto }
    }

    fn get(&self, row: Row) -> f32 {
        match row {
            Row::Size => self.font.size,
            Row::Weight => self.font.weight as f32,
            Row::Spacing => self.font.letter_spacing,
            Row::Line => self.font.line_height,
            Row::Opacity => self.opacity,
            _ => 0.0,
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
            _ => {}
        }
    }

    fn text(&self, row: Row) -> String {
        let v = self.get(row);
        match row {
            Row::Size | Row::Weight => format!("{v:.0}"),
            Row::Spacing => format!("{v:+.1}").replace("+0.0", "0").replace("-0.0", "0"),
            Row::Line | Row::Opacity => format!("{v:.2}"),
            _ => String::new(),
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
    pub page: Page,
    /// The selected row of the page, and on the input page the selected chip of the grid.
    pub sel: usize,
    /// The slot of the chip grid the keyboard is on.
    chip_sel: usize,
    /// A chip picked up with the pointer.
    chip_drag: Option<ChipDrag>,
    pub hover: Hit,
    /// What was under the pointer before `hover`, and how far (0 to 1) the change has faded in.
    prev: Hit,
    mix: f32,
    /// How visible the panel is: 0 when it has just opened, 1 once it has faded in.
    fade: f32,
    /// The tab marker slides: it left tab `ind_from` (a fractional index) and is `ind_t` (0 to 1) of the way.
    ind_from: f32,
    ind_t: f32,
    /// When the animation was last advanced.
    clock: Option<std::time::Instant>,
    drag: Option<Row>,
    /// A number being typed.
    edit: Option<Edit>,
    ui: Fonts,
    /// The proportional font the labels are drawn in, when the machine has one (`open` loads it).
    prop: Option<UiFont>,
    /// The compositor can blur behind the window: when it cannot, the switch says so.
    blur_supported: bool,
    /// A line for the header instead of its hint (a save that failed).
    pub notice: Option<String>,
}

impl Settings {
    /// Opens the screen on the current look, listing the fonts installed.
    pub fn open(font: &FontCfg, opacity: f32, input: &InputCfg) -> Settings {
        Settings::new(font, opacity, font::monospaced_families()).with_input(input).with_ui_font()
    }

    /// Draws the labels in the desktop's proportional font, if the machine has one.
    pub fn with_ui_font(mut self) -> Settings {
        self.prop = UiFont::load();
        self
    }

    /// The input settings the screen opens with (it starts from the defaults).
    pub fn with_input(mut self, input: &InputCfg) -> Settings {
        self.draft.input = input.clone();
        self.orig.input = input.clone();
        self
    }

    /// The language setting the screen opens with.
    pub fn with_lang(mut self, lang: i18n::Setting) -> Settings {
        self.draft.lang = lang;
        self.orig.lang = lang;
        self
    }

    /// The blur setting the screen opens with, and whether the compositor offers blur at all.
    pub fn with_blur(mut self, on: bool, supported: bool) -> Settings {
        self.draft.blur = on;
        self.orig.blur = on;
        self.blur_supported = supported;
        self
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
        let draft = Draft { font: font.clone(), opacity, input: InputCfg::default(), blur: false, lang: i18n::Setting::Auto };
        Settings {
            orig: draft.clone(),
            draft,
            families,
            fam,
            page: Page::Font,
            sel: 0,
            chip_sel: 0,
            chip_drag: None,
            hover: Hit::Outside,
            prev: Hit::Outside,
            mix: 1.0,
            fade: 1.0,
            ind_from: 0.0,
            ind_t: 1.0,
            clock: None,
            drag: None,
            edit: None,
            // The default family at a fixed size: never the font being edited.
            ui: Fonts::new(&FontCfg::sized(13.0)),
            prop: None,
            blur_supported: true,
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
        if d.lang != o.lang {
            out.push(("ui.language", Some(format!("{:?}", d.lang.name()))));
        }
        if d.blur != o.blur {
            out.push(("window.blur", Some(d.blur.to_string())));
        }
        if d.font.bold != o.font.bold {
            out.push(("font.bold", Some(d.font.bold.to_string())));
        }
        if d.input.inline != o.input.inline {
            out.push(("input.mode", Some(format!("{:?}", if d.input.inline { "chips" } else { "shell" }))));
        }
        if d.input.clock24 != o.input.clock24 {
            out.push(("input.clock", Some(format!("{:?}", if d.input.clock24 { "24h" } else { "12h" }))));
        }
        if d.input.cursor != o.input.cursor {
            out.push(("input.cursor", Some(format!("{:?}", d.input.cursor.name()))));
        }
        if d.input.chips != o.input.chips {
            out.push(("input.chips", Some(config::chip_list_text(&d.input.chips))));
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
            0 => tr(Str::DefaultFont),
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
        for page in Page::ALL {
            if inside(x, y, tab_rect(page)) {
                return Hit::Tab(page);
            }
        }
        for (i, &row) in rows(self.page).iter().enumerate() {
            let top = row_top(self.page, i);
            if y < top || y >= top + row_h(row) {
                continue;
            }
            return match row {
                // Rows that only matter with the chips on are inert without them.
                Row::Clock | Row::Chips if !self.draft.input.inline => Hit::Panel,
                Row::Mode | Row::Clock | Row::Cursor | Row::Language => {
                    let (n, count) = ((x - CTRL_X).div_euclid(choice_w(row)), options(row).1 as i32);
                    if (0..count).contains(&n) { Hit::Choice(row, n as usize) } else { Hit::Panel }
                }
                Row::Chips => (0..Chip::COUNT).find(|&i| inside(x, y, slot_rect(i))).map_or(Hit::Panel, |i| Hit::Chip(self.slots()[i])),
                Row::Family if (CTRL_X..CTRL_X + ARROW_W).contains(&x) => Hit::Arrow(-1),
                Row::Family if (PANEL_W - PAD - ARROW_W..PANEL_W - PAD).contains(&x) => Hit::Arrow(1),
                Row::Bold | Row::Blur if x < CTRL_X + PILL.0 + 8 => Hit::Toggle(row),
                Row::Family | Row::Bold | Row::Blur => Hit::Panel,
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
            self.set_hover(Hit::Slider(row, t));
            return if self.draft != before { Effect::Apply } else { Effect::None };
        }
        if let Some(d) = self.chip_drag {
            let (ox, oy) = origin(ww as i32, wh as i32);
            let at = (x.floor() as i32 - ox, y.floor() as i32 - oy);
            let moved = d.moved || (at.0 - d.from.0).abs().max((at.1 - d.from.1).abs()) > DRAG_SLOP;
            self.chip_drag = Some(ChipDrag { at, moved, ..d });
            if !moved {
                return Effect::None;
            }
            self.set_hover(Hit::Chip(d.chip));
            let before = self.draft.input.chips.clone();
            self.place(d.chip, self.slot_near(at));
            self.chip_sel = self.slots().iter().position(|&c| c == d.chip).unwrap_or(0);
            return if self.draft.input.chips != before { Effect::Apply } else { Effect::Redraw };
        }
        let now = self.hit(x, y, ww, wh);
        // A slider's hover does not depend on where along the track the pointer is.
        let same = |a: Hit, b: Hit| match (a, b) {
            (Hit::Slider(r, _), Hit::Slider(s, _)) => r == s,
            _ => a == b,
        };
        let changed = !same(now, self.hover);
        self.set_hover(now);
        if changed { Effect::Redraw } else { Effect::None }
    }

    /// The primary button went down at `(x, y)`.
    pub fn press(&mut self, x: f32, y: f32, ww: u32, wh: u32) -> Effect {
        let hit = self.hit(x, y, ww, wh);
        self.set_hover(hit);
        // Any press ends a number being typed, keeping it; clicking the same box again starts over.
        let committed = self.commit_edit();
        let again = matches!((hit, committed), (Hit::Value(r), Some((c, _))) if r == c);
        let e = self.press_on(hit, again);
        if let Hit::Chip(chip) = hit {
            let (ox, oy) = origin(ww as i32, wh as i32);
            let at = (x.floor() as i32 - ox, y.floor() as i32 - oy);
            self.chip_drag = Some(ChipDrag { chip, from: at, at, moved: false });
        }
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
            Hit::Toggle(row) => {
                self.sel = row.index();
                self.flip(row);
                Effect::Apply
            }
            Hit::Defaults => {
                self.defaults();
                Effect::Apply
            }
            Hit::Tab(page) => {
                self.set_page(page);
                Effect::Redraw
            }
            Hit::Choice(row, n) => {
                self.sel = row.index();
                self.choose(row, n);
                Effect::Apply
            }
            Hit::Chip(c) => {
                // A click switches the chip; moving first turns the press into a drag (see `pointer`).
                self.sel = Row::Chips.index();
                self.chip_sel = self.slots().iter().position(|&x| x == c).unwrap_or(0);
                Effect::Redraw
            }
            Hit::Cancel => Effect::Cancel,
            Hit::Save => Effect::Save,
            Hit::Panel | Hit::Outside => Effect::None,
        }
    }

    /// Shows another page, with its first row selected.
    fn set_page(&mut self, page: Page) {
        self.commit_edit();
        if page != self.page {
            self.ind_from = self.marker();
            self.ind_t = 0.0;
            self.clock = None;
            self.page = page;
            self.sel = 0;
        }
    }

    /// Option `n` of a choice row.
    fn choose(&mut self, row: Row, n: usize) {
        match row {
            Row::Mode => self.draft.input.inline = n == 1,
            Row::Clock => self.draft.input.clock24 = n == 1,
            Row::Cursor => self.draft.input.cursor = config::CursorKind::ALL[n.min(2)],
            Row::Language => self.draft.lang = i18n::Setting::ALL[n.min(2)],
            _ => {}
        }
    }

    /// The option a choice row is on.
    fn chosen(&self, row: Row) -> usize {
        match row {
            Row::Mode => self.draft.input.inline as usize,
            Row::Cursor => config::CursorKind::ALL.iter().position(|&c| c == self.draft.input.cursor).unwrap_or(0),
            Row::Language => i18n::Setting::ALL.iter().position(|&l| l == self.draft.lang).unwrap_or(0),
            _ => self.draft.input.clock24 as usize,
        }
    }

    /// The chips in the order the grid shows them: the ones on, in the order the bar has them, then the
    /// ones off, in the default order.
    fn slots(&self) -> [Chip; Chip::COUNT] {
        let on = &self.draft.input.chips;
        let mut out = [Chip::Time; Chip::COUNT];
        let off = Chip::ALL.into_iter().filter(|c| !on.contains(c));
        for (slot, c) in out.iter_mut().zip(on.iter().copied().chain(off)) {
            *slot = c;
        }
        out
    }

    /// Adds `c` at the end of the bar, or takes it out.
    fn toggle_chip(&mut self, c: Chip) {
        let chips = &mut self.draft.input.chips;
        match chips.iter().position(|&x| x == c) {
            Some(i) => {
                chips.remove(i);
            }
            None => chips.push(c),
        }
    }

    /// Moves `c` to slot `to` of the grid: among the chips that are on it takes that place (a chip that
    /// was off comes on); into the part of the grid that is off it goes off.
    fn place(&mut self, c: Chip, to: usize) {
        let chips = &mut self.draft.input.chips;
        let at = chips.iter().position(|&x| x == c);
        if to < chips.len() || (to == chips.len() && at.is_none() && !chips.is_empty()) {
            if let Some(i) = at {
                chips.remove(i);
            }
            chips.insert(to.min(chips.len()), c);
        } else if let Some(i) = at {
            if to >= chips.len() {
                chips.remove(i);
            }
        } else if chips.is_empty() && to == 0 {
            chips.push(c);
        }
    }

    /// The slot nearest to panel position `(x, y)`: the pointer may leave the grid while dragging.
    fn slot_near(&self, (x, y): (i32, i32)) -> usize {
        let (x0, y0, w, h) = slot_rect(0);
        let col = ((x - x0).div_euclid(w + CHIP_GAP)).clamp(0, 2);
        let row = ((y - y0).div_euclid(h + CHIP_GAP)).clamp(0, 2);
        (row * 3 + col) as usize
    }

    /// Moves the chip the keyboard is on one place left or right among the ones that are on.
    fn nudge(&mut self, dir: i32) -> Effect {
        let on = self.draft.input.chips.len();
        let from = self.chip_sel;
        let to = from as i32 + dir;
        if from >= on || to < 0 || to as usize >= on {
            return Effect::None;
        }
        self.draft.input.chips.swap(from, to as usize);
        self.chip_sel = to as usize;
        Effect::Apply
    }

    /// Shows chip `chip` picked up and held at panel position `at` (the design board's drag scene).
    #[cfg(test)]
    pub fn hold(&mut self, chip: Chip, at: (i32, i32)) {
        self.chip_drag = Some(ChipDrag { chip, from: at, at, moved: true });
        self.hover = Hit::Chip(chip);
    }

    /// The primary button went up: a drag ends, and a press on a chip that never moved switches it.
    pub fn release(&mut self) -> Effect {
        self.drag = None;
        match self.chip_drag.take() {
            Some(d) if !d.moved => {
                self.toggle_chip(d.chip);
                // The grid reorders (a chip that went off moves to the end): the cursor stays on the chip.
                self.chip_sel = self.slots().iter().position(|&c| c == d.chip).unwrap_or(0);
                Effect::Apply
            }
            Some(_) => Effect::Redraw,
            None => Effect::None,
        }
    }

    // --------------------------------------------------------- keyboard --

    /// A key while the screen is open: it never reaches the shell.
    pub fn key(&mut self, k: &Key) -> Effect {
        if self.edit.is_some() {
            return self.edit_key(k);
        }
        // Typing a number on a slider row starts typing it.
        let row = self.row();
        if row.is_number() && !k.mods.ctrl && !k.mods.alt {
            if k.text.chars().next().is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-')) {
                self.start_edit(row, Some(&k.text));
                return Effect::Redraw;
            }
        }
        let big = if k.mods.shift { 5.0 } else { 1.0 };
        match k.sym {
            sym::ESCAPE => Effect::Cancel,
            sym::RETURN => Effect::Save,
            sym::PAGE_DOWN | sym::PAGE_UP => {
                self.set_page(if k.sym == sym::PAGE_DOWN { Page::Input } else { Page::Font });
                Effect::Redraw
            }
            sym::TAB if k.mods.ctrl => {
                self.set_page(if self.page == Page::Font { Page::Input } else { Page::Font });
                Effect::Redraw
            }
            sym::TAB if k.mods.shift => self.move_sel(-1),
            sym::TAB | sym::DOWN => self.move_sel(1),
            sym::UP => self.move_sel(-1),
            sym::LEFT if k.mods.alt && row == Row::Chips => self.nudge(-1),
            sym::RIGHT if k.mods.alt && row == Row::Chips => self.nudge(1),
            sym::LEFT => self.adjust(-big),
            sym::RIGHT => self.adjust(big),
            0x20 if matches!(self.row(), Row::Bold | Row::Blur | Row::Language | Row::Mode | Row::Clock | Row::Cursor | Row::Chips) => self.space(),
            _ => Effect::None,
        }
    }

    /// Puts what is under the pointer; a change starts the fade between the old and the new look.
    fn set_hover(&mut self, now: Hit) {
        let same = match (now, self.hover) {
            (Hit::Slider(a, _), Hit::Slider(b, _)) => a == b,
            (a, b) => a == b,
        };
        if !same {
            // A change in the middle of a fade starts from what is on screen: the old target is forgotten.
            self.prev = self.hover;
            self.mix = 0.0;
            self.clock = None; // the first frame of the fade is the old look: time counts from the next one
        }
        self.hover = now;
    }

    /// Moves the animations to `now`. True while there is more to show, so the caller asks for another frame.
    pub fn step(&mut self, now: std::time::Instant) -> bool {
        let dt = self.clock.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        self.clock = Some(now);
        self.fade = (self.fade + dt / FADE_IN).min(1.0);
        self.mix = (self.mix + dt / FADE_HOVER).min(1.0);
        self.ind_t = (self.ind_t + dt / SLIDE).min(1.0);
        self.animating()
    }

    pub fn animating(&self) -> bool {
        self.fade < 1.0 || self.mix < 1.0 || self.ind_t < 1.0
    }

    /// Starts the panel invisible, to fade in.
    pub fn start_faded(&mut self) {
        self.fade = 0.0;
        self.clock = None;
    }

    /// Where the tab marker is, as a fractional tab index.
    fn marker(&self) -> f32 {
        let to = Page::ALL.iter().position(|&p| p == self.page).unwrap_or(0) as f32;
        mix(self.ind_from, to, ease(self.ind_t))
    }

    /// How lit (0 to 1) the part of the panel `is` picks out is: 1 under the pointer, fading in and out.
    fn lit(&self, is: &dyn Fn(Hit) -> bool) -> f32 {
        match (is(self.hover), is(self.prev)) {
            (true, true) => 1.0,
            (true, false) => ease(self.mix),
            (false, true) => 1.0 - ease(self.mix),
            _ => 0.0,
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
            self.sel = (self.sel as i32 + d).rem_euclid(rows(self.page).len() as i32) as usize;
        }
        if changed { Effect::Apply } else { Effect::Redraw }
    }

    fn move_sel(&mut self, d: i32) -> Effect {
        self.sel = (self.sel as i32 + d).rem_euclid(rows(self.page).len() as i32) as usize;
        Effect::Redraw
    }

    /// The selected row of the current page.
    fn row(&self) -> Row {
        rows(self.page)[self.sel.min(rows(self.page).len() - 1)]
    }

    /// Flips a switch row.
    fn flip(&mut self, row: Row) {
        match row {
            Row::Bold => self.draft.font.bold = !self.draft.font.bold,
            Row::Blur => self.draft.blur = !self.draft.blur,
            _ => {}
        }
    }

    /// Space on a row: flips the switch, the other choice, the chip under the cursor.
    fn space(&mut self) -> Effect {
        match self.row() {
            Row::Chips => {
                let c = self.slots()[self.chip_sel];
                self.toggle_chip(c);
                self.chip_sel = self.slots().iter().position(|&x| x == c).unwrap_or(0);
            }
            Row::Bold | Row::Blur => self.flip(self.row()),
            row => self.choose(row, (self.chosen(row) + 1) % options(row).1),
        }
        Effect::Apply
    }

    /// Left and right on the selected row: `n` steps of its slider, the next font, the switch.
    fn adjust(&mut self, n: f32) -> Effect {
        let row = self.row();
        match row {
            Row::Family => self.pick(n.signum() as i32),
            Row::Bold | Row::Blur => self.flip(row),
            Row::Mode | Row::Clock | Row::Cursor | Row::Language => {
                let to = (self.chosen(row) as i32 + n.signum() as i32).clamp(0, options(row).1 as i32 - 1);
                self.choose(row, to as usize)
            }
            Row::Chips => {
                self.chip_sel = (self.chip_sel as i32 + n.signum() as i32).rem_euclid(Chip::COUNT as i32) as usize;
                return Effect::Redraw;
            }
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
        let mut p = Paint { fb, w, h, ox: ox as i32, oy: oy as i32, fade: ease(self.fade) };
        let (ww, wh) = (ww as i32, wh as i32);
        let (fg, dim, accent) = (palette::unit(palette::FG), palette::unit(palette::DIM), palette::unit(palette::ACCENT));
        const BODY_RGB: [f32; 3] = [0.078, 0.078, 0.086]; // #141416
        const OFF: [f32; 3] = [0.173, 0.173, 0.192]; // #2c2c31: a switch or a slider track at rest
        const RING: [f32; 3] = [0.353, 0.353, 0.384]; // #5a5a62: an unchosen radio

        // The grid and the status bar, dimmed; the title bar and its buttons stay as they are.
        let r = if square { 0.0 } else { chrome::RADIUS };
        p.shape(0, TITLE_H as i32, ww, wh, [0.024, 0.024, 0.027], 0.5, &|x, y| {
            if y < wh as f32 - r { 1.0 } else { (0.5 - chrome::rounded_rect(x, y, ww as f32, wh as f32, r)).clamp(0.0, 1.0) }
        });

        let (px, py) = origin(ww, wh);
        let (pw, ph) = (PANEL_W as f32, PANEL_H as f32);
        const RADIUS: f32 = 16.0;
        // Shadow, body, outline, the light inside the top edge.
        p.shape(px - 30, py - 22, px + PANEL_W + 30, py + PANEL_H + 40, BLACK, 0.45, &|x, y| {
            let d = chrome::rounded_rect(x - px as f32, y - py as f32 - 8.0, pw, ph, RADIUS);
            let t = ((d + 6.0) / 30.0).clamp(0.0, 1.0);
            (1.0 - t).powi(3) * (0.5 + 0.5 * (d / 6.0).clamp(0.0, 1.0)).min(1.0)
        });
        let body = |x: f32, y: f32| (0.5 - chrome::rounded_rect(x - px as f32, y - py as f32, pw, ph, RADIUS)).clamp(0.0, 1.0);
        p.shape(px, py, px + PANEL_W, py + PANEL_H, BODY_RGB, 1.0, &body);
        p.shape(px, py, px + PANEL_W, py + PANEL_H, WHITE, 0.09, &|x, y| {
            let d = chrome::rounded_rect(x - px as f32, y - py as f32, pw, ph, RADIUS);
            (0.5 - d).clamp(0.0, 1.0) * (1.5 + d).clamp(0.0, 1.0)
        });
        p.shape(px, py, px + PANEL_W, py + 2, WHITE, 0.05, &|x, y| if y - (py as f32) >= 1.0 { body(x, y + 1.0) } else { 0.0 });

        let t = |p: &mut Paint, x: i32, cy: i32, s: &str, c: [f32; 3], a: f32, bold: bool| self.text(p, px + x, py + cy, s, c, a, bold);
        t(&mut p, PAD, 26, tr(Str::SettingsTitle), fg, 1.0, true);
        let (head, head_a) = match &self.notice {
            Some(n) => (n.as_str(), 0.95),
            None if self.page == Page::Font => (tr(Str::HintFont), 1.0),
            None => (tr(Str::HintInput), 1.0),
        };
        // As much of it as fits beside the title.
        let mut head: String = head.to_string();
        while self.width(&head) > PANEL_W - 2 * PAD - 90 && head.chars().count() > 1 {
            head = head.chars().take(head.chars().count() - 2).chain(['\u{2026}']).collect();
        }
        let head_c = if self.notice.is_some() { fg } else { dim };
        self.text(&mut p, px + PANEL_W - PAD - self.width(&head), py + 26, &head, head_c, head_a, false);

        // The tabs: a rounded container with a marker that slides to the page being shown.
        let tabs_w = Page::ALL.len() as i32 * TAB_W + 6;
        p.rrect(px + PAD, py + TAB_Y, tabs_w, TAB_H + 6, 10.0, WHITE, 0.05);
        p.rrect(px + PAD + 3 + (self.marker() * TAB_W as f32).round() as i32, py + TAB_Y + 3, TAB_W, TAB_H, 8.0, WHITE, 0.10);
        for page in Page::ALL {
            let (x, y, tw, th) = tab_rect(page);
            let (on, lit) = (page == self.page, self.lit(&|h| h == Hit::Tab(page)));
            let label = page.label();
            let lw = self.width(label);
            let c = if on { fg } else { [mix(dim[0], fg[0], lit), mix(dim[1], fg[1], lit), mix(dim[2], fg[2], lit)] };
            self.text(&mut p, px + x + (tw - lw) / 2, py + y + th / 2, label, c, 1.0, false);
        }

        let shell = !self.draft.input.inline;
        for (i, &row) in rows(self.page).iter().enumerate() {
            let top = py + row_top(self.page, i);
            let cy = top + ROW / 2;
            let inert = shell && matches!(row, Row::Clock | Row::Chips);
            let hovered = if inert { 0.0 } else { self.lit(&|h| {
                matches!(h, Hit::Slider(r, _) | Hit::Value(r) | Hit::Choice(r, _) if r == row)
                    || (row == Row::Chips && matches!(h, Hit::Chip(_)))
                    || (row == Row::Family && matches!(h, Hit::Arrow(_)))
                    || (matches!(row, Row::Bold | Row::Blur) && h == Hit::Toggle(row))
            }) };
            let a = if i == self.sel && !inert { 0.07 } else { 0.04 * hovered };
            if a > 0.0 {
                p.rrect(px + 8, top + 2, PANEL_W - 16, row_h(row) - 4, 10.0, WHITE, a);
            }
            // Rows that do nothing without the chips are faded, as the reference's inert rows.
            let live = if inert { 0.38 } else { 1.0 };
            let glow = if i == self.sel { 1.0f32.max(hovered) } else { hovered };
            let label = [mix(dim[0], fg[0], glow), mix(dim[1], fg[1], glow), mix(dim[2], fg[2], glow)];
            self.text(&mut p, px + PAD, cy, row.label(), label, live, false);
            let cx = px + CTRL_X;
            match row {
                Row::Mode | Row::Clock | Row::Cursor | Row::Language => {
                    let (labels, count) = options(row);
                    for (n, label) in labels[..count].iter().enumerate() {
                        let x = cx + n as i32 * choice_w(row);
                        let (on, lit) = (self.chosen(row) == n, if inert { 0.0 } else { self.lit(&|h| h == Hit::Choice(row, n)) });
                        // A radio button: a ring, a dot when chosen (the accent).
                        let ring = if on { accent } else { RING };
                        p.disc(x as f32 + 8.0, cy as f32, 7.5, ring, live);
                        p.disc(x as f32 + 8.0, cy as f32, 6.0, BODY_RGB, live);
                        if on {
                            p.disc(x as f32 + 8.0, cy as f32, 3.0, accent, live);
                        }
                        let c = if on { fg } else { [mix(dim[0], fg[0], lit), mix(dim[1], fg[1], lit), mix(dim[2], fg[2], lit)] };
                        self.text(&mut p, x + 22, cy, label, c, live, false);
                    }
                }
                Row::Chips => {
                    let slots = self.slots();
                    let held = self.chip_drag.filter(|d| d.moved);
                    let chip_at = |p: &mut Paint, chip: Chip, (x, y, cw, ch): (i32, i32, i32, i32), on: bool, a: f32, lit: f32, focus: bool| {
                        let color = bar::color(chip);
                        let r = 9.0;
                        if on {
                            p.rrect(x, y, cw, ch, r, color, a * mix(0.12, 0.20, lit));
                            p.outline(x, y, cw, ch, r, color, a * 0.45);
                        } else {
                            p.rrect(x, y, cw, ch, r, WHITE, a * mix(0.035, 0.08, lit));
                            p.outline(x, y, cw, ch, r, WHITE, a * 0.07);
                        }
                        if focus {
                            p.outline(x - 1, y - 1, cw + 2, ch + 2, r + 1.0, accent, 0.8);
                        }
                        let dot = (x + 14) as f32;
                        if on {
                            p.disc(dot, (y + ch / 2) as f32, 3.5, color, a);
                        } else {
                            p.disc(dot, (y + ch / 2) as f32, 3.5, dim, a);
                            p.disc(dot, (y + ch / 2) as f32, 2.0, BODY_RGB, a);
                        }
                        self.text(p, x + 26, y + ch / 2, chip_label(chip), if on { color } else { dim }, a, on);
                    };
                    for (n, &chip) in slots.iter().enumerate() {
                        let (x, y, cw, ch) = slot_rect(n);
                        let on = self.draft.input.chips.contains(&chip);
                        // The chip being dragged leaves its slot as a faint ghost; it is drawn again under the pointer.
                        let ghost = held.is_some_and(|d| d.chip == chip);
                        let a = if ghost { 0.35 } else { live };
                        chip_at(&mut p, chip, (px + x, py + y, cw, ch), on, a, if ghost { 0.0 } else { self.lit(&|h| h == Hit::Chip(chip)) }, i == self.sel && self.chip_sel == n && !inert);
                    }
                    if let Some(d) = held {
                        let (_, _, cw, ch) = slot_rect(0);
                        let (x, y) = (px + d.at.0 - cw / 2, py + d.at.1 - ch / 2);
                        // Lifted: a soft shadow under it, then the chip itself.
                        p.rrect(x + 1, y + 4, cw, ch, 9.0, BLACK, 0.35);
                        p.rrect(x, y, cw, ch, 9.0, BODY_RGB, 1.0);
                        chip_at(&mut p, d.chip, (x, y, cw, ch), self.draft.input.chips.contains(&d.chip), 1.0, 1.0, false);
                    }
                }
                Row::Family => {
                    let right = px + PANEL_W - PAD - ARROW_W;
                    for (x, dir) in [(cx, -1), (right, 1)] {
                        let lit = self.lit(&|h| h == Hit::Arrow(dir));
                        p.rrect(x, cy - 15, ARROW_W, 30, 8.0, WHITE, mix(0.06, 0.12, lit));
                        p.chevron(x + ARROW_W / 2, cy, dir, fg, mix(0.80, 1.0, lit));
                    }
                    p.rrect(cx + ARROW_W + 6, cy - 15, right - cx - ARROW_W - 12, 30, 8.0, WHITE, 0.04);
                    let room = right - cx - ARROW_W - 28;
                    let label = self.family_label();
                    let mut shown = label.to_string();
                    while self.width(&shown) > room && shown.chars().count() > 1 {
                        shown = label.chars().take(shown.chars().count().saturating_sub(2)).chain(['\u{2026}']).collect();
                    }
                    let tx = cx + ARROW_W + (right - cx - ARROW_W - self.width(&shown)) / 2;
                    self.text(&mut p, tx, cy, &shown, fg, 1.0, false);
                }
                Row::Bold | Row::Blur => {
                    let on = if row == Row::Blur { self.draft.blur } else { self.draft.font.bold };
                    let (pw, ph) = PILL;
                    p.rrect(cx, cy - ph / 2, pw, ph, ph as f32 / 2.0, if on { accent } else { OFF }, 1.0);
                    let kx = if on { cx + pw - ph / 2 } else { cx + ph / 2 };
                    p.disc(kx as f32, cy as f32, ph as f32 / 2.0 - 3.0, if on { [0.102, 0.075, 0.031] } else { [0.91, 0.91, 0.918] }, 1.0);
                    let note = match (row, on, self.blur_supported) {
                        (Row::Blur, true, false) => tr(Str::OnUnsupported),
                        (Row::Blur, false, false) => tr(Str::OffUnsupported),
                        (_, true, _) => tr(Str::On),
                        _ => tr(Str::Off),
                    };
                    self.text(&mut p, cx + pw + 12, cy, note, dim, 1.0, false);
                }
                _ => {
                    let (lo, hi, _) = row.range();
                    let t = ((self.draft.get(row) - lo) / (hi - lo)).clamp(0.0, 1.0);
                    let kx = cx as f32 + t * TRACK_W as f32;
                    let lit = if self.drag == Some(row) { 1.0 } else { self.lit(&|h| matches!(h, Hit::Slider(r, _) if r == row)) };
                    p.rrect(cx, cy - 2, TRACK_W, 4, 2.0, OFF, 1.0);
                    p.rrect(cx, cy - 2, (kx - cx as f32).round() as i32, 4, 2.0, accent, 1.0);
                    p.disc(kx, cy as f32 + 1.0, mix(7.0, 8.0, lit), BLACK, 0.35);
                    p.disc(kx, cy as f32, mix(7.0, 8.0, lit), [0.957, 0.957, 0.961], 1.0);
                    let right = px + PANEL_W - PAD;
                    match self.edit.as_ref().filter(|e| e.row == row) {
                        // The box being typed into: a lit field, the text, a caret after it.
                        Some(e) => {
                            let bw = self.width("0") * EDIT_MAX as i32 + 14;
                            p.rrect(right - bw + 4, cy - 13, bw, 26, 7.0, accent, 0.6);
                            p.rrect(right - bw + 5, cy - 12, bw - 2, 24, 6.0, [0.03, 0.03, 0.04], 1.0);
                            let tw = self.width(&e.text);
                            let tx = right - tw - 3;
                            self.text(&mut p, tx, cy, &e.text, fg, 1.0, false);
                            if e.selected {
                                p.rrect(tx - 2, cy - 9, tw + 4, 18, 3.0, accent, 0.28);
                            } else {
                                p.rrect(right - 2, cy - 8, 2, 16, 1.0, accent, 0.95);
                            }
                        }
                        None => {
                            let v = self.draft.text(row);
                            let lit = self.lit(&|h| matches!(h, Hit::Value(r) if r == row));
                            if lit > 0.0 {
                                let bw = self.width(&v) + 12;
                                p.outline(right - bw + 3, cy - 12, bw, 24, 6.0, WHITE, 0.10 * lit);
                            }
                            self.text(&mut p, right - self.width(&v), cy, &v, fg, 1.0, false);
                        }
                    }
                }
            }
        }

        if self.page == Page::Input {
            // What the page cannot say in its controls, in a quiet box.
            let y = py + HEAD + 3 * ROW + 3 * ROW + 8;
            p.rrect(px + PAD - 6, y, PANEL_W - 2 * PAD + 12, 76, 10.0, WHITE, 0.03);
            for (n, line) in [tr(Str::Note1), tr(Str::Note2), tr(Str::Note3)].into_iter().enumerate() {
                self.text(&mut p, px + PAD + 6, y + 17 + n as i32 * 21, line, dim, 1.0, false);
            }
        }

        // The footer: a hairline, the reset button, a note when something is unsaved, cancel and save.
        p.shape(px + 1, py + PANEL_H - FOOT, px + PANEL_W - 1, py + PANEL_H - FOOT + 1, WHITE, 0.07, &|_, _| 1.0);
        for (b, label) in [(Hit::Defaults, tr(Str::Reset)), (Hit::Cancel, tr(Str::Cancel)), (Hit::Save, tr(Str::Save))] {
            let (x, y, bw, bh) = button_rect(b);
            let lit = self.lit(&|h| h == b);
            let primary = b == Hit::Save;
            let (bg, a, ink) = if primary {
                ([mix(0.957, 1.0, lit); 3], 1.0, [0.067, 0.067, 0.075])
            } else {
                (WHITE, mix(0.07, 0.12, lit), fg)
            };
            p.rrect(px + x, py + y, bw, bh, 9.0, bg, a);
            let tw = self.width(label);
            self.text(&mut p, px + x + (bw - tw) / 2, py + y + bh / 2, label, ink, 1.0, false);
        }
        if self.draft != self.orig {
            let (x, y, bw, bh) = button_rect(Hit::Defaults);
            self.text(&mut p, px + x + bw + 14, py + y + bh / 2, tr(Str::Unsaved), accent, 1.0, false);
        }
    }

    /// The live preview box, over the panel's title row: what `bar::paint_preview` draws with the grid's
    /// own font. Called right after `draw`, with the same arguments for the geometry.
    pub fn draw_preview(&self, grid: &crate::soft::Soft, ctx: &crate::context::Context, fb: &mut [u32], (ww, wh): (u32, u32)) {
        let (px, py) = origin(ww as i32, wh as i32);
        let rect = (px + PAD, py + PREVIEW_Y, PANEL_W - 2 * PAD, PREVIEW_H);
        bar::paint_preview(grid, fb, rect, &self.draft.input, &ctx.with_examples(), ease(self.fade));
    }

    /// `s` with its left edge at `x` and its vertical centre at `cy` (window pixels).
    fn text(&self, p: &mut Paint, x: i32, cy: i32, s: &str, c: [f32; 3], a: f32, bold: bool) {
        if let Some(f) = &self.prop {
            // Centred on the middle of the capitals, the way the eye reads a label.
            let base = cy + (f.cap_height(UI_PX) / 2.0).round() as i32;
            f.draw(s, UI_PX, x, base, bold, |gx, gy, cov| p.put(gx, gy, c, a * cov as f32 / 255.0));
            return;
        }
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

    /// The width of `s` as `text` draws it.
    fn width(&self, s: &str) -> i32 {
        match &self.prop {
            Some(f) => f.width(s, UI_PX),
            None => s.chars().count() as i32 * self.ui.cell_w as i32,
        }
    }
}

// --------------------------------------------------------------- painting --

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
        let cy = |row: Row| row_top(Page::Font, row.index()) + ROW / 2;
        assert!(matches!(h(CTRL_X + TRACK_W / 2, cy(Row::Size)), Hit::Slider(Row::Size, t) if (t - 0.5).abs() < 0.01));
        assert!(matches!(h(CTRL_X - 4, cy(Row::Opacity)), Hit::Slider(Row::Opacity, t) if t == 0.0));
        assert_eq!(h(CTRL_X + 5, cy(Row::Family)), Hit::Arrow(-1));
        assert_eq!(h(PANEL_W - PAD - 5, cy(Row::Family)), Hit::Arrow(1));
        assert_eq!(h(CTRL_X + 10, cy(Row::Bold)), Hit::Toggle(Row::Bold));
        assert_eq!(h(CTRL_X + 10, cy(Row::Blur)), Hit::Toggle(Row::Blur));
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
        let cy = row_top(Page::Font, Row::Size.index()) + ROW / 2;
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
        assert_eq!(s.row(), Row::Size);
        assert_eq!(s.key(&key(sym::RIGHT, false)), Effect::Apply);
        assert_eq!(s.draft.font.size, 16.0);
        s.key(&key(sym::LEFT, true)); // shift: five steps
        assert_eq!(s.draft.font.size, 11.0);
        s.key(&key(sym::UP, false));
        s.key(&key(sym::UP, false)); // wraps to the last row, the language
        assert_eq!(s.row(), Row::Language);
        // Right picks the next language, Left the one before; the ends do not wrap.
        s.key(&key(sym::RIGHT, false));
        assert_eq!(s.draft.lang, i18n::Setting::En);
        s.key(&key(sym::RIGHT, false));
        s.key(&key(sym::RIGHT, false));
        assert_eq!(s.draft.lang, i18n::Setting::Es);
        assert!(s.changes().iter().any(|(k, v)| *k == "ui.language" && v.as_deref() == Some("\"es\"")));
        s.key(&key(sym::LEFT, false));
        s.key(&key(sym::LEFT, false));
        assert_eq!(s.draft.lang, i18n::Setting::Auto);
        assert!(s.changes().iter().all(|(k, _)| *k != "ui.language"));
        s.key(&key(sym::UP, false));
        assert_eq!(s.row(), Row::Blur);
        s.key(&key(sym::UP, false));
        assert_eq!(s.row(), Row::Opacity);
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
        at(VALUE_X + 10, row_top(Page::Font, row.index()) + ROW / 2)
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
        assert_eq!((s.draft.font.weight, s.row()), (700, Row::Bold));
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

    // ------------------------------------------------------------ input page --

    /// The window pixel of a point of the panel, after the Input tab was clicked.
    fn on_input_page() -> Settings {
        let mut s = open();
        let (x, y, w, h) = tab_rect(Page::Input);
        let (fx, fy) = at(x + w / 2, y + h / 2);
        assert_eq!(s.hit(fx, fy, WW, WH), Hit::Tab(Page::Input));
        assert_eq!(s.press(fx, fy, WW, WH), Effect::Redraw);
        assert_eq!(s.page, Page::Input);
        s
    }

    /// The input page with the chips on: the clock and the chip grid answer only then.
    fn on_chips_page() -> Settings {
        let mut s = on_input_page();
        s.draft.input.inline = true;
        s.orig.input.inline = true;
        s
    }

    #[test]
    fn the_panel_fades_in_and_a_hover_fades_between_looks() {
        use std::time::{Duration, Instant};
        let mut s = open();
        assert!(!s.animating(), "a panel that is not asked to fade is just there");
        let frame = |s: &Settings| {
            let (ml, mt, mb) = chrome::margins(false);
            let (w, h) = (WW as usize + 2 * ml, WH as usize + mt + mb);
            let mut fb = vec![0xe0101010u32; w * h];
            s.draw(&mut fb, w, h, (ml, mt), (WW, WH), false);
            fb
        };
        let (ml, mt, mb) = chrome::margins(false);
        let blank = vec![0xe0101010u32; (WW as usize + 2 * ml) * (WH as usize + mt + mb)];
        s.start_faded();
        assert!(s.animating());
        assert_eq!(frame(&s), blank, "fully faded out it draws nothing, not even the dimming");
        let t0 = Instant::now();
        assert!(s.step(t0), "the first step only starts the clock");
        let half = frame(&s);
        assert!(s.step(t0 + Duration::from_millis(80)));
        let more = frame(&s);
        assert_ne!(half, more);
        assert!(!s.step(t0 + Duration::from_millis(400)), "it ends, and then nothing asks for frames");
        assert_eq!(s.fade, 1.0);
        assert_eq!(frame(&s), frame(&open()), "faded in it is exactly the panel");

        // Moving onto a button starts a fade of 0.12 s from the old look to the new one.
        let (x, y, w, h) = button_rect(Hit::Save);
        let (fx, fy) = at(x + w / 2, y + h / 2);
        let rest = frame(&s);
        s.pointer(fx, fy, WW, WH);
        assert!(s.animating());
        assert_eq!(s.lit(&|h| h == Hit::Save), 0.0);
        assert_eq!(frame(&s), rest, "the first frame is still the old look");
        let t1 = Instant::now();
        s.step(t1);
        s.step(t1 + Duration::from_millis(60));
        let mid = s.lit(&|h| h == Hit::Save);
        assert!(mid > 0.3 && mid < 0.7, "{mid}");
        assert!(!s.step(t1 + Duration::from_millis(300)));
        assert_eq!(s.lit(&|h| h == Hit::Save), 1.0);
        // Leaving fades it out the same way, and moving along a slider does not restart anything.
        s.pointer(0.0, 0.0, WW, WH);
        assert_eq!(s.lit(&|h| h == Hit::Save), 1.0, "still lit on the frame it is left");
        s.step(Instant::now());
        assert!(s.animating());
        let (sx, sy) = at(CTRL_X + 20, row_top(Page::Font, Row::Size.index()) + ROW / 2);
        s.pointer(sx, sy, WW, WH);
        let t2 = Instant::now();
        s.step(t2);
        s.step(t2 + Duration::from_millis(500));
        assert!(!s.animating());
        s.pointer(sx + 40.0, sy, WW, WH);
        assert!(!s.animating(), "the same slider: nothing to fade");
    }

    #[test]
    fn blur_is_a_switch_apart_from_opacity() {
        let mut s = Settings::new(&FontCfg::default(), 0.88, vec![]).with_blur(false, true);
        assert!(s.changes().is_empty());
        // Space on its row, a click on its pill and the arrows all flip it, and nothing else moves.
        s.sel = Row::Blur.index();
        assert_eq!(s.key(&key(0x20, false)), Effect::Apply);
        assert!(s.draft.blur);
        assert_eq!(s.draft.opacity, 0.88);
        assert_eq!(s.changes(), vec![("window.blur", Some("true".to_string()))]);
        assert_eq!(click(&mut s, (CTRL_X + 10, row_top(Page::Font, Row::Blur.index()) + ROW / 2)), Effect::Apply);
        assert!(!s.draft.blur);
        assert!(s.changes().is_empty(), "back to what it opened with");
        s.key(&key(sym::RIGHT, false));
        assert!(s.draft.blur);
        // Opacity does not touch blur, and the defaults turn it off.
        s.sel = Row::Opacity.index();
        s.key(&key(sym::LEFT, false));
        assert!(s.draft.blur);
        s.defaults();
        assert!(!s.draft.blur);
        // Saved, it is what the file says; a compositor without blur still draws the row.
        s.sel = Row::Blur.index();
        s.key(&key(0x20, false));
        s.saved();
        assert!(s.changes().is_empty());
        let (ml, mt, mb) = chrome::margins(false);
        let (w, h) = (WW as usize + 2 * ml, WH as usize + mt + mb);
        let frame = |s: &Settings| {
            let mut fb = vec![0xe0101010u32; w * h];
            s.draw(&mut fb, w, h, (ml, mt), (WW, WH), false);
            fb
        };
        let yes = Settings::new(&FontCfg::default(), 0.88, vec![]).with_blur(true, true);
        let no = Settings::new(&FontCfg::default(), 0.88, vec![]).with_blur(true, false);
        assert_ne!(frame(&yes), frame(&no), "a compositor without blur is told so on the row");
    }

    fn click(s: &mut Settings, (x, y): (i32, i32)) -> Effect {
        let (fx, fy) = at(x, y);
        s.press(fx, fy, WW, WH)
    }

    #[test]
    fn the_tabs_switch_pages_and_each_page_has_its_own_rows() {
        let mut s = on_input_page();
        assert_eq!((s.sel, s.row()), (0, Row::Mode));
        // The font rows are not under the pointer any more: a click where the opacity slider was does nothing.
        let (x, y) = (CTRL_X + 50, row_top(Page::Font, Row::Opacity.index()) + ROW / 2);
        let before = s.draft.clone();
        assert_eq!(click(&mut s, (x, y)), Effect::None);
        assert_eq!(s.draft, before);
        // And back.
        let (tx, ty, tw, th) = tab_rect(Page::Font);
        assert_eq!(click(&mut s, (tx + tw / 2, ty + th / 2)), Effect::Redraw);
        assert_eq!((s.page, s.row()), (Page::Font, Row::Family));
    }

    #[test]
    fn page_keys_and_ctrl_tab_switch_pages() {
        let mut s = open();
        s.key(&key(sym::PAGE_DOWN, false));
        assert_eq!(s.page, Page::Input);
        s.key(&key(sym::PAGE_UP, false));
        assert_eq!(s.page, Page::Font);
        s.key(&Key { sym: sym::TAB, text: String::new(), mods: Mods { ctrl: true, ..Mods::default() } });
        assert_eq!(s.page, Page::Input);
        // Up and down stay inside the page: four rows there.
        s.key(&key(sym::DOWN, false));
        s.key(&key(sym::DOWN, false));
        s.key(&key(sym::DOWN, false));
        assert_eq!(s.row(), Row::Chips);
        s.key(&key(sym::DOWN, false));
        assert_eq!(s.row(), Row::Mode);
    }

    #[test]
    fn the_choices_set_the_mode_and_the_clock() {
        let mut s = on_input_page();
        let cy = |row: Row| row_top(Page::Input, row.index()) + ROW / 2;
        assert!(!s.draft.input.inline);
        // Without the chips the clock and the chips are inert.
        assert_eq!(click(&mut s, (CTRL_X + CHOICE_W + 20, cy(Row::Clock))), Effect::None);
        assert!(!s.draft.input.clock24);
        assert_eq!(click(&mut s, (CTRL_X + CHOICE_W + 20, cy(Row::Mode))), Effect::Apply);
        assert!(s.draft.input.inline);
        assert_eq!(click(&mut s, (CTRL_X + CHOICE_W + 20, cy(Row::Clock))), Effect::Apply);
        assert!(s.draft.input.clock24);
        assert_eq!(click(&mut s, (CTRL_X + 20, cy(Row::Mode))), Effect::Apply);
        assert!(!s.draft.input.inline);
        // The cursor row has three options.
        assert_eq!(click(&mut s, (CTRL_X + 2 * 100 + 20, cy(Row::Cursor))), Effect::Apply);
        assert_eq!(s.draft.input.cursor, config::CursorKind::Underline);
        assert_eq!(click(&mut s, (CTRL_X + 100 + 20, cy(Row::Cursor))), Effect::Apply);
        assert_eq!(s.draft.input.cursor, config::CursorKind::Bar);
        s.sel = Row::Cursor.index();
        s.key(&key(sym::LEFT, false));
        assert_eq!(s.draft.input.cursor, config::CursorKind::Block);
        s.key(&key(sym::LEFT, false));
        assert_eq!(s.draft.input.cursor, config::CursorKind::Block, "no wrapping at the ends");
        s.key(&Key { sym: 0x20, text: " ".into(), mods: Mods::default() });
        assert_eq!(s.draft.input.cursor, config::CursorKind::Bar, "space goes to the next one");
        s.draft.input.cursor = config::CursorKind::Block;
        assert!(s.changes().iter().all(|(k, _)| *k != "input.cursor"));
        s.draft.input.cursor = config::CursorKind::Underline;
        assert_eq!(s.changes().last().map(|(k, v)| (*k, v.clone())), Some(("input.cursor", Some("\"underline\"".to_string()))));
        s.draft.input.cursor = config::CursorKind::Block;
        s.draft.input.clock24 = true;
        // Between and beyond the two options is the panel.
        let (fx, fy) = at(CTRL_X + 2 * CHOICE_W + 20, cy(Row::Mode));
        assert_eq!(s.hit(fx, fy, WW, WH), Hit::Panel);
        // The keyboard: Right picks the second, Left the first, Space flips.
        s.sel = Row::Mode.index();
        s.key(&key(sym::RIGHT, false));
        assert!(s.draft.input.inline);
        s.key(&key(sym::LEFT, false));
        assert!(!s.draft.input.inline);
        s.key(&typed(" "));
        s.key(&Key { sym: 0x20, text: " ".into(), mods: Mods::default() });
        assert!(s.draft.input.inline, "space flipped the choice once (the text-only key does nothing)");
    }

    /// Where chip `c` is in the grid now.
    fn chip_rect(s: &Settings, c: Chip) -> (i32, i32, i32, i32) {
        slot_rect(s.slots().iter().position(|&x| x == c).unwrap())
    }

    fn centre(s: &Settings, c: Chip) -> (i32, i32) {
        let (x, y, w, h) = chip_rect(s, c);
        (x + w / 2, y + h / 2)
    }

    /// Presses on a chip, moves the pointer to panel position `(x, y)` in steps, and lets go.
    fn drag(s: &mut Settings, from: (i32, i32), to: (i32, i32)) {
        click(s, from);
        for step in 1..=8 {
            let p = (from.0 + (to.0 - from.0) * step / 8, from.1 + (to.1 - from.1) * step / 8);
            let (fx, fy) = at(p.0, p.1);
            s.pointer(fx, fy, WW, WH);
        }
        s.release();
    }

    #[test]
    fn a_click_on_a_chip_switches_it_off_and_back_at_the_end() {
        let mut s = on_chips_page();
        assert_eq!(s.draft.input.chips, Chip::ALL);
        let c = centre(&s, Chip::Dir);
        // Pressing alone changes nothing; letting go without having moved is the click.
        assert_eq!(click(&mut s, c), Effect::Redraw);
        assert_eq!(s.draft.input.chips, Chip::ALL);
        assert_eq!(s.release(), Effect::Apply);
        assert!(!s.draft.input.chips.contains(&Chip::Dir));
        assert_eq!(s.draft.input.chips.len(), Chip::COUNT - 1);
        // It is listed with the chips that are off, after the ones that are on.
        assert_eq!(*s.slots().last().unwrap(), Chip::Dir);
        assert_eq!(s.chip_sel, Chip::COUNT - 1, "the cursor follows the chip");
        // Back on: at the end of the bar, not in its old place.
        let c = centre(&s, Chip::Dir);
        click(&mut s, c);
        s.release();
        assert_eq!(s.draft.input.chips.last(), Some(&Chip::Dir));
        // All nine cells are distinct and inside the panel.
        for (i, c) in s.slots().into_iter().enumerate() {
            let (x, y, w, h) = slot_rect(i);
            assert!(x >= CTRL_X && x + w <= PANEL_W - PAD + 1 && y + h <= row_top(Page::Input, 3) + 3 * ROW, "{c:?}");
            let (fx, fy) = at(x + w / 2, y + h / 2);
            assert_eq!(s.hit(fx, fy, WW, WH), Hit::Chip(c));
        }
    }

    #[test]
    fn dragging_a_chip_moves_it_to_the_slot_under_the_pointer() {
        let mut s = on_chips_page();
        let (from, to) = (centre(&s, Chip::Time), centre(&s, Chip::Host));
        drag(&mut s, from, to);
        // Time went to Host's place (slot 4) and the ones between moved up one.
        assert_eq!(
            s.draft.input.chips,
            [Chip::User, Chip::Env, Chip::Pyenv, Chip::Host, Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes, Chip::Kube]
        );
        assert_eq!(s.chip_sel, 4);
        // A drag is not a click: nothing was switched off.
        assert_eq!(s.draft.input.chips.len(), Chip::COUNT);
        // And back to the front.
        let (from, to) = (centre(&s, Chip::Time), centre(&s, Chip::User));
        drag(&mut s, from, to);
        assert_eq!(s.draft.input.chips[0], Chip::Time);
        assert!(s.changes().is_empty(), "the default order again: nothing to save");
        // Any other order is a setting: it is written as the list.
        let (from, to) = (centre(&s, Chip::Kube), centre(&s, Chip::User));
        drag(&mut s, from, to);
        let c = s.changes();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].0, "input.chips");
        assert!(c[0].1.as_deref().unwrap().starts_with("[\"time\", \"kube\", \"user\""), "{:?}", c[0].1);
    }

    #[test]
    fn a_small_wobble_is_still_a_click_and_the_drag_survives_leaving_the_grid() {
        let mut s = on_chips_page();
        let c = centre(&s, Chip::Env);
        click(&mut s, c);
        let (fx, fy) = at(c.0 + 2, c.1 + 1);
        assert_eq!(s.pointer(fx, fy, WW, WH), Effect::None, "inside the slop nothing moves");
        s.release();
        assert!(!s.draft.input.chips.contains(&Chip::Env));
        // Two chips off, so the last slots of the grid are the "off" part. Dragging far to the right and below
        // the grid lands on the last one: the chip goes off.
        let mut s = on_chips_page();
        for c in [Chip::Kube, Chip::Changes] {
            let at = centre(&s, c);
            click(&mut s, at);
            s.release();
        }
        let c = centre(&s, Chip::Time);
        drag(&mut s, c, (PANEL_W + 40, PANEL_H));
        assert!(!s.draft.input.chips.contains(&Chip::Time));
        assert_eq!(s.draft.input.chips.len(), Chip::COUNT - 3);
        // Dragging a chip that is off into the bar turns it on, in the slot it is dropped on.
        let slot = s.slots().iter().position(|&x| x == Chip::Dir).unwrap();
        let c = centre(&s, Chip::Time);
        let to = centre(&s, Chip::Dir);
        drag(&mut s, c, to);
        assert_eq!(s.draft.input.chips.iter().position(|&x| x == Chip::Time), Some(slot), "{:?}", s.draft.input.chips);
        // The press that started a drag is over: a later click elsewhere is a plain click.
        assert!(s.chip_drag.is_none());
    }

    #[test]
    fn alt_and_the_arrows_move_the_chip_the_cursor_is_on() {
        let mut s = on_input_page();
        s.sel = Row::Chips.index();
        s.key(&key(sym::RIGHT, false)); // on User
        let alt = |sym| Key { sym, text: String::new(), mods: Mods { alt: true, ..Mods::default() } };
        assert_eq!(s.key(&alt(sym::RIGHT)), Effect::Apply);
        assert_eq!(&s.draft.input.chips[..3], [Chip::Time, Chip::Env, Chip::User]);
        assert_eq!(s.chip_sel, 2, "the cursor goes with the chip");
        s.key(&alt(sym::LEFT));
        s.key(&alt(sym::LEFT));
        assert_eq!(s.draft.input.chips[0], Chip::User);
        assert_eq!(s.key(&alt(sym::LEFT)), Effect::None, "it stops at the front");
        // A chip that is off has no place to move in.
        s.chip_sel = Chip::COUNT - 1;
        s.toggle_chip(s.slots()[Chip::COUNT - 1]);
        let kept = s.draft.input.chips.clone();
        s.chip_sel = Chip::COUNT - 1;
        assert_eq!(s.key(&alt(sym::LEFT)), Effect::None);
        assert_eq!(s.draft.input.chips, kept);
    }

    #[test]
    fn the_order_survives_changing_page_and_coming_back() {
        let mut s = on_chips_page();
        let (from, to) = (centre(&s, Chip::Kube), centre(&s, Chip::Time));
        drag(&mut s, from, to);
        let order = s.draft.input.chips.clone();
        assert_eq!(order[0], Chip::Kube);
        s.set_page(Page::Font);
        s.set_page(Page::Input);
        s.key(&key(sym::PAGE_UP, false));
        s.key(&key(sym::PAGE_DOWN, false));
        assert_eq!(s.draft.input.chips, order);
        // Saved and reopened with that list, the grid shows it.
        s.saved();
        let t = Settings::new(&FontCfg::default(), 0.88, fams()).with_input(&InputCfg { chips: order.clone(), ..InputCfg::default() });
        assert_eq!(&t.slots()[..Chip::COUNT], &order[..]);
    }

    #[test]
    fn the_chip_cursor_moves_with_the_arrows_and_space_flips_the_chip() {
        let mut s = on_input_page();
        s.sel = Row::Chips.index();
        s.key(&key(sym::RIGHT, false));
        s.key(&key(sym::RIGHT, false));
        assert_eq!(s.chip_sel, 2);
        assert_eq!(s.key(&Key { sym: 0x20, text: " ".into(), mods: Mods::default() }), Effect::Apply);
        assert!(!s.draft.input.chips.contains(&Chip::Env));
        assert_eq!(s.chip_sel, Chip::COUNT - 1, "the chip went to the off part of the grid, the cursor with it");
        s.key(&key(sym::RIGHT, false));
        assert_eq!(s.chip_sel, 0, "wraps");
        s.key(&key(sym::LEFT, false));
        assert_eq!(s.chip_sel, Chip::COUNT - 1);
    }

    #[test]
    fn only_the_input_settings_that_changed_are_saved() {
        let mut s = on_input_page();
        assert!(s.changes().is_empty());
        s.choose(Row::Mode, 1);
        s.choose(Row::Clock, 1);
        s.toggle_chip(Chip::Kube);
        s.toggle_chip(Chip::Time);
        let c = s.changes();
        assert_eq!(c[0], ("input.mode", Some("\"chips\"".into())));
        assert_eq!(c[1], ("input.clock", Some("\"24h\"".into())));
        assert_eq!(c[2].0, "input.chips");
        assert!(c[2].1.as_deref().is_some_and(|v| v.starts_with("[\"user\"") && !v.contains("kube") && !v.contains("time")));
        // Saved: nothing differs any more.
        s.saved();
        assert!(s.changes().is_empty());
        // Defaults put the input back too, and Cancel's copy is what the screen opened with.
        let mut t = Settings::new(&FontCfg::default(), 0.88, fams()).with_input(&InputCfg { inline: true, chips: vec![Chip::Dir], clock24: true, ..Default::default() });
        assert!(t.original().input.inline);
        t.defaults();
        assert_eq!(t.draft.input, InputCfg::default());
        assert!(t.original().input.inline);
    }

    #[test]
    fn the_input_page_draws_and_the_chips_show_their_colours() {
        let mut s = on_input_page();
        s.choose(Row::Mode, 1);
        s.toggle_chip(Chip::Kube);
        let (w, h) = (WW as usize, WH as usize);
        let mut fb = vec![0u32; w * h];
        s.draw(&mut fb, w, h, (0, 0), (WW, WH), false);
        // An enabled chip's cell carries its colour: the time chip's red outweighs its green and blue.
        let (x, y, cw, ch) = chip_rect(&s, Chip::Time);
        let (ox, oy) = origin(WW as i32, WH as i32);
        let px = fb[((oy + y + ch / 2) as usize) * w + (ox + x + cw - 6) as usize];
        let (r, g, b) = ((px >> 16 & 255) as i32, (px >> 8 & 255) as i32, (px & 255) as i32);
        assert!(r > g + 8 && r > b + 8, "{r} {g} {b}");
        // The disabled one is grey.
        let (x, y, cw, ch) = chip_rect(&s, Chip::Kube);
        let px = fb[((oy + y + ch / 2) as usize) * w + (ox + x + cw - 6) as usize];
        let (r, g, b) = ((px >> 16 & 255) as i32, (px >> 8 & 255) as i32, (px & 255) as i32);
        assert!((r - g).abs() < 6 && (g - b).abs() < 6, "{r} {g} {b}");
    }
}
