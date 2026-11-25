//! The settings panel (ADR 0026): docked beside the terminal, not modal, after the reference's
//! `.prefs`. A header with a close button, three tabs with a sliding marker (Font, Input, Windows),
//! rows with the label at the left and the control at the right (a font picker, sliders with a number
//! box, switches, radio groups, the chips to switch and reorder), a note and a footer with the reset
//! button. The grid is narrower while it is open; every change applies at once and is saved by the app
//! (`wlapp.rs`) when the gesture ends, so there is no Save and no Cancel.
//!
//! Like the chrome, nothing here is kept between frames but the state in `Settings`: the panel is a pure
//! function of that state and its box, painted into the cleared box (`Soft::clear_rect`) whenever
//! something in it changed. The geometry is computed when asked (the layout depends on the width of the
//! labels, which depends on the language), never stored. The text is rasterized when drawn, with the
//! desktop's proportional font loaded only while the panel is open (`UiFont`), so it costs no memory the
//! rest of the time and does not move while the font being edited does.
//!
//! Coordinates in this module are panel-local (the origin is the box's top-left corner) unless the
//! function says window pixels; the body scrolls, and its content coordinates start under the tabs.
use std::path::PathBuf;

use crate::{
    bar,
    chrome,
    config::{self, Chip, CursorKind, FontCfg, InputCfg, TabsPos, WindowsCfg},
    font::{self, Fonts, UiFont},
    i18n::{self, Str, tr},
    input::sym,
    paint::{BLACK, Paint, WHITE, seg},
    palette,
    xkb::Key,
};

// ----------------------------------------------------------------- layout --

/// The panel's width (the reference's `.prefs`), and the share of the window it may take at most.
pub const PANEL_W: i32 = 372;
const MAX_SHARE: u32 = 62;

/// The box's width for a window `ww` wide: 372 px, or 62 % of a narrow window.
pub fn panel_width(ww: u32) -> u32 {
    (PANEL_W as u32).min(ww * MAX_SHARE / 100)
}

/// Header: 10 px above the 28 px close button (the tabs start at 48). Tabs: a 34 px container (3 + 28 + 3 px),
/// 6 px below it (the body starts at 88). Body: 4 px above its first row, 8 px below its last. Footer: a
/// hairline, 10 px, the 34 px button, 14 px.
const CLOSE: i32 = 28;
const TABS_Y: i32 = 48;
const TABS_H: i32 = 34;
const TAB_H: i32 = 28;
const TAB_PAD: i32 = 16;
const BODY_Y: i32 = 88;
const BODY_TOP: i32 = 4;
const BODY_BOTTOM: i32 = 8;
const FOOT_H: i32 = 59;
const BTN_H: i32 = 34;
/// Header, tabs and footer are inset by this much; a row is inset by the body's 8 px plus its own 10.
const EDGE: i32 = 18;
const ROW_X: i32 = 8;
const ROW_PAD: i32 = 10;
const ROW_MIN: i32 = 40;
/// The label column, and the gap before the control.
const LABEL_W: i32 = 112;
/// A label may run this far into the gap before it wraps (the reference's font is a little narrower than ours).
const LABEL_SLACK: i32 = 6;
const COL_GAP: i32 = 10;

/// Text sizes: labels and controls, the title, the notes, the keycaps and the code in a note.
const PX: f32 = 13.0;
const PX_TITLE: f32 = 15.0;
const PX_NOTE: f32 = 12.0;
const PX_KBD: f32 = 10.5;
const PX_CODE: f32 = 11.5;
const NOTE_LINE: f32 = 20.4;

/// A radio's box: padding 6 and 8, a 14 px ring, an 8 px gap to its label; radios sit 6 px apart, lines 2 px.
const RADIO_H: i32 = 30;
const RADIO_GAP: i32 = 6;
const RADIO_LINE_GAP: i32 = 2;
const RADIO_EXTRA: i32 = 8 + 14 + 8 + 8;
const SWITCH: (i32, i32) = (38, 22);
const ARROW: i32 = 30;
const PICKER_GAP: i32 = 6;
const VALUE_W: i32 = 58;
const VALUE_GAP: i32 = 14;
const VALUE_H: i32 = 26;
const THUMB_R: i32 = 7;
/// The chips: two columns, 6 px apart, each 34 px high; the row has 10 px above and below.
const CHIP_H: i32 = 34;
const CHIP_GAP: i32 = 6;
const CHIP_COLS: i32 = 2;
const CHIPS_PAD: i32 = 10;
const NOTE_MARGIN_TOP: i32 = 8;
const NOTE_MARGIN_BOTTOM: i32 = 6;

/// How long the panel takes to appear, a hover to fade and the tab marker to slide, in seconds.
const FADE_IN: f32 = 0.16;
const FADE_HOVER: f32 = 0.12;
const SLIDE: f32 = 0.22;
/// Pixels a wheel notch scrolls the body.
pub const WHEEL_PX: i32 = 48;

fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)]
}

type Rect = (i32, i32, i32, i32);

fn inside(x: i32, y: i32, (rx, ry, rw, rh): Rect) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Font,
    Input,
    Windows,
}

impl Page {
    const ALL: [Page; 3] = [Page::Font, Page::Input, Page::Windows];

    fn label(self) -> &'static str {
        match self {
            Page::Font => tr(Str::TabFont),
            Page::Input => tr(Str::TabInput),
            Page::Windows => tr(Str::TabWindows),
        }
    }

    fn index(self) -> usize {
        Page::ALL.iter().position(|&p| p == self).unwrap_or(0)
    }

    /// The paragraph under the rows, if the page has one.
    fn note(self) -> Option<Str> {
        match self {
            Page::Font => None,
            Page::Input => Some(Str::NoteInput),
            Page::Windows => Some(Str::NoteWindows),
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
    /// Input type: the shell's own prompt, or the context chips.
    Mode,
    Clock,
    /// The shape of the cursor.
    Cursor,
    Chips,
    /// Suggest the rest of a command while it is typed (read and saved; the editor that draws it comes later).
    Autosuggest,
    /// Where the list of windows lives.
    Tabs,
    /// Whether the panel of windows is shown.
    Side,
    SideW,
    /// The language of the interface.
    Language,
}

/// How a row's control works and looks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Picker,
    Slider,
    Switch,
    Radios,
    Chips,
}

const FONT_ROWS: [Row; 8] = [Row::Family, Row::Size, Row::Weight, Row::Bold, Row::Spacing, Row::Line, Row::Opacity, Row::Blur];
const INPUT_ROWS: [Row; 5] = [Row::Mode, Row::Clock, Row::Cursor, Row::Chips, Row::Autosuggest];
const WINDOWS_ROWS: [Row; 4] = [Row::Tabs, Row::Side, Row::SideW, Row::Language];
const MAX_ROWS: usize = 8;

fn rows(page: Page) -> &'static [Row] {
    match page {
        Page::Font => &FONT_ROWS,
        Page::Input => &INPUT_ROWS,
        Page::Windows => &WINDOWS_ROWS,
    }
}

/// The options of a radio row, in order, and how many there are.
fn options(row: Row) -> ([&'static str; 3], usize) {
    match row {
        Row::Mode => ([tr(Str::OptShell), tr(Str::OptChips), ""], 2),
        Row::Cursor => ([tr(Str::OptBlock), tr(Str::OptBar), tr(Str::OptUnderline)], 3),
        Row::Tabs => ([tr(Str::OptLeft), tr(Str::OptRight), tr(Str::OptTop)], 3),
        Row::Language => ([tr(Str::OptAuto), tr(Str::OptEnglish), tr(Str::OptSpanish)], 3),
        _ => ([tr(Str::Opt12), tr(Str::Opt24), ""], 2),
    }
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
            Row::Mode => Str::RowMode,
            Row::Clock => Str::RowClock,
            Row::Cursor => Str::RowCursor,
            Row::Chips => Str::RowChips,
            Row::Autosuggest => Str::RowAutosuggest,
            Row::Tabs => Str::RowTabs,
            Row::Side => Str::RowSide,
            Row::SideW => Str::RowSideW,
            Row::Language => Str::RowLanguage,
        })
    }

    fn kind(self) -> Kind {
        match self {
            Row::Family => Kind::Picker,
            Row::Size | Row::Weight | Row::Spacing | Row::Line | Row::Opacity | Row::SideW => Kind::Slider,
            Row::Bold | Row::Blur | Row::Autosuggest | Row::Side => Kind::Switch,
            Row::Mode | Row::Clock | Row::Cursor | Row::Tabs | Row::Language => Kind::Radios,
            Row::Chips => Kind::Chips,
        }
    }

    fn page(self) -> Page {
        Page::ALL.into_iter().find(|&p| rows(p).contains(&self)).unwrap_or(Page::Font)
    }

    /// `(min, max, step)` of a slider row, in the unit the setting is written in. The panel's own ranges are
    /// tighter than the file's (`config.rs` clamps those): what is useful to drag.
    fn range(self) -> (f32, f32, f32) {
        match self {
            Row::Size => (8.0, 40.0, 1.0),
            Row::Weight => (100.0, 900.0, 10.0),
            Row::Spacing => (-4.0, 16.0, 0.5),
            Row::Line => (0.8, 2.0, 0.05),
            Row::Opacity => (0.4, 1.0, 0.02),
            Row::SideW => (config::SIDE_W.0, config::SIDE_W.1, 4.0),
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
            Row::SideW => config::SIDE_W,
            _ => (0.0, 0.0),
        }
    }

    fn is_number(self) -> bool {
        self.kind() == Kind::Slider
    }

    /// Its place in its page.
    fn index(self) -> usize {
        rows(self.page()).iter().position(|&r| r == self).unwrap_or(0)
    }
}

/// What is under the pointer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    /// The cross in the header.
    Close,
    /// A page's tab.
    Tab(Page),
    /// A slider, and where on its track (0..1).
    Slider(Row, f32),
    /// The number at a slider's right: click to type it.
    Value(Row),
    /// The font picker's arrows: previous (-1) and next (+1).
    Arrow(i32),
    /// A switch.
    Toggle(Row),
    /// Option `n` of a radio row.
    Choice(Row, usize),
    /// A chip.
    Chip(Chip),
    /// A live row, on none of its controls: the row is lit all the same.
    Row(Row),
    /// The reset button.
    Defaults,
    /// On the panel, on nothing in particular.
    Panel,
    /// Off the panel.
    Outside,
}

impl Hit {
    /// The row the pointer is in, for the row's hover highlight.
    fn row(self) -> Option<Row> {
        match self {
            Hit::Slider(r, _) | Hit::Value(r) | Hit::Toggle(r) | Hit::Choice(r, _) | Hit::Row(r) => Some(r),
            Hit::Arrow(_) => Some(Row::Family),
            Hit::Chip(_) => Some(Row::Chips),
            _ => None,
        }
    }
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

// ------------------------------------------------------------------ notes --

/// How a piece of a note is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ink {
    Text,
    /// `` `code` ``
    Code,
    /// `[Key]`
    Key,
}

/// A word of a note: its text, how it is drawn, whether a space came before it and whether a line ended before.
struct Tok<'a> {
    text: &'a str,
    ink: Ink,
    space: bool,
    brk: bool,
}

/// Splits a note into words. `\n` ends a line, `` `code` `` and `[Key]` are one unbreakable word each.
fn tokens(s: &str) -> Vec<Tok<'_>> {
    /// Ends the plain word that started at `start`, if one did, at byte `end`.
    fn flush<'a>(s: &'a str, out: &mut Vec<Tok<'a>>, start: &mut Option<usize>, end: usize, (space, brk): (&mut bool, &mut bool)) {
        if let Some(a) = start.take() {
            out.push(Tok { text: &s[a..end], ink: Ink::Text, space: *space, brk: *brk });
            *space = false;
            *brk = false;
        }
    }
    let mut out = Vec::new();
    let (mut start, mut space, mut brk) = (None, false, false);
    let mut span: Option<(usize, Ink)> = None;
    for (i, c) in s.char_indices() {
        match (c, span) {
            ('`', None) | ('[', None) => {
                flush(s, &mut out, &mut start, i, (&mut space, &mut brk));
                span = Some((i + 1, if c == '`' { Ink::Code } else { Ink::Key }));
            }
            ('`', Some((a, Ink::Code))) | (']', Some((a, Ink::Key))) => {
                let ink = span.map_or(Ink::Text, |(_, k)| k);
                out.push(Tok { text: &s[a..i], ink, space, brk });
                (space, brk, span) = (false, false, None);
            }
            (_, Some(_)) => {}
            ('\n', None) => {
                flush(s, &mut out, &mut start, i, (&mut space, &mut brk));
                brk = true;
            }
            (' ', None) => {
                flush(s, &mut out, &mut start, i, (&mut space, &mut brk));
                space = true;
            }
            (_, None) => {
                start.get_or_insert(i);
            }
        }
    }
    flush(s, &mut out, &mut start, s.len(), (&mut space, &mut brk));
    out
}

/// Lays a note out in lines `max_w` wide: calls `put(word, ink, x, line)` for each word and returns how many
/// lines there are. `width` measures a word as it is drawn (keycaps and code are wider than their text).
fn flow(s: &str, max_w: i32, width: &dyn Fn(&str, Ink) -> i32, mut put: impl FnMut(&str, Ink, i32, i32)) -> i32 {
    let space_w = width(" ", Ink::Text);
    let (mut x, mut line) = (0, 0);
    for t in tokens(s) {
        let w = width(t.text, t.ink);
        if t.brk {
            line += 1;
            x = 0;
        } else if x > 0 {
            let gap = if t.space { space_w } else { 0 };
            if x + gap + w > max_w {
                line += 1;
                x = 0;
            } else {
                x += gap;
            }
        }
        put(t.text, t.ink, x, line);
        x += w;
    }
    line + 1
}

// ------------------------------------------------------------------ state --

/// A chip held by the pointer: pressed on it, and moved far enough to be a drag rather than a click.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ChipDrag {
    chip: Chip,
    /// Where it was pressed and where the pointer is, in body content coordinates.
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
    pub windows: WindowsCfg,
}

impl Draft {
    fn defaults() -> Draft {
        Draft {
            font: FontCfg::default(),
            opacity: chrome::OPACITY,
            input: InputCfg::default(),
            blur: false,
            lang: i18n::Setting::Auto,
            windows: WindowsCfg::default(),
        }
    }

    fn get(&self, row: Row) -> f32 {
        match row {
            Row::Size => self.font.size,
            Row::Weight => self.font.weight as f32,
            Row::Spacing => self.font.letter_spacing,
            Row::Line => self.font.line_height,
            Row::Opacity => self.opacity,
            Row::SideW => self.windows.side_w as f32,
            _ => 0.0,
        }
    }

    /// Sets a slider row, snapped to its step and kept inside the panel's range.
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
            Row::SideW => self.windows.side_w = v.round() as u32,
            _ => {}
        }
    }

    fn text(&self, row: Row) -> String {
        let v = self.get(row);
        match row {
            Row::Size | Row::Weight | Row::SideW => format!("{v:.0}"),
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
    /// Only the panel changed: repaint it.
    Redraw,
    /// A value changed: apply `draft` to the window, repaint, and save it once `settled`.
    Apply,
    /// A gesture ended (a slider or a chip let go): save what changed.
    Settled,
    /// Close the panel.
    Close,
}

pub struct Settings {
    pub draft: Draft,
    /// What the file says: what the draft is compared with to know what to write.
    orig: Draft,
    /// Installed monospaced families `(name, file)`; the picker has one more stop before them: the default.
    families: Vec<(String, PathBuf)>,
    /// 0 = the default font, `n` = `families[n - 1]`.
    fam: usize,
    pub page: Page,
    /// The selected row of the page (the keyboard's), and on the input page the selected chip of the grid.
    pub sel: usize,
    chip_sel: usize,
    /// A chip picked up with the pointer.
    chip_drag: Option<ChipDrag>,
    pub hover: Hit,
    /// What was under the pointer before `hover`, and how far (0 to 1) the change has faded in.
    prev: Hit,
    mix: f32,
    /// How visible the panel is: 0 when it has just opened, 1 once it has faded in.
    fade: f32,
    /// The tab marker slides: it left `ind_from` (x and width) and is `ind_t` (0 to 1) of the way.
    ind_from: (f32, f32),
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
    /// A line in the footer (a save that failed).
    pub notice: Option<String>,
    /// The keyboard is on the panel: the selected row is lit and keys act on it. Clicking the terminal takes it away.
    pub focus: bool,
    /// ... and on the tab bar rather than on a row (the reference focuses the active tab when it opens): a ring
    /// around the tab, and left and right change page.
    pub tabs_focus: bool,
    /// How far the body is scrolled, in pixels.
    scroll: i32,
    /// The panel's box in window pixels, and whether the hairline is at its left edge (it is at the grid's side).
    rect: Rect,
    hairline_left: bool,
}

impl Settings {
    /// Opens the panel on the current look, listing the fonts installed.
    pub fn open(font: &FontCfg, opacity: f32) -> Settings {
        let mut s = Settings::new(font, opacity, font::monospaced_families()).with_ui_font();
        s.tabs_focus = true;
        s
    }

    /// Draws the labels in the desktop's proportional font, if the machine has one.
    pub fn with_ui_font(mut self) -> Settings {
        self.prop = UiFont::load();
        self
    }

    /// The input settings the panel opens with (it starts from the defaults).
    pub fn with_input(mut self, input: &InputCfg) -> Settings {
        self.draft.input = input.clone();
        self.orig.input = input.clone();
        self
    }

    /// The language setting the panel opens with.
    pub fn with_lang(mut self, lang: i18n::Setting) -> Settings {
        self.draft.lang = lang;
        self.orig.lang = lang;
        self
    }

    /// The windows' settings the panel opens with.
    pub fn with_windows(mut self, windows: &WindowsCfg) -> Settings {
        self.draft.windows = windows.clone();
        self.orig.windows = windows.clone();
        self
    }

    /// The blur setting the panel opens with, and whether the compositor offers blur at all.
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
        let draft = Draft { font: font.clone(), opacity, ..Draft::defaults() };
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
            ind_from: (0.0, 0.0),
            ind_t: 1.0,
            clock: None,
            drag: None,
            edit: None,
            // The default family at a fixed size: never the font being edited.
            ui: Fonts::new(&FontCfg::sized(13.0)),
            prop: None,
            blur_supported: true,
            notice: None,
            focus: true,
            tabs_focus: false,
            scroll: 0,
            rect: (0, 0, PANEL_W, 570),
            hairline_left: true,
        }
    }

    /// Puts the panel's box (window pixels) and says whether it is at the window's left edge, so the
    /// hairline is on the grid's side. Called when it opens and whenever the window or the docks change.
    pub fn place(&mut self, rect: Rect, left: bool) {
        self.rect = rect;
        self.hairline_left = !left;
        self.scroll = self.scroll.clamp(0, self.max_scroll());
    }

    /// No gesture is in progress: nothing is being dragged and no number is being typed, so what the draft
    /// says may be written to the file.
    pub fn settled(&self) -> bool {
        self.drag.is_none() && self.edit.is_none() && self.chip_drag.is_none_or(|d| !d.moved)
    }

    /// Takes in a number still being typed, and returns the draft (before it is written).
    pub fn commit(&mut self) -> &Draft {
        self.commit_edit();
        &self.draft
    }

    /// The settings that differ from what the file says, as `config::save` takes them.
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
        num("windows.side_width", d.windows.side_w as f32, o.windows.side_w as f32, format!("{}", d.windows.side_w));
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
        if d.input.autosuggest != o.input.autosuggest {
            out.push(("input.autosuggest", Some(d.input.autosuggest.to_string())));
        }
        if d.input.chips != o.input.chips {
            out.push(("input.chips", Some(config::chip_list_text(&d.input.chips))));
        }
        if d.windows.tabs != o.windows.tabs {
            out.push(("windows.tabs", Some(format!("{:?}", d.windows.tabs.name()))));
        }
        if d.windows.side != o.windows.side {
            out.push(("windows.side", Some(d.windows.side.to_string())));
        }
        out
    }

    /// After a successful save the draft is what the file says.
    pub fn saved(&mut self) {
        self.orig = self.draft.clone();
        self.notice = None;
    }

    /// The windows' settings changed outside the panel (`Alt+B`): the switch follows, and it is not a change to save.
    pub fn follow_windows(&mut self, windows: &WindowsCfg) {
        self.draft.windows.side = windows.side;
        self.orig.windows.side = windows.side;
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

    // --------------------------------------------------------- geometry --

    /// The box's left edge inside it (past the hairline) and the width of what the hairline leaves.
    fn x0(&self) -> i32 {
        if self.hairline_left { 1 } else { 0 }
    }

    fn cw(&self) -> i32 {
        self.rect.2 - 1
    }

    /// Where the controls start and how wide their column is (213 px in the 372 px panel).
    fn ctrl_x(&self) -> i32 {
        self.x0() + ROW_X + ROW_PAD + LABEL_W + COL_GAP
    }

    fn ctrl_w(&self) -> i32 {
        (self.cw() - 2 * ROW_X - 2 * ROW_PAD - LABEL_W - COL_GAP).max(60)
    }

    fn foot_top(&self) -> i32 {
        (self.rect.3 - FOOT_H).max(BODY_Y)
    }

    fn viewport_h(&self) -> i32 {
        self.foot_top() - BODY_Y
    }

    fn inert(&self, row: Row) -> bool {
        match row {
            Row::Clock | Row::Chips => !self.draft.input.inline,
            Row::Side | Row::SideW => self.draft.windows.tabs == TabsPos::Top,
            _ => false,
        }
    }

    /// Where radio `n` of `row` is: its x in the control column and its line, and how many lines there are.
    fn radio_at(&self, row: Row, n: usize) -> (i32, i32, i32) {
        let (labels, count) = options(row);
        let (mut x, mut line) = (0, 0);
        let mut at = (0, 0);
        for (i, label) in labels[..count].iter().enumerate() {
            let w = self.width(label, PX) + RADIO_EXTRA;
            if x > 0 && x + w > self.ctrl_w() {
                x = 0;
                line += 1;
            }
            if i == n {
                at = (x, line);
            }
            x += w + RADIO_GAP;
        }
        (at.0, at.1, line + 1)
    }

    fn radio_w(&self, row: Row, n: usize) -> i32 {
        self.width(options(row).0[n], PX) + RADIO_EXTRA
    }

    fn chips_h() -> i32 {
        let lines = (Chip::COUNT as i32 + CHIP_COLS - 1) / CHIP_COLS;
        lines * CHIP_H + (lines - 1) * CHIP_GAP
    }

    /// A label broken into lines that fit the label column (up to three, the last takes what is left): the
    /// reference lets a long label ("Espaciado de letras") wrap and the row grow with it.
    fn label_lines<'a>(&self, s: &'a str) -> ([&'a str; 3], usize) {
        let mut out = [""; 3];
        let (mut n, mut start, mut end) = (0, 0, 0);
        for (i, c) in s.char_indices().chain([(s.len(), ' ')]) {
            if c != ' ' {
                continue;
            }
            // The words from `start` to `i`: if they do not fit and there is a line before the word, break there.
            if end > start && self.width(&s[start..i], PX) > LABEL_W + LABEL_SLACK && n < 2 {
                out[n] = &s[start..end];
                (n, start) = (n + 1, end + 1);
            }
            end = i;
        }
        out[n] = &s[start..];
        (out, n + 1)
    }

    /// The height `lines` lines of label need (the reference's line height is 18.2 px, with the row's 4 px padding).
    fn label_h(lines: usize) -> i32 {
        if lines <= 1 { 0 } else { (lines as f32 * 18.2).round() as i32 + 8 }
    }

    fn row_h(&self, row: Row) -> i32 {
        let own = match row.kind() {
            Kind::Radios => {
                let lines = self.radio_at(row, 0).2;
                (lines * RADIO_H + (lines - 1) * RADIO_LINE_GAP + 8).max(ROW_MIN)
            }
            Kind::Chips => 2 * CHIPS_PAD + Settings::chips_h(),
            _ => ROW_MIN,
        };
        own.max(Settings::label_h(self.label_lines(row.label()).1))
    }

    /// The note's text width: its box is inset by the body's padding and its margin, and has 12 px inside.
    fn note_w(&self) -> i32 {
        self.cw() - 2 * (ROW_X + ROW_PAD) - 24
    }

    /// The width of a piece of a note as it is drawn.
    fn ink_width(&self, s: &str, ink: Ink) -> i32 {
        match ink {
            Ink::Text => self.width(s, PX_NOTE),
            Ink::Code => self.width(s, PX_CODE) + 10,
            Ink::Key if s == "←" || s == "→" => 10 + 12,
            Ink::Key => self.width(s, PX_KBD) + 12,
        }
    }

    fn note_lines(&self, note: Str) -> i32 {
        flow(tr(note), self.note_w(), &|s, ink| self.ink_width(s, ink), |_, _, _, _| {})
    }

    /// Rows' tops and heights in content coordinates (0 is the first row's top), and the note's box.
    fn lay(&self, page: Page) -> Lay {
        let mut l = Lay { n: rows(page).len(), top: [0; MAX_ROWS], h: [0; MAX_ROWS], note: None, end: 0 };
        let mut y = 0;
        for (i, &row) in rows(page).iter().enumerate() {
            (l.top[i], l.h[i]) = (y, self.row_h(row));
            y += l.h[i];
        }
        if let Some(note) = page.note() {
            let h = 20 + (self.note_lines(note) as f32 * NOTE_LINE).round() as i32;
            l.note = Some((y + NOTE_MARGIN_TOP, h));
            y += NOTE_MARGIN_TOP + h + NOTE_MARGIN_BOTTOM;
        }
        l.end = y;
        l
    }

    fn max_scroll(&self) -> i32 {
        (BODY_TOP + self.lay(self.page).end + BODY_BOTTOM - self.viewport_h()).max(0)
    }

    /// Panel-local y of content coordinate `cy`.
    fn body_y(&self, cy: i32) -> i32 {
        BODY_Y + BODY_TOP + cy - self.scroll
    }

    fn close_rect(&self) -> Rect {
        (self.x0() + self.cw() - 8 - CLOSE, 10, CLOSE, CLOSE)
    }

    fn tab_w(&self, page: Page) -> i32 {
        self.width(page.label(), PX) + 2 * TAB_PAD
    }

    fn tabs_w(&self) -> i32 {
        Page::ALL.iter().map(|&p| self.tab_w(p)).sum::<i32>() + 6
    }

    fn tab_rect(&self, page: Page) -> Rect {
        let before: i32 = Page::ALL[..page.index()].iter().map(|&p| self.tab_w(p)).sum();
        (self.x0() + EDGE + 3 + before, TABS_Y + 3, self.tab_w(page), TAB_H)
    }

    fn button_rect(&self) -> Rect {
        (self.x0() + EDGE, self.foot_top() + 1 + 10, self.width(tr(Str::Reset), PX) + 32, BTN_H)
    }

    /// Chip slot `i` in the grid: x in the panel and y in content coordinates.
    fn slot_rect(&self, i: usize) -> Rect {
        let (col, line) = (i as i32 % CHIP_COLS, i as i32 / CHIP_COLS);
        let w = (self.ctrl_w() - (CHIP_COLS - 1) * CHIP_GAP) / CHIP_COLS;
        let top = self.lay(Page::Input).top[Row::Chips.index()] + CHIPS_PAD;
        (self.ctrl_x() + col * (w + CHIP_GAP), top + line * (CHIP_H + CHIP_GAP), w, CHIP_H)
    }

    /// A slider's track: where it starts and how long it is.
    fn track(&self) -> (i32, i32) {
        (self.ctrl_x(), (self.ctrl_w() - VALUE_W - VALUE_GAP).max(3 * THUMB_R))
    }

    /// The number box at a slider's right, in a row whose vertical centre is `cy`.
    fn value_rect(&self, cy: i32) -> Rect {
        let (tx, tw) = self.track();
        (tx + tw + VALUE_GAP, cy - VALUE_H / 2, VALUE_W, VALUE_H)
    }

    /// Where along its track (0..1) the pointer's panel x is: the knob's centre travels 7 px inside the ends.
    fn along(&self, lx: f32) -> f32 {
        let (tx, tw) = self.track();
        ((lx - tx as f32 - THUMB_R as f32) / (tw - 2 * THUMB_R) as f32).clamp(0.0, 1.0)
    }

    // ---------------------------------------------------------- pointer --

    /// What is at window pixel `(x, y)`.
    pub fn hit(&self, x: f32, y: f32) -> Hit {
        let (bx, by, bw, bh) = self.rect;
        let (lx, ly) = (x.floor() as i32 - bx, y.floor() as i32 - by);
        if lx < 0 || ly < 0 || lx >= bw || ly >= bh {
            return Hit::Outside;
        }
        if inside(lx, ly, self.close_rect()) {
            return Hit::Close;
        }
        if let Some(page) = Page::ALL.into_iter().find(|&p| inside(lx, ly, self.tab_rect(p))) {
            return Hit::Tab(page);
        }
        if inside(lx, ly, self.button_rect()) {
            return Hit::Defaults;
        }
        if ly < BODY_Y || ly >= self.foot_top() {
            return Hit::Panel;
        }
        let cy = ly - BODY_Y - BODY_TOP + self.scroll;
        let lay = self.lay(self.page);
        for (i, &row) in rows(self.page).iter().enumerate() {
            if cy < lay.top[i] || cy >= lay.top[i] + lay.h[i] {
                continue;
            }
            if self.inert(row) || lx < self.x0() + ROW_X || lx >= self.x0() + self.cw() - ROW_X {
                return Hit::Panel;
            }
            return self.hit_row(row, lx, cy, (lay.top[i], lay.h[i]));
        }
        Hit::Panel
    }

    fn hit_row(&self, row: Row, lx: i32, cy: i32, (top, h): (i32, i32)) -> Hit {
        let mid = top + h / 2;
        let (cx, cw) = (self.ctrl_x(), self.ctrl_w());
        match row.kind() {
            Kind::Picker => {
                if (cy - mid).abs() < ARROW / 2 {
                    if (cx..cx + ARROW).contains(&lx) {
                        return Hit::Arrow(-1);
                    }
                    if (cx + cw - ARROW..cx + cw).contains(&lx) {
                        return Hit::Arrow(1);
                    }
                }
            }
            Kind::Switch => {
                if inside(lx, cy, (cx, mid - SWITCH.1 / 2, SWITCH.0, SWITCH.1)) {
                    return Hit::Toggle(row);
                }
            }
            Kind::Slider => {
                let (tx, tw) = self.track();
                if (cy - mid).abs() < 10 && (tx - 4..tx + tw + 4).contains(&lx) {
                    return Hit::Slider(row, self.along(lx as f32));
                }
                if inside(lx, cy, self.value_rect(mid)) {
                    return Hit::Value(row);
                }
            }
            Kind::Radios => {
                let count = options(row).1;
                let lines = self.radio_at(row, 0).2;
                let block = lines * RADIO_H + (lines - 1) * RADIO_LINE_GAP;
                let y0 = top + (h - block) / 2;
                for n in 0..count {
                    let (x, line, _) = self.radio_at(row, n);
                    if inside(lx, cy, (cx + x, y0 + line * (RADIO_H + RADIO_LINE_GAP), self.radio_w(row, n), RADIO_H)) {
                        return Hit::Choice(row, n);
                    }
                }
            }
            Kind::Chips => {
                let slots = self.slots();
                if let Some(i) = (0..Chip::COUNT).find(|&i| inside(lx, cy, self.slot_rect(i))) {
                    return Hit::Chip(slots[i]);
                }
            }
        }
        Hit::Row(row)
    }

    /// The pointer moved to window pixel `(x, y)`. Returns what the caller has to do.
    pub fn pointer(&mut self, x: f32, y: f32) -> Effect {
        let bx = self.rect.0;
        if let Some(row) = self.drag {
            let t = self.along(x - bx as f32);
            let (lo, hi, _) = row.range();
            let before = self.draft.clone();
            self.draft.set(row, lo + t * (hi - lo));
            self.set_hover(Hit::Slider(row, t));
            return if self.draft != before { Effect::Apply } else { Effect::None };
        }
        if let Some(d) = self.chip_drag {
            let at = self.content_at(x, y);
            let moved = d.moved || (at.0 - d.from.0).abs().max((at.1 - d.from.1).abs()) > DRAG_SLOP;
            self.chip_drag = Some(ChipDrag { at, moved, ..d });
            if !moved {
                return Effect::None;
            }
            self.set_hover(Hit::Chip(d.chip));
            let before = self.draft.input.chips.clone();
            self.place_chip(d.chip, self.slot_near(at));
            self.chip_sel = self.slots().iter().position(|&c| c == d.chip).unwrap_or(0);
            return if self.draft.input.chips != before { Effect::Apply } else { Effect::Redraw };
        }
        let now = self.hit(x, y);
        // A slider's hover does not depend on where along the track the pointer is.
        let same = |a: Hit, b: Hit| match (a, b) {
            (Hit::Slider(r, _), Hit::Slider(s, _)) => r == s,
            _ => a == b,
        };
        let changed = !same(now, self.hover);
        self.set_hover(now);
        if changed { Effect::Redraw } else { Effect::None }
    }

    /// Window pixel `(x, y)` in body content coordinates (panel x, y under the tabs plus the scroll).
    fn content_at(&self, x: f32, y: f32) -> (i32, i32) {
        (x.floor() as i32 - self.rect.0, y.floor() as i32 - self.rect.1 - BODY_Y - BODY_TOP + self.scroll)
    }

    /// The primary button went down at window pixel `(x, y)`.
    pub fn press(&mut self, x: f32, y: f32) -> Effect {
        self.focus = true;
        let hit = self.hit(x, y);
        self.set_hover(hit);
        if hit.row().is_some() {
            self.tabs_focus = false;
        }
        // Any press ends a number being typed, keeping it; clicking the same box again starts over.
        let committed = self.commit_edit();
        let again = matches!((hit, committed), (Hit::Value(r), Some((c, _))) if r == c);
        let e = self.press_on(hit, again);
        if let Hit::Chip(chip) = hit {
            let at = self.content_at(x, y);
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
                self.tabs_focus = true;
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
            Hit::Row(row) => {
                self.sel = row.index();
                Effect::Redraw
            }
            Hit::Close => Effect::Close,
            Hit::Panel | Hit::Outside => Effect::None,
        }
    }

    /// Shows another page, with its first row selected, scrolled to the top.
    fn set_page(&mut self, page: Page) {
        self.commit_edit();
        if page != self.page {
            let r = self.tab_rect(self.page);
            let (x, w) = (mix(self.ind_from.0, r.0 as f32, ease(self.ind_t)), mix(self.ind_from.1, r.2 as f32, ease(self.ind_t)));
            self.ind_from = (x, w);
            self.ind_t = 0.0;
            self.clock = None;
            self.page = page;
            self.sel = 0;
            self.scroll = 0;
            self.chip_drag = None;
        }
    }

    /// Option `n` of a radio row.
    fn choose(&mut self, row: Row, n: usize) {
        match row {
            Row::Mode => self.draft.input.inline = n == 1,
            Row::Clock => self.draft.input.clock24 = n == 1,
            Row::Cursor => self.draft.input.cursor = CursorKind::ALL[n.min(2)],
            Row::Language => self.draft.lang = i18n::Setting::ALL[n.min(2)],
            Row::Tabs => {
                self.draft.windows.tabs = TabsPos::ALL[n.min(2)];
                // Picking a side shows the panel there, as the reference does.
                if self.draft.windows.tabs != TabsPos::Top {
                    self.draft.windows.side = true;
                }
            }
            _ => {}
        }
    }

    /// The option a radio row is on.
    fn chosen(&self, row: Row) -> usize {
        match row {
            Row::Mode => self.draft.input.inline as usize,
            Row::Cursor => CursorKind::ALL.iter().position(|&c| c == self.draft.input.cursor).unwrap_or(0),
            Row::Language => i18n::Setting::ALL.iter().position(|&l| l == self.draft.lang).unwrap_or(0),
            Row::Tabs => TabsPos::ALL.iter().position(|&t| t == self.draft.windows.tabs).unwrap_or(0),
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
    fn place_chip(&mut self, c: Chip, to: usize) {
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

    /// The slot nearest to content position `(x, y)`: the pointer may leave the grid while dragging.
    fn slot_near(&self, (x, y): (i32, i32)) -> usize {
        let (x0, y0, w, h) = self.slot_rect(0);
        let lines = (Chip::COUNT as i32 + CHIP_COLS - 1) / CHIP_COLS;
        let col = ((x - x0).div_euclid(w + CHIP_GAP)).clamp(0, CHIP_COLS - 1);
        let line = ((y - y0).div_euclid(h + CHIP_GAP)).clamp(0, lines - 1);
        ((line * CHIP_COLS + col) as usize).min(Chip::COUNT - 1)
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

    /// Shows chip `chip` picked up and held at body content position `at` (the design board's drag scene).
    #[cfg(test)]
    pub fn hold(&mut self, chip: Chip, at: (i32, i32)) {
        self.chip_drag = Some(ChipDrag { chip, from: at, at, moved: true });
        self.hover = Hit::Chip(chip);
    }

    /// The primary button went up: a drag ends, and a press on a chip that never moved switches it.
    pub fn release(&mut self) -> Effect {
        let dragged = self.drag.take().is_some();
        match self.chip_drag.take() {
            Some(d) if !d.moved => {
                self.toggle_chip(d.chip);
                // The grid reorders (a chip that went off moves to the end): the cursor stays on the chip.
                self.chip_sel = self.slots().iter().position(|&c| c == d.chip).unwrap_or(0);
                Effect::Apply
            }
            Some(_) => Effect::Settled,
            None if dragged => Effect::Settled,
            None => Effect::None,
        }
    }

    /// The wheel: scrolls the body by `px` pixels (positive is down).
    pub fn scroll_by(&mut self, px: i32) -> Effect {
        let to = (self.scroll + px).clamp(0, self.max_scroll());
        if to == self.scroll {
            return Effect::None;
        }
        self.scroll = to;
        Effect::Redraw
    }

    /// Scrolls just enough to show content rows `top..top + h`.
    fn reveal(&mut self, top: i32, h: i32) {
        let view = self.viewport_h() - BODY_TOP - BODY_BOTTOM;
        if top < self.scroll {
            self.scroll = top;
        } else if top + h > self.scroll + view {
            self.scroll = top + h - view;
        }
        self.scroll = self.scroll.clamp(0, self.max_scroll());
    }

    /// Brings the selected row into view.
    fn reveal_sel(&mut self) {
        if self.tabs_focus {
            self.scroll = 0;
            return;
        }
        let lay = self.lay(self.page);
        let i = self.sel.min(lay.n - 1);
        // The first row also shows what is above it (the body's padding), the last what is below.
        let (top, h) = (lay.top[i], lay.h[i]);
        self.reveal(top, h);
        if i == 0 {
            self.scroll = 0;
        } else if i == lay.n - 1 && self.page.note().is_none() {
            self.scroll = self.scroll.max(self.max_scroll());
        }
    }

    // --------------------------------------------------------- keyboard --

    /// A key while the panel has the keyboard.
    pub fn key(&mut self, k: &Key) -> Effect {
        if self.edit.is_some() {
            return self.edit_key(k);
        }
        let row = self.row();
        if self.tabs_focus && !k.mods.ctrl && !k.mods.alt {
            match k.sym {
                // On the tab bar the arrows change page; down, or Tab, go to the rows, up to the last row.
                sym::LEFT | sym::RIGHT => {
                    let n = (self.page.index() as i32 + if k.sym == sym::RIGHT { 1 } else { -1 }).rem_euclid(Page::ALL.len() as i32);
                    self.set_page(Page::ALL[n as usize]);
                    return Effect::Redraw;
                }
                sym::DOWN | sym::TAB if !k.mods.shift => return self.move_sel(1),
                sym::UP | sym::ISO_LEFT_TAB | sym::TAB => return self.move_sel(-1),
                sym::ESCAPE => return Effect::Close,
                sym::PAGE_UP | sym::PAGE_DOWN => {}
                _ => return Effect::None,
            }
        }
        // Typing a number on a slider row starts typing it.
        if row.is_number() && !self.inert(row) && !k.mods.ctrl && !k.mods.alt {
            if k.text.chars().next().is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-')) {
                self.start_edit(row, Some(&k.text));
                return Effect::Redraw;
            }
        }
        let big = if k.mods.shift { 5.0 } else { 1.0 };
        match k.sym {
            sym::ESCAPE => Effect::Close,
            sym::PAGE_DOWN | sym::PAGE_UP => {
                let n = (self.page.index() as i32 + if k.sym == sym::PAGE_DOWN { 1 } else { -1 }).rem_euclid(Page::ALL.len() as i32);
                self.set_page(Page::ALL[n as usize]);
                Effect::Redraw
            }
            sym::TAB if k.mods.ctrl => {
                self.set_page(Page::ALL[(self.page.index() + 1) % Page::ALL.len()]);
                Effect::Redraw
            }
            sym::TAB if k.mods.shift => self.move_sel(-1),
            sym::ISO_LEFT_TAB | sym::UP => self.move_sel(-1),
            sym::TAB | sym::DOWN => self.move_sel(1),
            sym::LEFT if k.mods.alt && row == Row::Chips => self.nudge(-1),
            sym::RIGHT if k.mods.alt && row == Row::Chips => self.nudge(1),
            sym::LEFT => self.adjust(-big),
            sym::RIGHT => self.adjust(big),
            sym::SPACE | sym::RETURN | sym::KP_ENTER if matches!(row.kind(), Kind::Switch | Kind::Radios | Kind::Chips) => self.space(),
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

    /// Where the tab marker is: its x and width.
    fn marker(&self) -> (f32, f32) {
        let r = self.tab_rect(self.page);
        let t = ease(self.ind_t);
        (mix(self.ind_from.0, r.0 as f32, t), mix(self.ind_from.1, r.2 as f32, t))
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
            sym::RETURN | sym::KP_ENTER => self.finish_edit(0),
            sym::TAB if k.mods.shift => self.finish_edit(-1),
            sym::ISO_LEFT_TAB => self.finish_edit(-1),
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
            self.move_sel(d);
        }
        if changed { Effect::Apply } else { Effect::Redraw }
    }

    /// Moves the keyboard `d` stops: the tab bar, then the rows of the page, round again; the rows that are
    /// inert are skipped.
    fn move_sel(&mut self, d: i32) -> Effect {
        let n = rows(self.page).len() as i32 + 1;
        // Stop 0 is the tab bar, stop `i + 1` is row `i`.
        let mut i = if self.tabs_focus { 0 } else { self.sel as i32 + 1 };
        for _ in 0..n {
            i = (i + d).rem_euclid(n);
            if i == 0 || !self.inert(rows(self.page)[i as usize - 1]) {
                break;
            }
        }
        self.tabs_focus = i == 0;
        if i > 0 {
            self.sel = i as usize - 1;
        }
        self.reveal_sel();
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
            Row::Autosuggest => self.draft.input.autosuggest = !self.draft.input.autosuggest,
            Row::Side => self.draft.windows.side = !self.draft.windows.side,
            _ => {}
        }
    }

    /// Space on a row: flips the switch, picks the next radio, switches the chip under the cursor.
    fn space(&mut self) -> Effect {
        let row = self.row();
        match row.kind() {
            Kind::Chips => {
                let c = self.slots()[self.chip_sel];
                self.toggle_chip(c);
                self.chip_sel = self.slots().iter().position(|&x| x == c).unwrap_or(0);
            }
            Kind::Switch => self.flip(row),
            _ => self.choose(row, (self.chosen(row) + 1) % options(row).1),
        }
        Effect::Apply
    }

    /// Left and right on the selected row: `n` steps of its slider, the next font, the switch, the other radio.
    fn adjust(&mut self, n: f32) -> Effect {
        let row = self.row();
        match row.kind() {
            Kind::Picker => self.pick(n.signum() as i32),
            Kind::Switch => self.flip(row),
            Kind::Radios => {
                let count = options(row).1 as i32;
                let to = (self.chosen(row) as i32 + n.signum() as i32).rem_euclid(count);
                self.choose(row, to as usize)
            }
            Kind::Chips => {
                self.chip_sel = (self.chip_sel as i32 + n.signum() as i32).rem_euclid(Chip::COUNT as i32) as usize;
                return Effect::Redraw;
            }
            Kind::Slider => {
                let (_, _, step) = row.range();
                let v = self.draft.get(row) + n * step;
                self.draft.set(row, v);
            }
        }
        Effect::Apply
    }

    // ------------------------------------------------------------- paint --

    /// Paints the panel in its box, which the caller has cleared (`Soft::clear_rect`). `(ox, oy)` is where
    /// the window starts in the `w` x `h` framebuffer.
    pub fn draw(&self, fb: &mut [u32], w: usize, h: usize, (ox, oy): (usize, usize)) {
        let (_, _, bw, bh) = self.rect;
        if fb.len() < w * h || bw <= 0 || bh <= 0 {
            return;
        }
        let fade = ease(self.fade);
        let (px0, py0) = (ox as i32 + self.rect.0, oy as i32 + self.rect.1);
        let fg = palette::unit(palette::FG);
        let dim = palette::unit(palette::DIM);
        let accent = palette::unit(palette::ACCENT);
        let x0 = self.x0();

        // Header, tabs, hairline.
        {
            let mut p = Paint { fb: &mut *fb, w, h, ox: px0, oy: py0, fade };
            let hx = if self.hairline_left { 0 } else { bw - 1 };
            p.shape(hx, 0, hx + 1, bh, WHITE, 0.07, &|_, _| 1.0);

            self.text(&mut p, x0 + EDGE, 10 + CLOSE / 2, tr(Str::SettingsTitle), PX_TITLE, fg, 1.0, true);
            let (cx, cy, cwid, chei) = self.close_rect();
            let lit = self.lit(&|h| h == Hit::Close);
            if lit > 0.0 {
                p.rrect(cx, cy, cwid, chei, 8.0, WHITE, 0.07 * lit);
            }
            let col = mix3(dim, fg, lit);
            let (mx, my) = (cx as f32 + cwid as f32 / 2.0, cy as f32 + chei as f32 / 2.0);
            p.shape(cx, cy, cx + cwid, cy + chei, col, 1.0, &|px, py| {
                let d = seg(px, py, (mx - 4.0, my - 4.0), (mx + 4.0, my + 4.0)).min(seg(px, py, (mx + 4.0, my - 4.0), (mx - 4.0, my + 4.0)));
                (1.1 - d).clamp(0.0, 1.0)
            });

            // The tabs: a rounded container with a marker that slides to the page being shown.
            p.rrect(x0 + EDGE, TABS_Y, self.tabs_w(), TABS_H, 10.0, WHITE, 0.05);
            let (mxp, mwp) = self.marker();
            p.rrect(mxp.round() as i32, TABS_Y + 3, mwp.round() as i32, TAB_H, 8.0, WHITE, 0.10);
            for page in Page::ALL {
                let (x, y, tw, th) = self.tab_rect(page);
                let (on, lit) = (page == self.page, self.lit(&|h| h == Hit::Tab(page)));
                let label = page.label();
                let c = if on { fg } else { mix3(dim, fg, lit) };
                self.text(&mut p, x + (tw - self.width(label, PX)) / 2, y + th / 2, label, PX, c, 1.0, false);
                // The keyboard is on the tab bar: a 2 px ring, 2 px off the active tab (the reference's focus outline).
                if on && self.focus && self.tabs_focus {
                    p.outline(x - 3, y - 3, tw + 6, th + 6, 11.0, accent, 0.7);
                    p.outline(x - 4, y - 4, tw + 8, th + 8, 12.0, accent, 0.7);
                }
            }
        }

        // The body, clipped to its viewport: only the rows of the framebuffer between the tabs and the footer
        // are lent to the painter, so what is scrolled out of view cannot draw over the header or the footer.
        let (r0, r1) = ((py0 + BODY_Y).max(0) as usize, ((py0 + self.foot_top()).max(0) as usize).min(h));
        if r0 < r1 {
            let mut p = Paint { fb: &mut fb[r0 * w..r1 * w], w, h: r1 - r0, ox: px0, oy: -BODY_Y, fade };
            self.draw_body(&mut p, fg, dim, accent);
        }

        // The footer: a hairline and the reset button, and a line when a save failed.
        {
            let mut p = Paint { fb: &mut *fb, w, h, ox: px0, oy: py0, fade };
            let ft = self.foot_top();
            p.shape(x0, ft, x0 + self.cw(), ft + 1, WHITE, 0.07, &|_, _| 1.0);
            let (bx, by, bwid, bhei) = self.button_rect();
            let lit = self.lit(&|h| h == Hit::Defaults);
            p.rrect(bx, by, bwid, bhei, 9.0, WHITE, mix(0.07, 0.12, lit));
            let label = tr(Str::Reset);
            self.text(&mut p, bx + (bwid - self.width(label, PX)) / 2, by + bhei / 2, label, PX, fg, 1.0, false);
            if let Some(n) = &self.notice {
                let room = self.cw() - (bx + bwid - x0) - EDGE - 12;
                let mut shown = n.clone();
                while self.width(&shown, PX_NOTE) > room && shown.chars().count() > 1 {
                    shown = shown.chars().take(shown.chars().count() - 2).chain(['\u{2026}']).collect();
                }
                self.text(&mut p, bx + bwid + 12, by + bhei / 2, &shown, PX_NOTE, accent, 1.0, false);
            }
        }
    }

    /// The rows, the note and the scrollbar, in panel-local y (the painter is clipped to the body).
    fn draw_body(&self, p: &mut Paint, fg: [f32; 3], dim: [f32; 3], accent: [f32; 3]) {
        const OFF: [f32; 3] = [0.173, 0.173, 0.192]; // #2c2c31: a switch or a slider track at rest
        const RING: [f32; 3] = [0.353, 0.353, 0.384]; // #5a5a62: an unchosen radio
        const THUMB: [f32; 3] = [0.957, 0.957, 0.961]; // #f4f4f5
        let x0 = self.x0();
        let lay = self.lay(self.page);
        let (cx, cwid) = (self.ctrl_x(), self.ctrl_w());
        for (i, &row) in rows(self.page).iter().enumerate() {
            let (top, h) = (self.body_y(lay.top[i]), lay.h[i]);
            if top + h < BODY_Y || top > self.foot_top() {
                continue;
            }
            let mid = top + h / 2;
            let inert = self.inert(row);
            let hovered = if inert { 0.0 } else { self.lit(&|hh| hh.row() == Some(row)) };
            let selected = self.focus && !self.tabs_focus && i == self.sel && !inert;
            let a = if selected { 0.07 } else { 0.04 * hovered };
            if a > 0.0 {
                p.rrect(x0 + ROW_X, top, self.cw() - 2 * ROW_X, h, 10.0, WHITE, a);
            }
            // A row that does nothing now is faded, as the reference's inert rows.
            let live = if inert { 0.38 } else { 1.0 };
            let glow = if selected { 1.0f32.max(hovered) } else { hovered };
            let (lines, n) = self.label_lines(row.label());
            // Centred on the row (the chips' row starts at its top); a wrapped label's lines are 18.2 px apart.
            let first = if row.kind() == Kind::Chips { top + CHIPS_PAD + 9 } else { mid - ((n as f32 - 1.0) * 9.1).round() as i32 };
            for (k, line) in lines[..n].iter().enumerate() {
                let y = first + (k as f32 * 18.2).round() as i32;
                self.text(p, x0 + ROW_X + ROW_PAD, y, line, PX, mix3(dim, fg, glow), live, false);
            }
            match row.kind() {
                Kind::Picker => {
                    let right = cx + cwid - ARROW;
                    for (x, dir) in [(cx, -1), (right, 1)] {
                        let lit = self.lit(&|hh| hh == Hit::Arrow(dir));
                        p.rrect(x, mid - ARROW / 2, ARROW, ARROW, 8.0, WHITE, mix(0.06, 0.12, lit));
                        p.chevron(x + ARROW / 2, mid, dir, fg, mix(0.80, 1.0, lit));
                    }
                    let (nx, nw) = (cx + ARROW + PICKER_GAP, cwid - 2 * ARROW - 2 * PICKER_GAP);
                    p.rrect(nx, mid - ARROW / 2, nw, ARROW, 8.0, WHITE, 0.04);
                    let label = self.family_label();
                    let mut shown = label.to_string();
                    while self.width(&shown, PX) > nw - 16 && shown.chars().count() > 1 {
                        shown = label.chars().take(shown.chars().count().saturating_sub(2)).chain(['\u{2026}']).collect();
                    }
                    self.text(p, nx + (nw - self.width(&shown, PX)) / 2, mid, &shown, PX, fg, 1.0, false);
                }
                Kind::Switch => {
                    let on = match row {
                        Row::Blur => self.draft.blur,
                        Row::Bold => self.draft.font.bold,
                        Row::Autosuggest => self.draft.input.autosuggest,
                        _ => self.draft.windows.side,
                    };
                    let (sw, sh) = SWITCH;
                    p.rrect(cx, mid - sh / 2, sw, sh, sh as f32 / 2.0, if on { accent } else { OFF }, live);
                    let kx = if on { cx + sw - 3 - 8 } else { cx + 3 + 8 };
                    p.disc(kx as f32, mid as f32, 8.0, if on { [0.102, 0.075, 0.031] } else { [0.91, 0.91, 0.918] }, live);
                    let note = match (row, on, self.blur_supported) {
                        (Row::Blur, true, false) => tr(Str::OnUnsupported),
                        (Row::Blur, false, false) => tr(Str::OffUnsupported),
                        (_, true, _) => tr(Str::On),
                        _ => tr(Str::Off),
                    };
                    self.text(p, cx + sw + 10, mid, note, PX, dim, live, false);
                }
                Kind::Radios => {
                    let count = options(row).1;
                    let lines = self.radio_at(row, 0).2;
                    let block = lines * RADIO_H + (lines - 1) * RADIO_LINE_GAP;
                    let y0 = top + (h - block) / 2;
                    for n in 0..count {
                        let (rx, line, _) = self.radio_at(row, n);
                        let (x, y, rw) = (cx + rx, y0 + line * (RADIO_H + RADIO_LINE_GAP), self.radio_w(row, n));
                        let (on, lit) = (self.chosen(row) == n, if inert { 0.0 } else { self.lit(&|hh| hh == Hit::Choice(row, n)) });
                        if lit > 0.0 {
                            p.rrect(x, y, rw, RADIO_H, 8.0, WHITE, 0.04 * lit);
                        }
                        // A radio button: a ring, a dot when chosen (the accent).
                        let (rcx, rcy) = ((x + 8 + 7) as f32, (y + RADIO_H / 2) as f32);
                        p.ring(rcx, rcy, 6.25, 1.5, if on { accent } else { RING }, live);
                        if on {
                            p.disc(rcx, rcy, 3.0, accent, live);
                        }
                        let c = if on { fg } else { mix3(dim, fg, lit) };
                        self.text(p, x + 8 + 14 + 8, y + RADIO_H / 2, options(row).0[n], PX, c, live, false);
                    }
                }
                Kind::Chips => self.draw_chips(p, selected, dim, accent),
                Kind::Slider => {
                    let (lo, hi, _) = row.range();
                    let t = ((self.draft.get(row) - lo) / (hi - lo)).clamp(0.0, 1.0);
                    let (tx, tw) = self.track();
                    let kx = tx as f32 + THUMB_R as f32 + t * (tw - 2 * THUMB_R) as f32;
                    let lit = if self.drag == Some(row) { 1.0 } else { self.lit(&|hh| matches!(hh, Hit::Slider(r, _) if r == row)) };
                    p.rrect(tx, mid - 2, tw, 4, 2.0, OFF, live);
                    p.rrect(tx, mid - 2, (kx - tx as f32).round() as i32, 4, 2.0, accent, live);
                    let r = mix(THUMB_R as f32, THUMB_R as f32 * 1.15, lit);
                    p.disc(kx, mid as f32 + 1.0, r, BLACK, 0.35 * live);
                    p.disc(kx, mid as f32, r, THUMB, live);
                    self.draw_value(p, row, mid, fg, accent);
                }
            }
        }
        if let Some((nt, nh)) = lay.note {
            self.draw_note(p, self.body_y(nt), nh, fg, dim);
        }
        // A thin scrollbar at the right edge when the body is longer than its window.
        let (total, view) = (BODY_TOP + lay.end + BODY_BOTTOM, self.viewport_h());
        if total > view && view > 0 {
            let (th, travel) = ((view * view / total).max(24), view - (view * view / total).max(24));
            let ty = BODY_Y + travel * self.scroll / (total - view).max(1);
            p.rrect(x0 + self.cw() - 8, ty, 6, th, 3.0, [0.165, 0.165, 0.18], 1.0);
        }
    }

    /// The number at a slider's right: plain, outlined under the pointer, or the box being typed into.
    fn draw_value(&self, p: &mut Paint, row: Row, mid: i32, fg: [f32; 3], accent: [f32; 3]) {
        let (vx, vy, vw, vh) = self.value_rect(mid);
        let right = vx + vw - 6;
        match self.edit.as_ref().filter(|e| e.row == row) {
            // The box being typed into: a lit field, the text, a caret after it.
            Some(e) => {
                p.rrect(vx, vy, vw, vh, 6.0, BLACK, 0.35);
                p.outline(vx, vy, vw, vh, 6.0, accent, 0.6);
                let tw = self.width(&e.text, PX);
                let tx = right - tw;
                self.text(p, tx, mid, &e.text, PX, fg, 1.0, false);
                if e.selected {
                    p.rrect(tx - 2, mid - 9, tw + 4, 18, 3.0, accent, 0.28);
                } else {
                    p.rrect(right + 1, mid - 8, 2, 16, 1.0, accent, 0.95);
                }
            }
            None => {
                let v = self.draft.text(row);
                let lit = self.lit(&|h| matches!(h, Hit::Value(r) if r == row));
                if lit > 0.0 {
                    p.outline(vx, vy, vw, vh, 6.0, WHITE, 0.10 * lit);
                }
                self.text(p, right - self.width(&v, PX), mid, &v, PX, fg, 1.0, false);
            }
        }
    }

    /// The chips' grid of the input page.
    fn draw_chips(&self, p: &mut Paint, selected: bool, dim: [f32; 3], accent: [f32; 3]) {
        let inert = self.inert(Row::Chips);
        let live = if inert { 0.38 } else { 1.0 };
        let slots = self.slots();
        let held = self.chip_drag.filter(|d| d.moved);
        let chip_at = |p: &mut Paint, chip: Chip, (x, y, cw, ch): Rect, on: bool, a: f32, lit: f32, focus: bool| {
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
            let dot = (x + 10 + 3) as f32 + 0.5;
            if on {
                p.disc(dot, (y + ch / 2) as f32, 3.5, color, a);
            } else {
                p.ring(dot, (y + ch / 2) as f32, 2.75, 1.5, dim, a);
            }
            self.text(p, x + 10 + 7 + 8, y + ch / 2, chip_label(chip), PX, if on { color } else { dim }, a, false);
        };
        for (n, &chip) in slots.iter().enumerate() {
            let (x, cy, cw, ch) = self.slot_rect(n);
            let on = self.draft.input.chips.contains(&chip);
            // The chip being dragged leaves its slot as a faint ghost; it is drawn again under the pointer.
            let ghost = held.is_some_and(|d| d.chip == chip);
            let a = if ghost { 0.35 } else { live };
            let lit = if ghost || inert { 0.0 } else { self.lit(&|hh| hh == Hit::Chip(chip)) };
            chip_at(p, chip, (x, self.body_y(cy), cw, ch), on, a, lit, selected && self.chip_sel == n);
        }
        if let Some(d) = held {
            let (_, _, cw, ch) = self.slot_rect(0);
            let (x, y) = (d.at.0 - cw / 2, self.body_y(d.at.1) - ch / 2);
            // Lifted: a soft shadow under it, then the chip itself.
            p.rrect(x + 1, y + 4, cw, ch, 9.0, BLACK, 0.35);
            p.rrect(x, y, cw, ch, 9.0, [0.06, 0.06, 0.07], 1.0);
            chip_at(p, d.chip, (x, y, cw, ch), self.draft.input.chips.contains(&d.chip), 1.0, 1.0, false);
        }
    }

    /// The note under the rows: a quiet box with the paragraphs wrapped, keycaps and code drawn as such.
    fn draw_note(&self, p: &mut Paint, top: i32, h: i32, fg: [f32; 3], dim: [f32; 3]) {
        let Some(note) = self.page.note() else { return };
        let x = self.x0() + ROW_X + ROW_PAD;
        let w = self.cw() - 2 * (ROW_X + ROW_PAD);
        p.rrect(x, top, w, h, 10.0, WHITE, 0.03);
        let (tx, ty) = (x + 12, top + 10);
        flow(tr(note), self.note_w(), &|s, ink| self.ink_width(s, ink), |word, ink, wx, line| {
            let cy = ty + (line as f32 * NOTE_LINE + NOTE_LINE / 2.0).round() as i32;
            let x = tx + wx;
            match ink {
                Ink::Text => self.text(p, x, cy, word, PX_NOTE, dim, 1.0, false),
                Ink::Code => {
                    let bw = self.ink_width(word, ink);
                    p.rrect(x, cy - 9, bw, 18, 4.0, WHITE, 0.07);
                    self.text(p, x + 5, cy, word, PX_CODE, fg, 1.0, false);
                }
                Ink::Key => {
                    let bw = self.ink_width(word, ink);
                    // A keycap: a hairline border, a thicker foot, a faint fill.
                    p.rrect(x, cy - 8, bw, 17, 4.0, WHITE, 0.07);
                    p.rrect(x + 1, cy - 7, bw - 2, 14, 3.0, [0.07, 0.07, 0.08], 1.0);
                    p.rrect(x + 1, cy - 7, bw - 2, 14, 3.0, WHITE, 0.04);
                    if word == "←" || word == "→" {
                        let d = if word == "←" { -1.0 } else { 1.0 };
                        let (ax, ay) = (x as f32 + bw as f32 / 2.0, cy as f32);
                        p.shape(x, cy - 7, x + bw, cy + 8, fg, 1.0, &|px, py| {
                            let tip = (ax + 4.0 * d, ay);
                            let d1 = seg(px, py, (ax - 4.0 * d, ay), tip);
                            let d2 = seg(px, py, tip, (ax + 1.0 * d, ay - 3.0)).min(seg(px, py, tip, (ax + 1.0 * d, ay + 3.0)));
                            (1.0 - d1.min(d2)).clamp(0.0, 1.0)
                        });
                    } else {
                        self.text(p, x + 6, cy, word, PX_KBD, fg, 1.0, false);
                    }
                }
            }
        });
    }

    /// `s` with its left edge at `x` and its vertical centre at `cy` (panel-local pixels), `px` high.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, p: &mut Paint, x: i32, cy: i32, s: &str, px: f32, c: [f32; 3], a: f32, bold: bool) {
        if let Some(f) = &self.prop {
            // Centred on the middle of the capitals, the way the eye reads a label.
            let base = cy + (f.cap_height(px) / 2.0).round() as i32;
            f.draw(s, px, x, base, bold, |gx, gy, cov| p.put(gx, gy, c, a * cov as f32 / 255.0));
            return;
        }
        let (cw, ch) = (self.ui.cell_w as i32, self.ui.cell_h as i32);
        let step = ((cw as f32 * px / PX).round() as i32).max(1);
        for (n, c0) in s.chars().enumerate() {
            if c0 == ' ' {
                continue;
            }
            let bmp = self.ui.raster(c0, bold, false);
            let (gx, gy) = (x + n as i32 * step, cy - ch / 2);
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

    /// The width of `s` at `px` as `text` draws it.
    fn width(&self, s: &str, px: f32) -> i32 {
        match &self.prop {
            Some(f) => f.width(s, px),
            None => s.chars().count() as i32 * ((self.ui.cell_w as f32 * px / PX).round() as i32).max(1),
        }
    }
}

/// Where the rows and the note of a page are, in content coordinates.
struct Lay {
    n: usize,
    top: [i32; MAX_ROWS],
    h: [i32; MAX_ROWS],
    /// The note's top and height.
    note: Option<(i32, i32)>,
    /// Where the content ends, margins included.
    end: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xkb::Mods;
    use std::time::{Duration, Instant};

    fn fams() -> Vec<(String, PathBuf)> {
        vec![("Alpha Mono".into(), "/fonts/Alpha.ttf".into()), ("Beta Mono".into(), "/fonts/Beta.ttf".into())]
    }

    /// A panel in its default box: `(0, 0, 372, 570)`, the hairline at its left edge.
    fn open() -> Settings {
        Settings::new(&FontCfg::default(), 0.88, fams())
    }

    fn key(sym: u32, shift: bool) -> Key {
        Key { sym, text: String::new(), mods: Mods { shift, ctrl: false, alt: false } }
    }

    fn typed(text: &str) -> Key {
        Key { sym: 0, text: text.into(), mods: Mods::default() }
    }

    fn type_all(s: &mut Settings, text: &str) {
        for c in text.chars() {
            s.key(&typed(&c.to_string()));
        }
    }

    fn alt(sym: u32) -> Key {
        Key { sym, text: String::new(), mods: Mods { alt: true, ..Mods::default() } }
    }

    /// Window pixel of the panel-local point `(x, y)`.
    fn win(s: &Settings, x: i32, y: i32) -> (f32, f32) {
        ((s.rect.0 + x) as f32 + 0.5, (s.rect.1 + y) as f32 + 0.5)
    }

    fn hit_at(s: &Settings, x: i32, y: i32) -> Hit {
        let (fx, fy) = win(s, x, y);
        s.hit(fx, fy)
    }

    fn click(s: &mut Settings, x: i32, y: i32) -> Effect {
        let (fx, fy) = win(s, x, y);
        s.press(fx, fy)
    }

    fn move_to(s: &mut Settings, x: i32, y: i32) -> Effect {
        let (fx, fy) = win(s, x, y);
        s.pointer(fx, fy)
    }

    /// Panel-local y of the middle of a row on the page being shown.
    fn mid(s: &Settings, row: Row) -> i32 {
        let lay = s.lay(row.page());
        let i = row.index();
        s.body_y(lay.top[i] + lay.h[i] / 2)
    }

    /// The panel on the input page with the chips on: the clock and the chips answer only then.
    fn on_chips_page() -> Settings {
        let mut s = open();
        s.set_page(Page::Input);
        s.draft.input.inline = true;
        s.orig.input.inline = true;
        s
    }

    // ----------------------------------------------------------------- geometry --

    #[test]
    fn the_box_is_372_px_or_62_percent_of_a_narrow_window() {
        assert_eq!(panel_width(1000), 372);
        assert_eq!(panel_width(900), 372);
        assert_eq!(panel_width(500), 310);
        assert_eq!(panel_width(300), 186);
    }

    #[test]
    fn the_reference_measures_come_out_of_the_layout() {
        let s = open();
        // 372 px with the hairline: the controls start at 141 and are 213 px wide, as measured in the reference.
        assert_eq!((s.ctrl_x(), s.ctrl_w()), (141, 213));
        let lay = s.lay(Page::Font);
        assert_eq!((lay.top[0], lay.h[0], lay.top[1]), (0, 40, 40), "rows are 40 px, no gap");
        assert_eq!(s.body_y(0), 92, "the first row is 4 px under the body's top, which is at 88");
        assert_eq!(s.tab_rect(Page::Font).1, 51, "the tabs' container starts at 48 and has 3 px of padding");
        assert_eq!(s.foot_top(), 570 - 59);
        assert_eq!(s.close_rect(), (1 + 371 - 8 - 28, 10, 28, 28));
        // A chip row is 10 + five lines of 34 and four gaps of 6 + 10 = 214 (the reference measured 214.9).
        assert_eq!(s.row_h(Row::Chips), 214);
        // The panel on the other side has its hairline on the right: everything shifts left by one pixel.
        let mut r = open();
        r.place((0, 0, 372, 570), true);
        assert_eq!((r.x0(), r.ctrl_x()), (0, 140));
    }

    #[test]
    fn a_radio_row_wraps_its_options_and_grows() {
        let s = open();
        // The mode row has two long labels: they do not fit one line of 213 px, so the row is two lines tall.
        let (x0, l0, lines) = s.radio_at(Row::Mode, 0);
        let (x1, l1, _) = s.radio_at(Row::Mode, 1);
        assert_eq!((x0, l0, x1, l1, lines), (0, 0, 0, 1, 2));
        assert_eq!(s.row_h(Row::Mode), 2 * RADIO_H + RADIO_LINE_GAP + 8);
        // The clock row's two short ones share a line and the row keeps its 40 px.
        assert_eq!(s.radio_at(Row::Clock, 1).1, 0);
        assert_eq!(s.row_h(Row::Clock), 40);
        // Whatever the language, every option lies inside the control column.
        for row in [Row::Mode, Row::Clock, Row::Cursor, Row::Tabs, Row::Language] {
            for n in 0..options(row).1 {
                let (x, _, _) = s.radio_at(row, n);
                assert!(x + s.radio_w(row, n) <= s.ctrl_w(), "{row:?} {n}");
            }
        }
    }

    #[test]
    fn the_tabs_fit_the_panel_and_every_row_fits_its_page() {
        let s = open();
        assert!(s.tabs_w() + 2 * EDGE <= PANEL_W, "{}", s.tabs_w());
        assert!(Page::ALL.iter().all(|&p| s.tab_rect(p).0 + s.tab_rect(p).2 <= s.x0() + EDGE + s.tabs_w()));
        // The font page and the windows page fit the default window without scrolling; the input page scrolls.
        let mut t = open();
        assert_eq!(t.max_scroll(), 0);
        t.page = Page::Windows;
        assert_eq!(t.max_scroll(), 0);
        t.page = Page::Input;
        assert!(t.max_scroll() > 0);
    }

    // --------------------------------------------------------------- hit testing --

    #[test]
    fn hit_testing_finds_each_control() {
        let s = open();
        let (cx, cw) = (s.ctrl_x(), s.ctrl_w());
        let (tx, tw) = s.track();
        assert!(matches!(hit_at(&s, tx + tw / 2, mid(&s, Row::Size)), Hit::Slider(Row::Size, t) if (t - 0.5).abs() < 0.02));
        assert!(matches!(hit_at(&s, tx, mid(&s, Row::Opacity)), Hit::Slider(Row::Opacity, t) if t == 0.0), "the left end of the track");
        assert_eq!(hit_at(&s, cx + 5, mid(&s, Row::Family)), Hit::Arrow(-1));
        assert_eq!(hit_at(&s, cx + cw - 5, mid(&s, Row::Family)), Hit::Arrow(1));
        assert_eq!(hit_at(&s, cx + 60, mid(&s, Row::Family)), Hit::Row(Row::Family), "the name box is the row");
        assert_eq!(hit_at(&s, cx + 10, mid(&s, Row::Bold)), Hit::Toggle(Row::Bold));
        assert_eq!(hit_at(&s, cx + 80, mid(&s, Row::Bold)), Hit::Row(Row::Bold), "beside the pill only the row is lit");
        let v = s.value_rect(mid(&s, Row::Size));
        assert_eq!(hit_at(&s, v.0 + 5, v.1 + 5), Hit::Value(Row::Size));
        let c = s.close_rect();
        assert_eq!(hit_at(&s, c.0 + 3, c.1 + 3), Hit::Close);
        let t = s.tab_rect(Page::Windows);
        assert_eq!(hit_at(&s, t.0 + 3, t.1 + 3), Hit::Tab(Page::Windows));
        let b = s.button_rect();
        assert_eq!(hit_at(&s, b.0 + 5, b.1 + 5), Hit::Defaults);
        assert_eq!(hit_at(&s, 30, 20), Hit::Panel, "the header's blank");
        assert_eq!(hit_at(&s, 3, 150), Hit::Panel, "the body's padding");
        assert_eq!(hit_at(&s, -5, 100), Hit::Outside);
        assert_eq!(hit_at(&s, PANEL_W + 5, 100), Hit::Outside);
    }

    #[test]
    fn a_box_elsewhere_in_the_window_is_found_where_it_is() {
        let mut s = open();
        s.place((600, 42, 372, 570), false);
        let y = mid(&s, Row::Bold);
        assert_eq!(s.hit(600.0 + s.ctrl_x() as f32 + 10.0, 42.0 + y as f32), Hit::Toggle(Row::Bold));
        assert_eq!(s.hit(10.0, 42.0 + y as f32), Hit::Outside);
        // The hairline is at the grid's side: left for a box at the right.
        assert_eq!(s.x0(), 1);
    }

    // ------------------------------------------------------------------ sliders --

    #[test]
    fn dragging_a_slider_sets_snapped_values_and_settles_on_release() {
        let mut s = open();
        let (tx, tw) = s.track();
        let y = mid(&s, Row::Size);
        // The knob's centre travels 7 px inside the ends: pressing there is the end of the range.
        assert_eq!(click(&mut s, tx + 7, y), Effect::Apply);
        assert_eq!(s.draft.font.size, 8.0);
        assert!(!s.settled(), "a drag is not a moment to write the file");
        assert_eq!(move_to(&mut s, tx + tw + 300, y), Effect::Apply, "past the end: clamped");
        assert_eq!(s.draft.font.size, 40.0);
        move_to(&mut s, tx + tw / 2, y + 200); // the pointer may leave the row while dragging
        assert_eq!(s.draft.font.size, 24.0);
        assert_eq!(s.release(), Effect::Settled);
        assert!(s.settled());
        let before = s.draft.clone();
        assert_ne!(move_to(&mut s, tx, y), Effect::Apply);
        assert_eq!(s.draft, before, "after the release the pointer only hovers");
        assert_eq!(s.release(), Effect::None);
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
        // The panel's width moves in steps of 4 inside the file's limits.
        d.set(Row::SideW, 221.0);
        assert_eq!(d.windows.side_w, 220);
        d.set(Row::SideW, 1000.0);
        assert_eq!(d.windows.side_w, 320);
        d.set(Row::SideW, 0.0);
        assert_eq!((d.windows.side_w, d.text(Row::SideW)), (160, "160".to_string()));
    }

    // ----------------------------------------------------------------- keyboard --

    #[test]
    fn the_keyboard_moves_between_rows_and_adjusts_values() {
        let mut s = open();
        assert_eq!(s.key(&key(sym::DOWN, false)), Effect::Redraw);
        assert_eq!(s.row(), Row::Size);
        assert_eq!(s.key(&key(sym::RIGHT, false)), Effect::Apply);
        assert_eq!(s.draft.font.size, 16.0);
        s.key(&key(sym::LEFT, true)); // shift: five steps
        assert_eq!(s.draft.font.size, 11.0);
        s.key(&key(sym::UP, false)); // the first row
        s.key(&key(sym::UP, false)); // the tab bar is the stop before it
        assert!(s.tabs_focus);
        s.key(&key(sym::UP, false)); // and round again to the last row of the page, the blur switch
        assert_eq!((s.row(), s.tabs_focus), (Row::Blur, false));
        s.key(&key(sym::RIGHT, false));
        assert!(s.draft.blur, "left and right flip a switch");
        s.key(&key(sym::UP, false));
        assert_eq!(s.row(), Row::Opacity);
        s.key(&key(sym::LEFT, false));
        assert_eq!(s.draft.opacity, 0.86);
        // Space flips the switch only on a switch's row.
        assert_eq!(s.key(&key(0x20, false)), Effect::None);
        s.sel = Row::Bold.index();
        assert_eq!(s.key(&key(0x20, false)), Effect::Apply);
        assert!(!s.draft.font.bold);
        s.key(&key(sym::RETURN, false));
        assert!(s.draft.font.bold, "Enter does what Space does on a switch");
        assert_eq!(s.key(&key(0x61, false)), Effect::None);
        // Escape closes the panel; on its own it saves nothing (the app writes when a change settles).
        assert_eq!(s.key(&key(sym::ESCAPE, false)), Effect::Close);
    }

    #[test]
    fn the_keyboard_skips_rows_that_do_nothing_now() {
        let mut s = open();
        s.set_page(Page::Input); // the shell's prompt: the clock and the chips are inert
        assert_eq!(s.row(), Row::Mode);
        s.key(&key(sym::DOWN, false));
        assert_eq!(s.row(), Row::Cursor);
        s.key(&key(sym::DOWN, false));
        assert_eq!(s.row(), Row::Autosuggest);
        s.key(&key(sym::DOWN, false));
        assert!(s.tabs_focus, "after the last row the tab bar, as Tab goes round in the reference");
        s.key(&key(sym::DOWN, false));
        assert_eq!((s.row(), s.tabs_focus), (Row::Mode, false), "and round again");
        s.key(&key(sym::UP, false));
        s.key(&key(sym::UP, false));
        assert_eq!(s.row(), Row::Autosuggest);
        s.draft.input.inline = true;
        s.key(&key(sym::UP, false));
        assert_eq!(s.row(), Row::Chips);
        // The windows page with the tabs at the top: the panel's rows are inert.
        s.set_page(Page::Windows);
        s.choose(Row::Tabs, 2);
        s.key(&key(sym::DOWN, false));
        assert_eq!(s.row(), Row::Language);
        s.key(&key(sym::DOWN, false));
        s.key(&key(sym::DOWN, false));
        assert_eq!(s.row(), Row::Tabs);
    }

    #[test]
    fn page_keys_and_ctrl_tab_switch_pages() {
        let mut s = open();
        s.key(&key(sym::PAGE_DOWN, false));
        assert_eq!(s.page, Page::Input);
        s.key(&key(sym::PAGE_DOWN, false));
        assert_eq!(s.page, Page::Windows);
        s.key(&key(sym::PAGE_DOWN, false));
        assert_eq!(s.page, Page::Font, "wraps");
        s.key(&key(sym::PAGE_UP, false));
        assert_eq!(s.page, Page::Windows);
        s.key(&Key { sym: sym::TAB, text: String::new(), mods: Mods { ctrl: true, ..Mods::default() } });
        assert_eq!(s.page, Page::Font);
        // Up and down stay inside the page: four rows on the windows page.
        s.set_page(Page::Windows);
        for _ in 0..4 {
            s.key(&key(sym::DOWN, false));
        }
        assert!(s.tabs_focus, "four rows, then the tab bar");
        s.key(&key(sym::DOWN, false));
        assert_eq!(s.row(), Row::Tabs);
    }

    #[test]
    fn the_tab_bar_has_the_keyboard_first_and_the_arrows_change_page_there() {
        let mut s = open();
        s.tabs_focus = true; // as `Settings::open` starts, the reference focuses the active tab
        assert_eq!(s.key(&key(sym::RIGHT, false)), Effect::Redraw);
        assert_eq!((s.page, s.tabs_focus), (Page::Input, true));
        s.key(&key(sym::RIGHT, false));
        s.key(&key(sym::RIGHT, false));
        assert_eq!(s.page, Page::Font, "wraps");
        s.key(&key(sym::LEFT, false));
        assert_eq!(s.page, Page::Windows);
        s.key(&key(sym::LEFT, false));
        s.key(&key(sym::LEFT, false));
        // Typing and space do nothing there; Escape closes; down goes to the first row, up from it comes back.
        assert_eq!((s.key(&typed("5")), s.key(&key(0x20, false))), (Effect::None, Effect::None));
        assert!(s.edit.is_none() && s.draft == open().draft);
        s.key(&key(sym::DOWN, false));
        assert_eq!((s.tabs_focus, s.row()), (false, Row::Family));
        s.key(&key(sym::UP, false));
        assert!(s.tabs_focus);
        assert_eq!(s.key(&key(sym::ESCAPE, false)), Effect::Close);
        // Clicking a tab puts the keyboard there; pressing on a row's control takes it off.
        let t = s.tab_rect(Page::Input);
        s.tabs_focus = false;
        click(&mut s, t.0 + 3, t.1 + 3);
        assert!(s.tabs_focus);
        let (cx, y) = (s.ctrl_x(), mid(&s, Row::Mode));
        click(&mut s, cx + 10, y);
        assert!(!s.tabs_focus);
        // The ring around the active tab is drawn only while the keyboard is on the bar.
        let (w, h) = (400usize, 600usize);
        let frame = |s: &Settings| {
            let mut fb = vec![0u32; w * h];
            s.draw(&mut fb, w, h, (0, 0));
            fb
        };
        s.tabs_focus = true;
        let on = frame(&s);
        s.focus = false;
        assert_ne!(on, frame(&s), "the keyboard is elsewhere: no ring");
        s.focus = true;
        s.tabs_focus = false;
        assert_ne!(on, frame(&s));
    }

    #[test]
    fn a_long_label_wraps_inside_its_column_and_the_row_grows() {
        let s = open();
        // The fallback font is 8 px a character: 112 px is 14 of them.
        let (l, n) = s.label_lines("Letter spacing");
        assert_eq!((n, l[0]), (1, "Letter spacing"));
        let (l, n) = s.label_lines("Espaciado de letras");
        assert_eq!((n, l[0], l[1]), (2, "Espaciado de", "letras"));
        let (l, n) = s.label_lines("Autosugerencias");
        assert_eq!((n, l[0]), (1, "Autosugerencias"), "a single word is never split");
        let (l, n) = s.label_lines("uno dos tres cuatro cinco seis siete ocho");
        assert_eq!(n, 3);
        assert!(l.iter().all(|x| !x.is_empty()) && l.join(" ") == "uno dos tres cuatro cinco seis siete ocho");
        // Two lines of 18.2 px and the row's 4 px above and below: the reference measured 44.4.
        assert_eq!((Settings::label_h(1), Settings::label_h(2), Settings::label_h(3)), (0, 44, 63));
    }

    // -------------------------------------------------------------- font picker --

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
        // The arrows of the picker do the same.
        let (cx, y) = (s.ctrl_x(), mid(&s, Row::Family));
        assert_eq!(click(&mut s, cx + 5, y), Effect::Apply);
        assert_eq!(s.family_label(), "Alpha Mono");
    }

    #[test]
    fn a_configured_font_the_scan_did_not_list_still_has_a_stop() {
        let cfg = FontCfg { family: Some("/home/me/fonts/Odd.ttf".into()), ..FontCfg::default() };
        assert_eq!(Settings::new(&cfg, 0.88, fams()).family_label(), "Odd.ttf");
        let cfg = FontCfg { family: Some("beta mono".into()), ..FontCfg::default() };
        assert_eq!(Settings::new(&cfg, 0.88, fams()).family_label(), "Beta Mono");
    }

    // ---------------------------------------------------------- what is written --

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
        let b = s.button_rect();
        assert_eq!(click(&mut s, b.0 + 3, b.1 + 3), Effect::Apply);
        assert_eq!(s.draft, Draft::defaults());
        assert_eq!(s.family_label(), "Default (JetBrains Mono)");
        assert!(s.changes().iter().any(|c| c.0 == "font.weight"), "reset values count as changes against the saved ones");
    }

    #[test]
    fn the_windows_and_input_settings_are_written_under_their_file_keys() {
        let mut s = open();
        s.draft.windows = WindowsCfg { tabs: TabsPos::Right, side: false, side_w: 260 };
        s.draft.input.autosuggest = false;
        s.draft.input.inline = true;
        s.draft.input.clock24 = true;
        s.draft.input.cursor = CursorKind::Bar;
        let c = s.changes();
        let get = |k: &str| c.iter().find(|x| x.0 == k).and_then(|x| x.1.clone());
        assert_eq!(get("windows.tabs").as_deref(), Some("\"right\""));
        assert_eq!(get("windows.side").as_deref(), Some("false"));
        assert_eq!(get("windows.side_width").as_deref(), Some("260"));
        assert_eq!(get("input.autosuggest").as_deref(), Some("false"));
        assert_eq!(get("input.mode").as_deref(), Some("\"chips\""));
        assert_eq!(get("input.clock").as_deref(), Some("\"24h\""));
        assert_eq!(get("input.cursor").as_deref(), Some("\"bar\""));
        // What `config::save` makes of it reads back as the same settings.
        let mut text = String::new();
        for (full, v) in &c {
            let (sec, key) = full.split_once('.').unwrap();
            text = config::set_key(&text, sec, key, v.as_deref());
        }
        let mut w = Vec::new();
        let back = config::parse(&text, &mut w);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((back.windows, back.input.autosuggest, back.input.cursor), (s.draft.windows.clone(), false, CursorKind::Bar));
        // Alt+B outside the panel moves the switch without making it something to save.
        let mut t = open();
        t.follow_windows(&WindowsCfg { side: false, ..WindowsCfg::default() });
        assert!(!t.draft.windows.side && t.changes().is_empty());
    }

    #[test]
    fn the_defaults_put_the_windows_and_the_input_back_too() {
        let mut s = open().with_windows(&WindowsCfg { tabs: TabsPos::Top, side: false, side_w: 300 }).with_input(&InputCfg { inline: true, chips: vec![Chip::Dir], clock24: true, autosuggest: false, ..Default::default() });
        s.defaults();
        assert_eq!(s.draft.windows, WindowsCfg::default());
        assert_eq!(s.draft.input, InputCfg::default());
        assert!(s.changes().iter().any(|c| c.0 == "windows.tabs"));
    }

    // ------------------------------------------------------------------ painting --

    #[test]
    fn the_panel_paints_only_inside_its_box_and_premultiplied() {
        let (w, h) = (700, 700);
        let glass = 0x20101010u32;
        let mut fb = vec![glass; w * h];
        let mut s = open();
        s.place((300, 60, 372, 570), false);
        s.draw(&mut fb, w, h, (10, 8));
        let (x0, y0) = (310, 68);
        let mut inside_changed = 0;
        for y in 0..h {
            for x in 0..w {
                let p = fb[y * w + x];
                if x >= x0 && x < x0 + 372 && y >= y0 && y < y0 + 570 {
                    inside_changed += (p != glass) as usize;
                } else {
                    assert_eq!(p, glass, "({x}, {y}) is outside the box");
                }
            }
        }
        assert!(inside_changed > 5000, "{inside_changed}");
        // Premultiplied: no channel above alpha, anywhere.
        for &p in &fb {
            assert!((p >> 16 & 255) <= (p >> 24) + 1 && (p >> 8 & 255) <= (p >> 24) + 1 && (p & 255) <= (p >> 24) + 1);
        }
    }

    #[test]
    fn what_is_scrolled_out_of_the_body_never_draws_over_the_header_or_the_footer() {
        let frame = |s: &Settings| {
            let (w, h) = (400, 600);
            let mut fb = vec![0u32; w * h];
            s.draw(&mut fb, w, h, (0, 0));
            fb
        };
        let mut s = on_chips_page();
        s.focus = false;
        let top = frame(&s);
        assert_eq!(s.scroll_by(10_000), Effect::Redraw);
        assert_eq!(s.scroll, s.max_scroll());
        let end = frame(&s);
        assert_ne!(top, end, "the rows moved");
        let w = 400;
        let rows = |fb: &[u32], a: usize, b: usize| fb[a * w..b * w].to_vec();
        // The header and tabs (above 88) and the footer (below it) are the same at both scroll positions.
        assert_eq!(rows(&top, 0, BODY_Y as usize), rows(&end, 0, BODY_Y as usize));
        let ft = s.foot_top() as usize;
        assert_eq!(rows(&top, ft, 570), rows(&end, ft, 570));
    }

    #[test]
    fn the_input_page_draws_and_the_chips_show_their_colours() {
        let mut s = on_chips_page();
        s.toggle_chip(Chip::Kube);
        let (w, h) = (400usize, 600usize);
        let mut fb = vec![0u32; w * h];
        s.draw(&mut fb, w, h, (0, 0));
        // An enabled chip's cell carries its colour: the time chip's red outweighs its green and blue.
        let slot = |chip: Chip| s.slot_rect(s.slots().iter().position(|&c| c == chip).unwrap());
        let probe = |fb: &[u32], chip: Chip| {
            let (x, y, cw, ch) = slot(chip);
            let px = fb[(s.body_y(y + ch / 2)) as usize * w + (x + cw - 8) as usize];
            ((px >> 16 & 255) as i32, (px >> 8 & 255) as i32, (px & 255) as i32)
        };
        let (r, g, b) = probe(&fb, Chip::Time);
        assert!(r > g + 8 && r > b + 8, "{r} {g} {b}");
        // The disabled one is grey.
        let (r, g, b) = probe(&fb, Chip::Kube);
        assert!((r - g).abs() < 6 && (g - b).abs() < 6, "{r} {g} {b}");
    }

    #[test]
    fn the_scrollbar_shows_only_when_the_body_overflows() {
        let (w, h) = (400usize, 600usize);
        let col = |s: &Settings| {
            let mut fb = vec![0u32; w * h];
            s.draw(&mut fb, w, h, (0, 0));
            (BODY_Y..s.foot_top()).filter(|&y| fb[y as usize * w + (s.x0() + s.cw() - 5) as usize] != 0).count()
        };
        let mut s = open();
        s.focus = false;
        assert_eq!(col(&s), 0);
        s.set_page(Page::Input);
        assert!(col(&s) > 20, "{}", col(&s));
    }

    // ------------------------------------------------------------- typed numbers --

    #[test]
    fn clicking_a_number_opens_its_box_with_the_value_selected() {
        let mut s = open();
        let v = s.value_rect(mid(&s, Row::Size));
        assert!(matches!(hit_at(&s, v.0 + 10, v.1 + 10), Hit::Value(Row::Size)));
        assert_eq!(click(&mut s, v.0 + 10, v.1 + 10), Effect::Redraw);
        assert_eq!(s.edit, Some(Edit { row: Row::Size, text: "15".into(), selected: true }));
        assert_eq!(s.sel, Row::Size.index());
        assert!(!s.settled(), "while a number is typed nothing is written");
        // The first key replaces the selected number; later ones add to it.
        type_all(&mut s, "2");
        assert_eq!(s.edit.as_ref().unwrap().text, "2");
        type_all(&mut s, "4");
        assert_eq!(s.key(&key(sym::RETURN, false)), Effect::Apply, "Enter commits the box");
        assert_eq!(s.draft.font.size, 24.0);
        assert!(s.edit.is_none() && s.settled());
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
        assert_eq!(s.draft.text(Row::Size), "96");
        // The panel's width is a number too, and the file's limits (160 to 320) are its own.
        s.page = Page::Windows;
        s.sel = Row::SideW.index();
        type_all(&mut s, "250");
        s.key(&key(sym::RETURN, false));
        assert_eq!(s.draft.windows.side_w, 250);
        s.sel = Row::SideW.index();
        type_all(&mut s, "999");
        s.key(&key(sym::RETURN, false));
        assert_eq!(s.draft.windows.side_w, 320);
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
    fn escape_ends_the_box_not_the_panel_and_other_ways_out_keep_the_number() {
        let mut s = open();
        s.sel = Row::Weight.index();
        type_all(&mut s, "700");
        assert_eq!(s.key(&key(sym::ESCAPE, false)), Effect::Redraw);
        assert!(s.edit.is_none());
        assert_eq!(s.draft.font.weight, 400, "Escape discards what was typed");
        assert_eq!(s.key(&key(sym::ESCAPE, false)), Effect::Close, "with no box open it closes the panel");
        // Tab commits and moves on; so does a click anywhere; so does closing (`commit`).
        type_all(&mut s, "700");
        assert_eq!(s.key(&key(sym::TAB, false)), Effect::Apply);
        assert_eq!((s.draft.font.weight, s.row()), (700, Row::Bold));
        s.sel = Row::Line.index();
        type_all(&mut s, "1.5");
        assert_eq!(click(&mut s, 30, 20), Effect::Apply, "a click elsewhere keeps the number");
        assert_eq!(s.draft.font.line_height, 1.5);
        s.sel = Row::Size.index();
        type_all(&mut s, "20");
        assert_eq!(s.commit().font.size, 20.0);
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
        let v = s.value_rect(mid(&s, Row::Size));
        click(&mut s, v.0 + 10, v.1 + 10);
        type_all(&mut s, "33");
        click(&mut s, v.0 + 10, v.1 + 10);
        assert_eq!(s.draft.font.size, 33.0);
        assert!(s.edit.is_none());
    }

    #[test]
    fn a_box_being_typed_is_painted() {
        let (w, h) = (400usize, 600usize);
        let frame = |s: &Settings| {
            let mut fb = vec![0u32; w * h];
            s.draw(&mut fb, w, h, (0, 0));
            fb
        };
        let mut s = open();
        let plain = frame(&s);
        s.sel = Row::Size.index();
        type_all(&mut s, "2");
        assert_ne!(frame(&s), plain);
    }

    // --------------------------------------------------------------------- pages --

    #[test]
    fn the_tabs_switch_pages_and_each_page_has_its_own_rows() {
        let mut s = open();
        let t = s.tab_rect(Page::Input);
        assert_eq!(click(&mut s, t.0 + 3, t.1 + 3), Effect::Redraw);
        assert_eq!((s.page, s.row(), s.scroll), (Page::Input, Row::Mode, 0));
        // The font rows are not under the pointer any more: a click where the opacity slider was does nothing.
        let (tx, tw) = s.track();
        let before = s.draft.clone();
        click(&mut s, tx + tw / 2, 92 + 6 * 40 + 20);
        assert_eq!(s.draft, before);
        let t = s.tab_rect(Page::Windows);
        click(&mut s, t.0 + 3, t.1 + 3);
        assert_eq!((s.page, s.row()), (Page::Windows, Row::Tabs));
        let t = s.tab_rect(Page::Font);
        click(&mut s, t.0 + 3, t.1 + 3);
        assert_eq!((s.page, s.row()), (Page::Font, Row::Family));
    }

    #[test]
    fn the_tab_marker_slides_to_the_page_and_back_to_rest() {
        let mut s = open();
        let (x, w) = s.marker();
        assert_eq!((x, w), (s.tab_rect(Page::Font).0 as f32, s.tab_rect(Page::Font).2 as f32));
        s.set_page(Page::Windows);
        assert!(s.animating());
        let (x1, w1) = s.marker();
        assert_eq!((x1, w1), (x, w), "it starts where it was");
        let t = Instant::now();
        s.step(t);
        s.step(t + Duration::from_millis(110));
        let (xm, wm) = s.marker();
        let to = s.tab_rect(Page::Windows);
        assert!(xm > x && xm < to.0 as f32, "{xm}");
        assert!(wm > w.min(to.2 as f32) - 0.5 && wm < w.max(to.2 as f32) + 0.5);
        assert!(!s.step(t + Duration::from_millis(600)));
        assert_eq!(s.marker(), (to.0 as f32, to.2 as f32));
    }

    // ------------------------------------------------------------------- choices --

    #[test]
    fn the_choices_set_the_mode_the_clock_the_cursor_and_the_language() {
        let mut s = open();
        s.set_page(Page::Input);
        let opt = |s: &Settings, row: Row, n: usize| {
            let (x, line, _) = s.radio_at(row, n);
            let (top, h) = {
                let lay = s.lay(Page::Input);
                (s.body_y(lay.top[row.index()]), lay.h[row.index()])
            };
            let lines = s.radio_at(row, 0).2;
            let block = lines * RADIO_H + (lines - 1) * RADIO_LINE_GAP;
            (s.ctrl_x() + x + 20, top + (h - block) / 2 + line * (RADIO_H + RADIO_LINE_GAP) + RADIO_H / 2)
        };
        // Without the chips the clock is inert.
        let (x, y) = opt(&s, Row::Clock, 1);
        assert_eq!(click(&mut s, x, y), Effect::None);
        assert!(!s.draft.input.clock24);
        let (x, y) = opt(&s, Row::Mode, 1);
        assert_eq!(hit_at(&s, x, y), Hit::Choice(Row::Mode, 1));
        assert_eq!(click(&mut s, x, y), Effect::Apply);
        assert!(s.draft.input.inline);
        let (x, y) = opt(&s, Row::Clock, 1);
        assert_eq!(click(&mut s, x, y), Effect::Apply);
        assert!(s.draft.input.clock24);
        let (x, y) = opt(&s, Row::Mode, 0);
        click(&mut s, x, y);
        assert!(!s.draft.input.inline);
        // The cursor row has three options, on two lines.
        let (x, y) = opt(&s, Row::Cursor, 2);
        click(&mut s, x, y);
        assert_eq!(s.draft.input.cursor, CursorKind::Underline);
        s.sel = Row::Cursor.index();
        s.key(&key(sym::LEFT, false));
        assert_eq!(s.draft.input.cursor, CursorKind::Bar);
        s.key(&key(sym::LEFT, false));
        s.key(&key(sym::LEFT, false));
        assert_eq!(s.draft.input.cursor, CursorKind::Underline, "the radios wrap, as the reference's do");
        s.key(&Key { sym: 0x20, text: " ".into(), mods: Mods::default() });
        assert_eq!(s.draft.input.cursor, CursorKind::Block, "space goes to the next one");
        s.draft.input.cursor = CursorKind::Underline;
        assert_eq!(s.changes().last().map(|(k, v)| (*k, v.clone())), Some(("input.cursor", Some("\"underline\"".to_string()))));
        // Autosuggest is a switch, on by default.
        assert!(s.draft.input.autosuggest);
        s.sel = Row::Autosuggest.index();
        s.key(&key(0x20, false));
        assert!(!s.draft.input.autosuggest);
        assert!(s.changes().iter().any(|(k, v)| *k == "input.autosuggest" && v.as_deref() == Some("false")));
    }

    #[test]
    fn the_windows_page_sets_where_the_list_lives_and_whether_the_panel_shows() {
        let mut s = open();
        s.set_page(Page::Windows);
        s.draft.windows.side = false;
        s.choose(Row::Tabs, 1);
        assert_eq!((s.draft.windows.tabs, s.draft.windows.side), (TabsPos::Right, true), "picking a side shows the panel");
        s.choose(Row::Tabs, 2);
        assert_eq!(s.draft.windows.tabs, TabsPos::Top);
        assert!(s.draft.windows.side, "top keeps what it was");
        assert!(s.inert(Row::Side) && s.inert(Row::SideW), "with the tabs at the top there is no panel to configure");
        // An inert row answers nothing to the pointer.
        let (tx, tw) = s.track();
        assert_eq!(hit_at(&s, tx + tw / 2, mid(&s, Row::SideW)), Hit::Panel);
        s.choose(Row::Tabs, 0);
        assert!(!s.inert(Row::SideW));
        // The switch and the width answer.
        let (cx, y) = (s.ctrl_x(), mid(&s, Row::Side));
        assert_eq!(click(&mut s, cx + 10, y), Effect::Apply);
        assert!(!s.draft.windows.side);
        let (tx, tw) = s.track();
        let y = mid(&s, Row::SideW);
        click(&mut s, tx + tw - 7, y);
        assert_eq!(s.draft.windows.side_w, 320);
        s.release();
        // The language is on this page too: Left/Right pick it.
        s.sel = Row::Language.index();
        s.key(&key(sym::RIGHT, false));
        assert_eq!(s.draft.lang, i18n::Setting::En);
        assert!(s.changes().iter().any(|(k, v)| *k == "ui.language" && v.as_deref() == Some("\"en\"")));
        s.key(&key(sym::LEFT, false));
        assert!(s.changes().iter().all(|(k, _)| *k != "ui.language"));
    }

    #[test]
    fn blur_is_a_switch_apart_from_opacity() {
        let mut s = Settings::new(&FontCfg::default(), 0.88, vec![]).with_blur(false, true);
        assert!(s.changes().is_empty());
        s.sel = Row::Blur.index();
        assert_eq!(s.key(&key(0x20, false)), Effect::Apply);
        assert!(s.draft.blur);
        assert_eq!(s.draft.opacity, 0.88);
        assert_eq!(s.changes(), vec![("window.blur", Some("true".to_string()))]);
        let (cx, y) = (s.ctrl_x(), mid(&s, Row::Blur));
        assert_eq!(click(&mut s, cx + 10, y), Effect::Apply);
        assert!(!s.draft.blur);
        assert!(s.changes().is_empty(), "back to what it opened with");
        s.key(&key(sym::RIGHT, false));
        assert!(s.draft.blur);
        s.sel = Row::Opacity.index();
        s.key(&key(sym::LEFT, false));
        assert!(s.draft.blur, "opacity does not touch blur");
        s.defaults();
        assert!(!s.draft.blur);
        // A compositor without blur is told so on the row.
        let (w, h) = (400usize, 600usize);
        let frame = |s: &Settings| {
            let mut fb = vec![0u32; w * h];
            s.draw(&mut fb, w, h, (0, 0));
            fb
        };
        let yes = Settings::new(&FontCfg::default(), 0.88, vec![]).with_blur(true, true);
        let no = Settings::new(&FontCfg::default(), 0.88, vec![]).with_blur(true, false);
        assert_ne!(frame(&yes), frame(&no));
    }

    // --------------------------------------------------------------------- chips --

    /// Where chip `c` is in the grid now, as panel-local x and y of its middle.
    fn centre(s: &Settings, c: Chip) -> (i32, i32) {
        let (x, y, w, h) = s.slot_rect(s.slots().iter().position(|&x| x == c).unwrap());
        (x + w / 2, s.body_y(y + h / 2))
    }

    /// Presses on a chip, moves the pointer to panel position `to` in steps, and lets go.
    fn drag(s: &mut Settings, from: (i32, i32), to: (i32, i32)) {
        click(s, from.0, from.1);
        for step in 1..=8 {
            move_to(s, from.0 + (to.0 - from.0) * step / 8, from.1 + (to.1 - from.1) * step / 8);
        }
        s.release();
    }

    #[test]
    fn the_chips_are_a_grid_of_two_columns_and_each_is_found_where_it_is_drawn() {
        let s = on_chips_page();
        for (i, c) in s.slots().into_iter().enumerate() {
            let (x, y, w, h) = s.slot_rect(i);
            assert!(x >= s.ctrl_x() && x + w <= s.ctrl_x() + s.ctrl_w(), "{c:?}");
            assert_eq!((x - s.ctrl_x() > 0), i % 2 == 1);
            assert_eq!(hit_at(&s, x + w / 2, s.body_y(y + h / 2)), Hit::Chip(c));
        }
        // The grid is inert with the shell's own prompt.
        let mut t = open();
        t.set_page(Page::Input);
        let (x, y, w, h) = t.slot_rect(0);
        assert_eq!(hit_at(&t, x + w / 2, t.body_y(y + h / 2)), Hit::Panel);
    }

    #[test]
    fn a_click_on_a_chip_switches_it_off_and_back_at_the_end() {
        let mut s = on_chips_page();
        assert_eq!(s.draft.input.chips, Chip::ALL);
        let c = centre(&s, Chip::Dir);
        // Pressing alone changes nothing; letting go without having moved is the click.
        assert_eq!(click(&mut s, c.0, c.1), Effect::Redraw);
        assert_eq!(s.draft.input.chips, Chip::ALL);
        assert_eq!(s.release(), Effect::Apply);
        assert!(s.settled(), "a click is over once it is let go");
        assert!(!s.draft.input.chips.contains(&Chip::Dir));
        // It is listed with the chips that are off, after the ones that are on.
        assert_eq!(*s.slots().last().unwrap(), Chip::Dir);
        assert_eq!(s.chip_sel, Chip::COUNT - 1, "the cursor follows the chip");
        // Back on: at the end of the bar, not in its old place.
        let c = centre(&s, Chip::Dir);
        click(&mut s, c.0, c.1);
        s.release();
        assert_eq!(s.draft.input.chips.last(), Some(&Chip::Dir));
    }

    #[test]
    fn dragging_a_chip_moves_it_to_the_slot_under_the_pointer() {
        let mut s = on_chips_page();
        let (from, to) = (centre(&s, Chip::Time), centre(&s, Chip::Host));
        click(&mut s, from.0, from.1);
        assert!(s.settled(), "a press that has not moved is not a drag yet");
        for step in 1..=8 {
            move_to(&mut s, from.0 + (to.0 - from.0) * step / 8, from.1 + (to.1 - from.1) * step / 8);
        }
        assert!(!s.settled(), "while it is held nothing is written");
        assert_eq!(s.release(), Effect::Settled);
        // Time went to Host's place (slot 4) and the ones between moved up one.
        assert_eq!(s.draft.input.chips, [Chip::User, Chip::Env, Chip::Pyenv, Chip::Host, Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes, Chip::Kube]);
        assert_eq!(s.chip_sel, 4);
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
        click(&mut s, c.0, c.1);
        assert_eq!(move_to(&mut s, c.0 + 2, c.1 + 1), Effect::None, "inside the slop nothing moves");
        s.release();
        assert!(!s.draft.input.chips.contains(&Chip::Env));
        // Two chips off, so the last slots of the grid are the "off" part. Dragging far to the right and below
        // the grid lands on the last one: the chip goes off.
        let mut s = on_chips_page();
        for c in [Chip::Kube, Chip::Changes] {
            let at = centre(&s, c);
            click(&mut s, at.0, at.1);
            s.release();
        }
        let c = centre(&s, Chip::Time);
        drag(&mut s, c, (PANEL_W + 40, 520));
        assert!(!s.draft.input.chips.contains(&Chip::Time));
        assert_eq!(s.draft.input.chips.len(), Chip::COUNT - 3);
        // Dragging a chip that is off into the bar turns it on, in the slot it is dropped on.
        let slot = s.slots().iter().position(|&x| x == Chip::Dir).unwrap();
        let (c, to) = (centre(&s, Chip::Time), centre(&s, Chip::Dir));
        drag(&mut s, c, to);
        assert_eq!(s.draft.input.chips.iter().position(|&x| x == Chip::Time), Some(slot), "{:?}", s.draft.input.chips);
        assert!(s.chip_drag.is_none(), "the press that started a drag is over");
    }

    #[test]
    fn alt_and_the_arrows_move_the_chip_the_cursor_is_on() {
        let mut s = on_chips_page();
        s.sel = Row::Chips.index();
        s.key(&key(sym::RIGHT, false)); // on User
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
    fn the_chip_cursor_moves_with_the_arrows_and_space_flips_the_chip() {
        let mut s = on_chips_page();
        s.sel = Row::Chips.index();
        s.key(&key(sym::RIGHT, false));
        s.key(&key(sym::RIGHT, false));
        assert_eq!(s.chip_sel, 2);
        assert_eq!(s.key(&Key { sym: 0x20, text: " ".into(), mods: Mods::default() }), Effect::Apply);
        assert!(!s.draft.input.chips.contains(&Chip::Env));
        assert_eq!(s.chip_sel, Chip::COUNT - 1, "the chip went to the off part of the grid, the cursor with it");
        s.key(&key(sym::RIGHT, false));
        assert_eq!(s.chip_sel, 0, "wraps");
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
        let t = Settings::new(&FontCfg::default(), 0.88, fams()).with_input(&InputCfg { chips: order.clone(), ..InputCfg::default() });
        assert_eq!(&t.slots()[..Chip::COUNT], &order[..], "reopened with that list, the grid shows it");
    }

    // ----------------------------------------------------------- autosave contract --

    #[test]
    fn a_change_asks_to_be_saved_only_when_nothing_is_in_the_middle_of_being_done() {
        let mut s = open();
        // A switch: changed and settled at once.
        let (cx, y) = (s.ctrl_x(), mid(&s, Row::Bold));
        assert_eq!(click(&mut s, cx + 10, y), Effect::Apply);
        assert!(s.settled());
        // A slider held: changes apply but the file waits; the release says it is time.
        let (tx, tw) = s.track();
        let y = mid(&s, Row::Weight);
        click(&mut s, tx + tw / 2, y);
        assert!(!s.settled());
        move_to(&mut s, tx + tw / 4, y);
        assert!(!s.settled());
        assert_eq!(s.release(), Effect::Settled);
        assert!(s.settled());
        // The keyboard changes a value one step at a time: each is settled (the app debounces them).
        s.sel = Row::Size.index();
        assert_eq!(s.key(&key(sym::RIGHT, false)), Effect::Apply);
        assert!(s.settled());
        // A number typed is in the middle of something until it is committed.
        type_all(&mut s, "2");
        assert!(!s.settled());
        s.key(&key(sym::RETURN, false));
        assert!(s.settled());
    }

    #[test]
    fn the_wheel_scrolls_the_body_and_the_keyboard_brings_the_selected_row_into_view() {
        let mut s = on_chips_page();
        assert_eq!(s.scroll_by(-30), Effect::None, "already at the top");
        assert_eq!(s.scroll_by(WHEEL_PX), Effect::Redraw);
        assert_eq!(s.scroll, WHEEL_PX);
        assert_eq!(s.scroll_by(100_000), Effect::Redraw);
        assert_eq!(s.scroll, s.max_scroll());
        assert_eq!(s.scroll_by(1), Effect::None, "and at the bottom");
        // The first row is in view again when the keyboard goes there; so is the chips row coming up from below.
        s.sel = Row::Autosuggest.index();
        s.key(&key(sym::DOWN, false));
        assert_eq!((s.tabs_focus, s.scroll), (true, 0), "the tab bar is at the top");
        s.key(&key(sym::DOWN, false));
        assert_eq!((s.row(), s.scroll), (Row::Mode, 0));
        s.key(&key(sym::UP, false));
        s.key(&key(sym::UP, false));
        assert_eq!(s.row(), Row::Autosuggest);
        assert!(s.scroll > 0);
        // The same point on the screen shows another chip once the body has scrolled.
        s.scroll = 0;
        let c = centre(&s, Chip::Time);
        s.scroll = s.max_scroll();
        assert_ne!(hit_at(&s, c.0, c.1), Hit::Chip(Chip::Time));
        // Changing page starts at the top again; a smaller box clamps the scroll.
        s.set_page(Page::Font);
        assert_eq!(s.scroll, 0);
        s.set_page(Page::Input);
        s.scroll_by(100_000);
        s.place((0, 0, 372, 900), false);
        assert_eq!(s.scroll, s.max_scroll());
    }

    #[test]
    fn a_note_is_wrapped_and_its_words_are_told_apart() {
        let w = |s: &str, _: Ink| s.chars().count() as i32 * 10;
        let mut seen = Vec::new();
        let lines = flow("aaaa bbbb cccc", 90, &w, |t, _, x, l| seen.push((t.to_string(), x, l)));
        assert_eq!(lines, 2);
        assert_eq!(seen, [("aaaa".to_string(), 0, 0), ("bbbb".to_string(), 50, 0), ("cccc".to_string(), 0, 1)]);
        // A hard break starts a line even when the next word would fit.
        assert_eq!(flow("a\nb\n\nc", 500, &w, |_, _, _, _| {}), 3, "a blank line is not kept, only breaks before words");
        let t = tokens("Press [Alt] + [←] / `x y`.");
        let got: Vec<(&str, Ink, bool)> = t.iter().map(|t| (t.text, t.ink, t.space)).collect();
        assert_eq!(
            got,
            [("Press", Ink::Text, false), ("Alt", Ink::Key, true), ("+", Ink::Text, true), ("←", Ink::Key, true), ("/", Ink::Text, true), ("x y", Ink::Code, true), (".", Ink::Text, false)]
        );
        // The real notes: wider box, fewer lines; they have the lines the strings say.
        let s = open();
        assert!(s.note_lines(Str::NoteWindows) >= 3 && s.note_lines(Str::NoteInput) >= 4);
        let mut wide = open();
        wide.place((0, 0, 700, 570), false);
        assert!(wide.note_lines(Str::NoteWindows) < s.note_lines(Str::NoteWindows));
        // Keycaps and code are wider than their text.
        assert!(s.ink_width("Alt", Ink::Key) > s.width("Alt", PX_KBD));
        assert!(s.ink_width("$", Ink::Code) > s.width("$", PX_CODE));
    }

    // ----------------------------------------------------------------- animations --

    #[test]
    fn the_panel_fades_in_and_a_hover_fades_between_looks() {
        let (w, h) = (400usize, 600usize);
        let frame = |s: &Settings| {
            let mut fb = vec![0u32; w * h];
            s.draw(&mut fb, w, h, (0, 0));
            fb
        };
        let mut s = open();
        assert!(!s.animating(), "a panel that is not asked to fade is just there");
        s.start_faded();
        assert!(s.animating());
        assert!(frame(&s).iter().all(|&p| p == 0), "fully faded out it draws nothing");
        let t0 = Instant::now();
        assert!(s.step(t0), "the first step only starts the clock");
        let half = frame(&s);
        assert!(s.step(t0 + Duration::from_millis(80)));
        assert_ne!(half, frame(&s));
        assert!(!s.step(t0 + Duration::from_millis(400)), "it ends, and then nothing asks for frames");
        assert_eq!(s.fade, 1.0);
        assert_eq!(frame(&s), frame(&open()), "faded in it is exactly the panel");

        // Moving onto the reset button starts a fade of 0.12 s from the old look to the new one.
        let b = s.button_rect();
        let rest = frame(&s);
        move_to(&mut s, b.0 + 5, b.1 + 5);
        assert!(s.animating());
        assert_eq!(s.lit(&|h| h == Hit::Defaults), 0.0);
        assert_eq!(frame(&s), rest, "the first frame is still the old look");
        let t1 = Instant::now();
        s.step(t1);
        s.step(t1 + Duration::from_millis(60));
        let mid = s.lit(&|h| h == Hit::Defaults);
        assert!(mid > 0.3 && mid < 0.7, "{mid}");
        assert!(!s.step(t1 + Duration::from_millis(300)));
        assert_eq!(s.lit(&|h| h == Hit::Defaults), 1.0);
        // Leaving fades it out the same way, and moving along a slider does not restart anything.
        move_to(&mut s, -5, -5);
        assert_eq!(s.lit(&|h| h == Hit::Defaults), 1.0, "still lit on the frame it is left");
        s.step(Instant::now());
        assert!(s.animating());
        let (tx, tw) = s.track();
        let y = mid_of(&s);
        move_to(&mut s, tx + 20, y);
        let t2 = Instant::now();
        s.step(t2);
        s.step(t2 + Duration::from_millis(500));
        assert!(!s.animating());
        move_to(&mut s, tx + tw - 20, y);
        assert!(!s.animating(), "the same slider: nothing to fade");
    }

    fn mid_of(s: &Settings) -> i32 {
        mid(s, Row::Size)
    }

    #[test]
    fn the_keyboard_is_lit_only_while_the_panel_has_it() {
        let (w, h) = (400usize, 600usize);
        let frame = |s: &Settings| {
            let mut fb = vec![0u32; w * h];
            s.draw(&mut fb, w, h, (0, 0));
            fb
        };
        let mut s = open();
        s.sel = 1;
        let on = frame(&s);
        s.focus = false;
        assert_ne!(frame(&s), on, "the selected row's light goes with the focus");
        // Pressing on the panel gives it back.
        let c = s.close_rect();
        click(&mut s, c.0 + 2, c.1 + 2);
        assert!(s.focus);
    }

    #[test]
    fn the_cross_and_escape_close_the_panel() {
        let mut s = open();
        let c = s.close_rect();
        assert_eq!(click(&mut s, c.0 + 4, c.1 + 4), Effect::Close);
        assert_eq!(s.key(&key(sym::ESCAPE, false)), Effect::Close);
    }
}
