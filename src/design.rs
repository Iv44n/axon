//! Frames for the design board (`design/board.html`, `scripts/design.sh`).
//!
//! Renders the real window, through `view.rs`, `soft.rs` and `chrome.rs`, in a fixed set of scenes
//! and writes each one as a PNG with alpha, so the board can put it over any backdrop and show
//! exactly what the compositor would. Also writes `frames.js`: the scene list and the design
//! tokens (sizes, palette) read from the code, so the board never restates a number by hand.
//!
//! `frames` is test-only and `#[ignore]`d: it writes files into the tree. The PNG encoder is a small
//! own one (per-row filter, then deflate with fixed Huffman codes and run matches), enough for UI
//! pictures. The same scenes drive `incremental_repaint_matches_a_full_one`, which runs always.
#![cfg(test)]

use alacritty_terminal::{
    Term,
    event::VoidListener,
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::Config,
    vte::ansi::Processor,
};

use crate::{
    bar,
    chrome::{self, Ui},
    config::{Chip, FontCfg, Smoothing, TabsPos, WindowsCfg},
    context::Context,
    history::History,
    i18n::{self, Lang},
    palette, panes, session,
    settings::{self, Settings},
    side::{self, Status},
    soft::{Dock, Slot, Soft, Target, UNFOCUSED},
    view::View,
};

mod group_scenes;
mod overlay_scenes;
mod overview_scenes;

/// Window size of every scene, in logical pixels (the frame adds the shadow margin around it).
const W: u32 = 900;
const H: u32 = 620;
const PX: f32 = 15.0;
const OPACITY: f32 = chrome::OPACITY;

const SHELL: &str = "\x1b[1;32muser@axon\x1b[0m:\x1b[1;34m~/lab/apps/axon\x1b[0m$ ls --color\r\n\
\x1b[1;34msrc\x1b[0m  \x1b[1;34mknowledge\x1b[0m  Cargo.toml  \x1b[32mscripts\x1b[0m  README.md  \x1b[31mbad.rs\x1b[0m\r\n\
\x1b[1;32muser@axon\x1b[0m:\x1b[1;34m~/lab/apps/axon\x1b[0m$ git diff --stat\r\n\
\x20src/chrome.rs | 120 \x1b[32m++++++++++\x1b[31m---\x1b[0m\r\n\
\x20src/soft.rs   |  42 \x1b[32m+++++\x1b[31m-\x1b[0m\r\n\
\x202 files changed, 141 insertions(+), 21 deletions(-)\r\n\
\x1b[1;32muser@axon\x1b[0m:\x1b[1;34m~/lab/apps/axon\x1b[0m$ cargo build --release\r\n\
\x1b[1;32m   Compiling\x1b[0m axon v0.1.0 (/home/user/lab/apps/axon)\r\n\
\x1b[1;32m    Finished\x1b[0m `release` profile [optimized] target(s) in 52.47s\r\n\
\x1b[1;32muser@axon\x1b[0m:\x1b[1;34m~/lab/apps/axon\x1b[0m$ ";

const COLORS: &str = "\x1b[1m normal \x1b[0m \x1b[30m██\x1b[31m██\x1b[32m██\x1b[33m██\x1b[34m██\x1b[35m██\x1b[36m██\x1b[37m██\x1b[0m  \
black red green yellow blue magenta cyan white\r\n\
\x1b[1m bright \x1b[0m \x1b[90m██\x1b[91m██\x1b[92m██\x1b[93m██\x1b[94m██\x1b[95m██\x1b[96m██\x1b[97m██\x1b[0m\r\n\
\r\n\
\x20\x1b[31merror\x1b[0m  \x1b[32mok\x1b[0m  \x1b[33mwarning\x1b[0m  \x1b[34minfo\x1b[0m  \x1b[35mtodo\x1b[0m  \x1b[36mhint\x1b[0m  \
\x1b[90mcomment\x1b[0m  \x1b[1mbold\x1b[0m  \x1b[2mdim\x1b[0m\r\n\
\x20\x1b[4munderline\x1b[0m  \x1b[9mstrike\x1b[0m  \x1b[7m inverse \x1b[0m  \x1b[41;97m bg red \x1b[0m \x1b[42;30m bg green \x1b[0m \
\x1b[44;97m bg blue \x1b[0m \x1b[100;97m bg gray \x1b[0m\r\n\
\r\n\
\x20┌─ box ─────────┐  █▓▒░  λ → ✓ ✗ • …\r\n\
\x20│ [\x1b[32m██████\x1b[90m░░░\x1b[0m]   │  64%  The quick brown fox jumps over the lazy dog\r\n\
\x20└───────────────┘  0O 1lI| {}[]() <= >= != == -> => :: 0x1f\r\n\
\r\n\
\x20selected text shows the selection colour\r\n\
\x1b[1;32muser@axon\x1b[0m:\x1b[1;34m~\x1b[0m$ ";

/// What the shell's hook makes of the demo session: every prompt becomes a reserved line above, numbered in the
/// colour of its concealed cell, and a bare `$` with the concealed space after it (`context::BASH_HOOK`); the
/// last one has `typed` after it.
fn with_hook(typed: &str) -> String {
    const PROMPT: &str = "\x1b[1;32muser@axon\x1b[0m:\x1b[1;34m~/lab/apps/axon\x1b[0m$ ";
    let mut out = String::new();
    for (n, part) in SHELL.split(PROMPT).enumerate() {
        if n > 0 {
            out.push_str(&format!("\x1b[8;38;2;0;0;{n}m \x1b[0m\r\n$\x1b[8m \x1b[0m"));
        }
        out.push_str(part);
    }
    out + typed
}

/// What pane `i` of a split window shows: the demo session in the first, shorter ones in the others, every
/// prompt marked the way the shell's hook marks it (so the chips go over them).
fn pane_text(i: usize) -> String {
    let p = |n: u32| format!("\x1b[8;38;2;0;0;{n}m \x1b[0m\r\n$\x1b[8m \x1b[0m");
    match i % 4 {
        0 => with_hook(""),
        1 => format!("{}ls\r\n\x1b[1;34msrc\x1b[0m  \x1b[1;34mknowledge\x1b[0m  Cargo.toml\r\n{}git status\r\nOn branch main\r\nnothing to commit, working tree clean\r\n{}", p(1), p(2), p(3)),
        2 => format!("{}cargo test\r\n\x1b[1;32m   Compiling\x1b[0m axon v0.1.0\r\ntest result: \x1b[32mok\x1b[0m. 211 passed\r\n{}\x1b[31mfailed\x1b[0m\r\n{}", p(1), p(2), p(3)),
        _ => format!("{}htop\r\n  PID USER   %CPU %MEM\r\n 4212 alice   3.1  0.9 axon\r\n{}", p(1), p(2)),
    }
}

/// The status icon of pane `i`'s row in the board's panel: a command running in the second, a failed one in the fourth.
fn pane_status(i: usize) -> Status {
    match i {
        1 => Status::Running,
        3 => Status::Failed,
        _ => Status::Idle,
    }
}

/// A window split into panes: how many, how they are arranged, which has the focus and the cross shown at a
/// pane's corner (the pane, and whether the pointer is on it).
#[derive(Clone, Copy)]
struct PaneScene {
    n: usize,
    kind: panes::Kind,
    focus: usize,
    cross: Option<(usize, bool)>,
}

/// The records of the demo session's prompts: the first command ran in 12 ms, the second failed after 1.5 s.
fn demo_history(ctx: &Context) -> History {
    let (mut h, t) = (History::default(), std::time::Instant::now());
    h.prompt(1, 0, t, ctx);
    h.start(t);
    h.prompt(2, 0, t + std::time::Duration::from_millis(12), ctx);
    h.start(t + std::time::Duration::from_millis(20));
    h.prompt(3, 127, t + std::time::Duration::from_millis(1520), ctx);
    h
}

/// The settings panel in a scene: the page shown, the row the keyboard is on, what is under the pointer, what
/// has been typed into a number's box, and so on. The panel's data (fonts installed, values) is fixed so the
/// board looks the same on every machine.
struct Prefs {
    page: settings::Page,
    sel: usize,
    hover: settings::Hit,
    typing: &'static str,
    /// A chip held mid-drag, and where the pointer is in the body's content.
    hold: Option<(Chip, (i32, i32))>,
    /// The blur switch: on, and whether the compositor offers blur.
    blur: (bool, bool),
    windows: WindowsCfg,
    /// Pixels the body is scrolled by the wheel.
    scroll: i32,
    /// The keyboard is on the panel (its selected row is lit), or on its tab bar (a ring round the active tab).
    focus: bool,
    tabs: bool,
}

fn prefs(page: settings::Page, sel: usize, hover: settings::Hit) -> Option<Prefs> {
    Some(Prefs { page, sel, hover, typing: "", hold: None, blur: (false, true), windows: WindowsCfg::default(), scroll: 0, focus: true, tabs: false })
}

/// Where the pointer is in a drag scene: over a window's row or tab, or a group's header or chip, at a fraction of
/// its box (across the panel, along the tabs), or past the last of the list.
#[derive(Clone, Copy)]
enum Over {
    Win(usize, f32),
    Group(usize, f32),
    Blank,
}

/// A window or group being dragged in the list, and where the pointer is: the drop's mark is what the real
/// `Layout::target` and `mark` say of that spot.
#[derive(Clone, Copy)]
struct DragScene {
    grab: crate::dnd::Grab,
    over: Over,
}

/// What the scenes of the list of windows add to the board's made-up session (arcton: three windows, servidor
/// and pruebas among them; "~" in none; docs: one): the tabs list the groups, the first group is folded, what
/// each window says about its commands, how far the spinners have turned, and a drag in progress.
#[derive(Clone, Copy)]
struct ListScene {
    grouped_tabs: bool,
    fold0: bool,
    status: [Status; 5],
    phase: f32,
    drag: Option<DragScene>,
    /// After painting the list at `phase`, turn the spinners to this phase the way the app does between full paints.
    respin: Option<f32>,
    /// The tabs at the top scrolled to start at this entry.
    first: usize,
}

const NO_LIST: ListScene = ListScene { grouped_tabs: false, fold0: false, status: [Status::Idle; 5], phase: 0.125, drag: None, respin: None, first: 0 };

/// The reference's session: servidor runs a command, pruebas failed one.
const BUSY: [Status; 5] = [Status::Idle, Status::Running, Status::Failed, Status::Idle, Status::Idle];

struct Scene {
    name: &'static str,
    label: &'static str,
    group: &'static str,
    ui: Ui,
    opacity: f32,
    font: FontCfg,
    /// The settings panel docked beside the grid.
    prefs: Option<Prefs>,
    title: &'static str,
    input: &'static str,
    /// The language of the interface texts.
    lang: Lang,
    /// Select `len` cells of this grid line, starting at column 1.
    select: Option<(i32, usize)>,
    /// The prompt chips: what they say, and which are listed. The last prompt of `input` is marked
    /// the way the shell's hook marks it (`prompt`).
    bar: Option<(Context, Vec<Chip>)>,
    /// The list of windows: where it is, and what the pointer is over. The windows are made up.
    side: Option<(crate::config::TabsPos, Option<crate::side::Part>)>,
    /// The second group of the panel is folded.
    folded: bool,
    /// The first window is split into panes (`side` lists them, or shows their number in its tab).
    panes: Option<PaneScene>,
    /// A window's name being edited in its row: the window, the text, and whether it is all selected (else the caret
    /// is at its end).
    edit: Option<(usize, &'static str, bool)>,
    list: ListScene,
}

/// What the bar says in the board's scenes: made-up values, the same on every machine.
fn sample_context() -> Context {
    let mut c = Context::default();
    for (chip, t) in [
        (Chip::Time, "03:48 pm"),
        (Chip::User, "alice"),
        (Chip::Env, "condaenv"),
        (Chip::Pyenv, "3.12.1"),
        (Chip::Host, "alice@axon"),
        (Chip::Dir, "~/lab/apps/axon"),
        (Chip::Branch, "feature/input-bar"),
        (Chip::Changes, "3 • +10 -2"),
        (Chip::Kube, "kube-context"),
    ] {
        c.text[chip as usize] = t.to_string();
    }
    c
}

/// The window title: the name (bold) and the place (dim), split at `|`.
const TITLE: &str = "arcton|~/lab/apps/arcton";

fn scenes() -> Vec<Scene> {
    let base = |name, label, group, ui| Scene {
        name,
        label,
        group,
        ui,
        opacity: OPACITY,
        font: FontCfg::sized(PX),
        prefs: None,
        title: TITLE,
        input: SHELL,
        select: None,
        bar: None,
        side: None,
        folded: false,
        panes: None,
        edit: None,
        list: NO_LIST,
        lang: Lang::En,
    };
    let bar = |chips: &[Chip]| Some((sample_context(), chips.to_vec()));
    let chips3 = || bar(&[Chip::Time, Chip::Dir, Chip::Branch]);
    let split = |n, kind, focus, cross| Some(PaneScene { n, kind, focus, cross });
    let side_ui = Ui { side: 2, ..Ui::IDLE };
    let hooked: &'static str = Box::leak(with_hook("").into_boxed_str());
    let typing: &'static str = Box::leak(with_hook("git commit -m \"wire the chips to the prompt\"").into_boxed_str());
    let bar_cursor: &'static str = Box::leak(format!("{SHELL}\x1b[6 q").into_boxed_str());
    let underline_cursor: &'static str = Box::leak(format!("{SHELL}\x1b[4 q").into_boxed_str());
    let font = |weight, letter_spacing, line_height| FontCfg { weight, letter_spacing, line_height, ..FontCfg::sized(PX) };
    // The settings panel is open: the gear in the title bar is lit.
    let open = Ui { prefs: true, ..Ui::IDLE };
    use settings::{Hit, Page, Row};
    vec![
        base("rest", "Focused, at rest", "window", Ui::IDLE),
        base("hover", "Pointer over the buttons (close)", "states", Ui { hover: 1, ..Ui::IDLE }),
        base("pressed", "Maximize held down", "states", Ui { hover: 3, down: true, ..Ui::IDLE }),
        base("unfocused", "Unfocused", "states", Ui { focused: false, ..Ui::IDLE }),
        base("maximized", "Maximized or tiled (square, no shadow)", "states", Ui { maximized: true, ..Ui::IDLE }),
        Scene { title: "colores|~", input: COLORS, select: Some((10, 13)), ..base("colors", "Palette, attributes and selection", "colour", Ui::IDLE) },
        Scene { opacity: 0.80, ..base("opacity-80", "Opacity 0.80", "opacity", Ui::IDLE) },
        Scene { opacity: 0.88, ..base("opacity-88", "Opacity 0.88 (default)", "opacity", Ui::IDLE) },
        Scene { opacity: 0.94, ..base("opacity-94", "Opacity 0.94", "opacity", Ui::IDLE) },
        base("gear-hover", "Pointer over the settings gear", "states", Ui { hover: 4, ..Ui::IDLE }),
        Scene { prefs: prefs(Page::Font, 0, Hit::Panel).map(|p| Prefs { tabs: true, ..p }), font: FontCfg { family: Some("JetBrains Mono".into()), ..FontCfg::sized(PX) }, ..base("prefs", "Settings panel docked beside the grid: Font page, the keyboard on the active tab (as it opens)", "settings", open) },
        Scene { prefs: prefs(Page::Font, 0, Hit::Panel), ..base("prefs-row", "The keyboard on the first row", "settings", open) },
        Scene { prefs: prefs(Page::Font, 2, Hit::Slider(Row::Weight, 0.4)), ..base("prefs-slider", "Dragging the weight slider", "settings", open) },
        Scene { prefs: prefs(Page::Font, 4, Hit::Panel).map(|p| Prefs { typing: "-1.5", ..p }), ..base("prefs-typing", "Typing a number into a box", "settings", open) },
        Scene { prefs: prefs(Page::Font, 6, Hit::Defaults), font: font(500, 1.0, 1.0), opacity: 0.80, ..base("prefs-reset", "Pointer over the reset button, other values changed", "settings", open) },
        Scene { bar: bar(&Chip::ALL), input: hooked, ..base("chips", "Every chip: the longest are cut, then the last dropped", "prompt chips", Ui::IDLE) },
        Scene { bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes]), input: typing, ..base("chips-few", "Four chips, a command being typed", "prompt chips", Ui::IDLE) },
        Scene { bar: bar(&Chip::ALL), input: hooked, font: FontCfg::sized(20.0), ..base("chips-large", "At 20 px", "prompt chips", Ui::IDLE) },
        Scene { bar: bar(&[Chip::Dir, Chip::Branch]), input: typing, opacity: 0.80, ..base("chips-80", "Opacity 0.80", "prompt chips", Ui::IDLE) },
        Scene { ..base("chips-off", "Chips off: the prompt as the shell draws it", "prompt chips", Ui::IDLE) },
        Scene { input: bar_cursor, ..base("cursor-bar", "Cursor as a bar (DECSCUSR 6, or [input] cursor = \"bar\")", "window", Ui::IDLE) },
        Scene { input: underline_cursor, ..base("cursor-underline", "Cursor as an underline", "window", Ui::IDLE) },
        base("no-status", "Status bar off ([window] status_bar = false)", "window", Ui { status: false, ..Ui::IDLE }),
        Scene { prefs: prefs(Page::Font, 7, Hit::Toggle(Row::Blur)).map(|p| Prefs { blur: (true, false), ..p }), opacity: 0.80, ..base("prefs-blur", "Blur on, but the compositor (GNOME) offers none", "settings", open) },
        Scene { prefs: prefs(Page::Input, 0, Hit::Tab(Page::Input)), ..base("prefs-input", "Input page, shell prompt: the clock and the chips are inert", "settings", open) },
        Scene {
            prefs: prefs(Page::Input, 3, Hit::Chip(Chip::Dir)),
            bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes]),
            input: hooked,
            ..base("prefs-chips", "Input page, chips on: four chips, pointer over dir, the terminal beside it narrower", "settings", open)
        },
        Scene {
            prefs: prefs(Page::Input, 4, Hit::Panel).map(|p| Prefs { scroll: 999, ..p }),
            bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes]),
            input: hooked,
            ..base("prefs-note", "Input page scrolled to its end: the note with its keycaps, the scrollbar", "settings", open)
        },
        Scene { prefs: prefs(Page::Windows, 0, Hit::Choice(Row::Tabs, 1)), ..base("prefs-windows", "Windows page: position, panel visible, width, the note", "settings", open) },
        Scene { prefs: prefs(Page::Font, 0, Hit::Close), ..base("prefs-close", "Pointer over the close cross", "settings", open) },
        Scene { prefs: prefs(Page::Font, 1, Hit::Panel).map(|p| Prefs { focus: false, ..p }), ..base("prefs-unfocused", "The keyboard is on the terminal: no row is lit", "settings", open) },
        Scene {
            prefs: prefs(Page::Windows, 0, Hit::Panel),
            side: Some((TabsPos::Left, None)),
            ..base("prefs-with-side", "Panel of windows at the left and the settings at the right: both beside one narrow grid", "settings", Ui { side: 2, ..open })
        },
        Scene {
            prefs: prefs(Page::Windows, 0, Hit::Choice(Row::Tabs, 0)).map(|p| Prefs { windows: WindowsCfg { tabs: TabsPos::Right, ..WindowsCfg::default() }, ..p }),
            side: Some((TabsPos::Right, None)),
            ..base("prefs-side-right", "Windows at the right: the settings go to the left, as the reference does", "settings", Ui { side: 2, ..open })
        },
        Scene {
            prefs: prefs(Page::Windows, 0, Hit::Panel).map(|p| Prefs { windows: WindowsCfg { tabs: TabsPos::Top, ..WindowsCfg::default() }, ..p }),
            side: Some((TabsPos::Top, None)),
            ..base("prefs-tabs-top", "Tabs at the top: the panel's rows (visible, width) are inert", "settings", Ui { side: 0, ..open })
        },
        Scene { lang: Lang::Es, prefs: prefs(Page::Font, 0, Hit::Panel), ..base("prefs-es", "Settings in Spanish: Font page", "language", open) },
        Scene {
            lang: Lang::Es,
            prefs: prefs(Page::Input, 3, Hit::Chip(Chip::Dir)),
            bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes]),
            input: hooked,
            ..base("prefs-es-input", "Settings in Spanish: Input page", "language", open)
        },
        Scene { lang: Lang::Es, prefs: prefs(Page::Windows, 0, Hit::Panel), ..base("prefs-es-windows", "Settings in Spanish: Windows page", "language", open) },
        Scene { lang: Lang::Es, bar: bar(&Chip::ALL), input: hooked, ..base("chips-es", "Status bar hints in Spanish", "language", Ui::IDLE) },
        Scene { font: FontCfg { smoothing: Smoothing::Gray, ..FontCfg::sized(PX) }, ..base("text-gray", "Grey smoothing, hinted", "font", Ui::IDLE) },
        Scene { font: FontCfg { hinting: false, ..FontCfg::sized(PX) }, ..base("text-unhinted", "LCD smoothing, no hinting", "font", Ui::IDLE) },
        Scene { font: FontCfg { smoothing: Smoothing::Gray, hinting: false, ..FontCfg::sized(PX) }, ..base("text-plain", "Grey smoothing, no hinting (how it was)", "font", Ui::IDLE) },
        Scene {
            prefs: prefs(Page::Input, 3, Hit::Panel).map(|p| Prefs { hold: Some((Chip::Branch, (262, 262))), ..p }),
            bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes]),
            input: hooked,
            ..base("prefs-chip-drag", "Input page: a chip held mid-drag", "settings", open)
        },
        Scene { side: Some((TabsPos::Left, None)), ..base("side", "Panel of windows at the left (Alt+B hides it)", "windows", Ui { side: 2, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Left, Some(crate::side::Part::Head(0)))), folded: true, ..base("side-groups", "Groups: a header with its count and chevron, a folded group, pointer over a header", "windows", Ui { side: 2, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Left, Some(crate::side::Part::Close(2)))), bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes]), input: hooked, ..base("side-hover", "Pointer over a row's cross, chips on", "windows", Ui { side: 2, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Right, Some(crate::side::Part::New))), ..base("side-right", "Panel at the right, pointer over the +", "windows", Ui { side: 2, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Top, Some(crate::side::Part::Row(1)))), ..base("tabs-top", "Windows as tabs in the title bar, pointer over one", "windows", Ui { side: 0, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Top, Some(crate::side::Part::Close(2)))), bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch]), input: hooked, ..base("tabs-top-close", "Tabs, pointer over a cross, chips on", "windows", Ui { side: 0, ..Ui::IDLE }) },
        base("side-btn-hover", "Pointer over the panel button, panel hidden", "windows", Ui { side: 1, hover: 5, ..Ui::IDLE }),
        Scene { panes: split(2, panes::Kind::Cols, 1, None), side: Some((TabsPos::Left, None)), bar: chips3(), ..base("panes-2", "Two panes in columns (Alt+\\): the focused one has the accent border and its ring, the other is dimmer with a hollow cursor; the panel lists them", "panes", side_ui) },
        Scene { panes: split(3, panes::Kind::Grid, 2, None), side: Some((TabsPos::Left, None)), bar: chips3(), ..base("panes-3", "Three panes (Ctrl+Space twice): the last spans both columns of the second row", "panes", side_ui) },
        Scene { panes: split(4, panes::Kind::Grid, 3, None), side: Some((TabsPos::Left, None)), bar: chips3(), ..base("panes-4", "Four panes: a grid of two by two", "panes", side_ui) },
        Scene { panes: split(5, panes::Kind::Grid, 4, None), side: Some((TabsPos::Left, Some(crate::side::Part::Pane(0, 2)))), bar: chips3(), ..base("panes-5", "Five panes: three columns, the last pane spans the two cells the second row leaves empty; pointer over a pane's row in the panel; the rows' status icons (running, failed)", "panes", side_ui) },
        Scene { panes: split(6, panes::Kind::Grid, 0, None), side: Some((TabsPos::Left, Some(crate::side::Part::ClosePane(0, 3)))), bar: chips3(), ..base("panes-6", "Six panes, the first focused; pointer over the cross of a pane's row", "panes", side_ui) },
        Scene { panes: split(3, panes::Kind::Rows, 1, Some((1, false))), side: Some((TabsPos::Left, None)), bar: chips3(), ..base("panes-rows", "Three panes in rows (Alt+-): the cross at the corner of the pane under the pointer", "panes", side_ui) },
        Scene { panes: split(4, panes::Kind::Grid, 0, Some((1, true))), side: Some((TabsPos::Left, None)), bar: chips3(), ..base("panes-cross-hot", "The pointer is on a pane's cross (closes it, Alt+Shift+W closes the focused one)", "panes", side_ui) },
        Scene { panes: split(4, panes::Kind::Cols, 2, None), side: Some((TabsPos::Top, Some(crate::side::Part::Row(0)))), bar: chips3(), ..base("panes-tabs-top", "Windows as tabs: the first says how many panes it has", "panes", Ui { side: 0, ..Ui::IDLE }) },
        Scene { panes: split(3, panes::Kind::Grid, 1, None), side: Some((TabsPos::Left, None)), bar: chips3(), lang: Lang::Es, ..base("panes-es", "Panes in Spanish: the rows of the panel say Panel n", "panes", side_ui) },
        Scene { panes: split(2, panes::Kind::Cols, 0, None), bar: chips3(), ..base("panes-no-panel", "Two panes with the panel hidden (Alt+B)", "panes", Ui { side: 1, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Left, None)), list: ListScene { status: BUSY, ..NO_LIST }, ..base("status-side", "Status icons in the panel: the prompt (idle), a ring that turns (a command runs), a red dot (the last command failed); the cyan dot is output that came while it was hidden", "status", side_ui) },
        Scene { side: Some((TabsPos::Left, None)), list: ListScene { status: BUSY, phase: 0.6, ..NO_LIST }, ..base("status-side-turned", "The same a little later: the ring's bright quarter has gone round", "status", side_ui) },
        Scene { side: Some((TabsPos::Left, Some(side::Part::Head(0)))), folded: true, list: ListScene { fold0: true, status: BUSY, ..NO_LIST }, ..base("status-folded", "A folded group shows the window on screen, and its dot turns while a window in it runs a command (the other group is folded too, idle)", "status", side_ui) },
        Scene { side: Some((TabsPos::Top, None)), list: ListScene { grouped_tabs: true, status: BUSY, ..NO_LIST }, ..base("tabs-top-groups", "Groups in the tabs: a container tinted with the group's colour, its chip (dot, name, count), the tab on screen at 16 % of the colour; status icons in the tabs", "tabs", Ui { side: 0, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Top, None)), list: ListScene { grouped_tabs: true, status: BUSY, first: 4, ..NO_LIST }, ..base("tabs-top-scrolled", "The tabs scrolled a group on (the wheel moves them a group or a window at a time): the first group is out of sight and the rest start at the left of the strip", "tabs", Ui { side: 0, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Top, Some(side::Part::Head(0)))), list: ListScene { grouped_tabs: true, fold0: true, status: BUSY, ..NO_LIST }, ..base("tabs-top-folded", "A folded group at the top keeps only the window on screen; the pointer is over its chip; the ring turns because a hidden window runs a command", "tabs", Ui { side: 0, ..Ui::IDLE }) },
        Scene { panes: split(2, panes::Kind::Cols, 1, None), side: Some((TabsPos::Top, Some(side::Part::Row(1)))), list: ListScene { grouped_tabs: true, status: BUSY, ..NO_LIST }, ..base("tabs-top-badge", "A window of two panes says so in its tab at the top (the panel lists them as rows instead)", "tabs", Ui { side: 0, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Left, None)), list: ListScene { drag: Some(DragScene { grab: crate::dnd::Grab::Win(3), over: Over::Win(1, 0.2) }), ..NO_LIST }, ..base("dnd-side-before", "Dragging a window in the panel: it stays at 40 %, its ghost follows the pointer, a 2 px accent line says it would go before servidor, in its group", "drag", side_ui) },
        Scene { side: Some((TabsPos::Left, None)), list: ListScene { drag: Some(DragScene { grab: crate::dnd::Grab::Win(3), over: Over::Group(0, 0.5) }), ..NO_LIST }, ..base("dnd-side-into", "Over a group's header: a 2 px ring in the group's colour, the window would go into the group", "drag", side_ui) },
        Scene { side: Some((TabsPos::Left, None)), list: ListScene { drag: Some(DragScene { grab: crate::dnd::Grab::Group(0), over: Over::Group(1, 0.9) }), ..NO_LIST }, ..base("dnd-side-group", "Dragging a group: its header is dimmed and the line is after the block of the group it is over", "drag", side_ui) },
        Scene { side: Some((TabsPos::Left, None)), list: ListScene { drag: Some(DragScene { grab: crate::dnd::Grab::Win(0), over: Over::Blank }), ..NO_LIST }, ..base("dnd-side-end", "Over the empty part of the panel: out of its group, last", "drag", side_ui) },
        Scene { side: Some((TabsPos::Top, None)), list: ListScene { grouped_tabs: true, drag: Some(DragScene { grab: crate::dnd::Grab::Win(3), over: Over::Win(1, 0.2) }), ..NO_LIST }, ..base("dnd-tabs-before", "Dragging a tab: a vertical accent line before the tab it is over, which would take its group", "drag", Ui { side: 0, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Top, None)), list: ListScene { grouped_tabs: true, drag: Some(DragScene { grab: crate::dnd::Grab::Win(3), over: Over::Group(0, 0.5) }), ..NO_LIST }, ..base("dnd-tabs-into", "A tab over a group's chip: the ring in the group's colour", "drag", Ui { side: 0, ..Ui::IDLE }) },
        Scene { side: Some((TabsPos::Top, None)), list: ListScene { grouped_tabs: true, drag: Some(DragScene { grab: crate::dnd::Grab::Group(0), over: Over::Group(1, 0.8) }), ..NO_LIST }, ..base("dnd-tabs-group", "Dragging a group by its chip: the line is after the group it is over", "drag", Ui { side: 0, ..Ui::IDLE }) },
        Scene { font: font(300, 0.0, 1.0), ..base("font-light", "Weight 300 (light)", "font", Ui::IDLE) },
        Scene { font: font(500, 0.0, 1.0), ..base("font-medium", "Weight 500 (medium)", "font", Ui::IDLE) },
        Scene { font: font(400, 1.0, 1.0), ..base("font-spaced", "Letter spacing +1 px", "font", Ui::IDLE) },
        Scene { font: font(400, 0.0, 1.3), ..base("font-tall", "Line height 1.3", "font", Ui::IDLE) },
    ]
}

/// Renders a scene; returns the frame (premultiplied `Argb8888`) and its size.
fn render(s: &Scene) -> (Vec<u32>, usize, usize) {
    i18n::set(s.lang);
    let mut soft = Soft::new(&s.font, s.opacity);
    soft.cursor_color = crate::cell::pack(palette::CURSOR.r, palette::CURSOR.g, palette::CURSOR.b, 255);
    soft.set_title(s.title.split_once('|').map_or(s.title, |t| t.0), s.title.split_once('|').map_or("", |t| t.1));
    soft.set_ui(s.ui);
    let (cw, ch) = soft.cell();
    let side_w = if matches!(s.side, Some((TabsPos::Left | TabsPos::Right, _))) { 220 } else { 0 };
    let tabs = s.side.map_or(TabsPos::Left, |(pos, _)| pos);
    let prefs_w = if s.prefs.is_some() { settings::panel_width(W) } else { 0 };
    // The settings are at the edge opposite the windows' panel (`wlapp::docks`).
    soft.set_docks(Dock { w: side_w, left: tabs != TabsPos::Right }, Dock { w: prefs_w, left: tabs == TabsPos::Right });
    if s.side.is_some() {
        soft.set_tabs_top(tabs == TabsPos::Top);
    }
    let (cols, rows) = session::grid_size(W - side_w - prefs_w, H, cw, ch, soft.pad, soft.top, soft.bottom);
    let (ml, mt, mb) = chrome::margins(s.ui.maximized);
    let (w, h) = (W as usize + 2 * ml, H as usize + mt + mb);
    let mut fb = vec![0u32; w * h];
    let (ctx, chips) = s.bar.clone().unwrap_or_default();
    if let Some(ps) = s.panes {
        // A window split into panes: each has a terminal of the size of its own box, and is drawn in turn.
        let ws = soft.workspace(W, H);
        let slots: Vec<Slot> = (0..ps.n).map(|i| Slot { rect: panes::rect(ps.kind, ps.n, i, ws), focus: i == ps.focus, framed: ps.n > 1 }).collect();
        soft.set_layout(&slots);
        let grids: Vec<panes::Grid> = slots.iter().map(|sl| soft.grid(sl.rect, sl.framed)).collect();
        soft.set_status("zsh", "main", grids[ps.focus].cols, grids[ps.focus].rows);
        for (i, g) in grids.iter().enumerate() {
            let mut term = Term::new(Config::default(), &session::Size { cols: g.cols, rows: g.rows }, VoidListener);
            Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut term, pane_text(i).as_bytes());
            soft.select(i);
            soft.set_dim(if i == ps.focus { 255 } else { UNFOCUSED });
            bar::Chips::default().frame(s.bar.is_some(), true, &demo_history(&ctx), &chips, &mut term, &mut View::new(), &mut soft, &mut fb, (w, h), s.ui.focused && i == ps.focus);
        }
        soft.set_dim(255);
        soft.select(0);
        if let Some((i, hot)) = ps.cross {
            soft.paint_pane_close(&mut fb, i, hot);
        }
    } else {
        let mut term = Term::new(Config::default(), &session::Size { cols, rows }, VoidListener);
        let mut p: Processor = Processor::new();
        p.advance(&mut term, s.input.as_bytes());
        if let Some((line, len)) = s.select {
            let mut sel = Selection::new(SelectionType::Simple, Point::new(Line(line), Column(1)), Side::Left);
            sel.update(Point::new(Line(line), Column(len)), Side::Right);
            term.selection = Some(sel);
        }
        soft.set_status("zsh", "main", cols, rows);
        bar::Chips::default().frame(s.bar.is_some(), true, &demo_history(&ctx), &chips, &mut term, &mut View::new(), &mut soft, &mut fb, (w, h), s.ui.focused);
    }
    if let Some((pos, hover)) = s.side {
        // The first window's panes: rows under it in the panel, a number in its tab at the top.
        let pane_n = s.panes.map_or(1, |p| p.n);
        let mut items = crate::side::Items::default();
        let titles = [("arcton", true, false), ("servidor", false, false), ("pruebas", false, true), ("~", false, false), ("notas", false, false)];
        for (i, ((title, selected, unread), st)) in titles.into_iter().zip(s.list.status).enumerate() {
            // A window being renamed has no title under its field.
            let title = if s.edit.is_some_and(|(wi, ..)| wi == i) { "" } else { title };
            let ps = s.panes.filter(|p| i == 0 && p.n > 1);
            items.push_win(title, selected, unread, ps.filter(|_| pos == TabsPos::Top).map_or(0, |p| p.n), st);
            for j in 0..ps.map_or(0, |p| p.n) {
                items.push_pane(j == ps.map_or(0, |p| p.focus), pane_status(j), |t| t.push_str(&format!("{} {}", i18n::tr(i18n::Str::Pane), j + 1)));
            }
        }
        // The reference's session: a group of three windows, one in none, and a folded group of one.
        let group = |name: &str, color, collapsed, count, busy| crate::side::GroupItem { id: 0, name: name.to_string(), color, collapsed, count, busy };
        // A folded group is busy while a window in it runs a command: the windows 0 to 2 are in the first, 4 in the second.
        let running = |ws: &[usize]| ws.iter().any(|&w| s.list.status[w] == Status::Running);
        let groups = [group("arcton", 1, s.list.fold0, 2 + pane_n, s.list.fold0 && running(&[0, 1, 2])), group("docs", 3, s.folded, 1, s.folded && running(&[4]))];
        let win_group = [Some(0), Some(0), Some(0), None, Some(1)];
        let folds = [s.list.fold0, s.folded];
        let (ox, oy) = soft.origin();
        let edit = s.edit.map(|(wi, text, all)| (wi, text, text.chars().count(), all.then(|| (0, text.chars().count()))));
        let top = matches!((pos, soft.side_rect()), (TabsPos::Top, _) | (_, None));
        let entries = if top && !s.list.grouped_tabs {
            // The older scenes: one tab for each window, none grouped.
            (0..items.len()).map(|win| crate::side::Entry::Row { win, grouped: false }).collect()
        } else if top {
            crate::side::top_entries(&win_group, &folds, 0)
        } else {
            crate::side::entries(&win_group, &folds, &[pane_n, 1, 1, 1, 1], Some(0))
        };
        let (x0, x1) = (chrome::TABS_X0 as i32, W as i32 - chrome::TABS_FROM_RIGHT as i32);
        let panel = soft.side_rect().map(|rect| (rect, ((rect.0 - ox) as i32, (rect.1 - oy) as i32, (rect.2 - rect.0) as i32, (rect.3 - rect.1) as i32)));
        // The drag, if any: the pointer at its place in the list, and what the real layout makes of it.
        let layout = match panel {
            Some((_, b)) if !top => crate::side::Layout::Side { x0: b.0, y0: b.1, w: b.2, entries: &entries },
            _ => crate::side::Layout::Top { x0, x1, entries: &entries, groups: &groups, cw: crate::side::chip_cw(&soft), first: s.list.first },
        };
        let drag = s.list.drag.and_then(|d| {
            let r = layout.grab_box(d.grab)?;
            let at = |b: (i32, i32, i32, i32), f: f32| if top { (b.0 as f32 + b.2 as f32 * f, b.1 as f32 + b.3 as f32 / 2.0) } else { (b.0 as f32 + b.2 as f32 / 2.0, b.1 as f32 + b.3 as f32 * f) };
            let pointer = match d.over {
                Over::Win(w, f) => at(layout.grab_box(crate::dnd::Grab::Win(w))?, f),
                Over::Group(g, f) => at(layout.grab_box(crate::dnd::Grab::Group(g))?, f),
                Over::Blank => match panel {
                    Some((_, b)) if !top => ((b.0 + b.2 / 2) as f32, (b.1 + b.3 - 60) as f32),
                    _ => ((x1 - 40) as f32, r.1 as f32 + 10.0),
                },
            };
            let target = layout.target(d.grab, pointer.0, pointer.1);
            let region = match panel {
                Some((_, b)) if !top => b,
                _ => (x0, r.1, x1 - x0, r.3),
            };
            let off = (24.0f32.min(r.2 as f32 / 2.0), r.3 as f32 / 2.0);
            Some(side::Dragging { grab: d.grab, target, at: side::ghost_at(region, (r.2, r.3), pointer, off) })
        });
        let look = side::Look { hover, drag, phase: s.list.phase };
        if top {
            soft.clear_rect(&mut fb, (ox + x0 as usize, oy, ox + x1 as usize, oy + chrome::TITLE_H));
            crate::side::paint_top(&soft, &mut fb, (x0, x1), s.list.first, &entries, &groups, &items, look, &mut Vec::new());
            if let Some((wi, text, caret, sel)) = edit {
                crate::side::paint_rename(&soft, &mut fb, layout, wi, &crate::side::Edit { text, caret, sel }, hover, &mut Vec::new());
            }
        } else if let Some((rect, b)) = panel {
            soft.clear_rect(&mut fb, rect);
            crate::side::paint(&soft, &mut fb, b, pos == TabsPos::Left, &i18n::tr(i18n::Str::Windows), &entries, &groups, &items, look, &mut Vec::new());
            if let Some((wi, text, caret, sel)) = edit {
                crate::side::paint_rename(&soft, &mut fb, layout, wi, &crate::side::Edit { text, caret, sel }, hover, &mut Vec::new());
            }
        }
        if let Some(phase) = s.list.respin {
            let bottom = panel.map_or(0, |(_, b)| b.1 + b.3);
            side::paint_spin(&soft, &mut fb, layout, bottom, &entries, &groups, &items, side::Look { phase, ..look });
        }
    }
    if let (Some(pf), Some(rect), Some(b)) = (&s.prefs, soft.prefs_rect(), soft.prefs_box((W, H))) {
        // Fixed fonts, not whatever is installed: the board must look the same on every machine.
        let families = vec![("DejaVu Sans Mono".to_string(), "/x/a.ttf".into()), ("JetBrains Mono".to_string(), "/x/b.ttf".into())];
        let input = match &s.bar {
            Some((_, chips)) => crate::config::InputCfg { inline: true, chips: chips.clone(), clock24: false, ..Default::default() },
            None => crate::config::InputCfg::default(),
        };
        let mut panel = Settings::new(&s.font, s.opacity, families).with_input(&input).with_windows(&pf.windows).with_blur(pf.blur.0, pf.blur.1).with_ui_font();
        panel.place(b, tabs == TabsPos::Right);
        panel.page = pf.page;
        panel.focus = pf.focus;
        panel.tabs_focus = pf.tabs;
        if let Some((chip, at)) = pf.hold {
            panel.hold(chip, at);
        }
        (panel.sel, panel.hover) = (pf.sel, pf.hover);
        for c in pf.typing.chars() {
            panel.key(&crate::xkb::Key { text: c.to_string(), ..Default::default() });
        }
        panel.scroll_by(pf.scroll);
        soft.clear_rect(&mut fb, rect);
        panel.draw(&mut fb, w, h, (ml, mt));
    }
    i18n::set(Lang::En);
    (fb, w, h)
}

/// The renderer repaints only what changed (rows, the title bar) and leaves the shadow margin
/// alone between full repaints; after every step that must equal painting from scratch.
#[test]
fn incremental_repaint_matches_a_full_one() {
    let steps: [(Ui, &str, &str); 7] = [
        (Ui::IDLE, "one", SHELL),
        (Ui { hover: 1, ..Ui::IDLE }, "one", ""),
        (Ui { hover: 3, down: true, ..Ui::IDLE }, "a longer title here", "\r\nmore output\r\n$ "),
        (Ui { focused: false, ..Ui::IDLE }, "a longer title here", ""),
        (Ui { maximized: true, ..Ui::IDLE }, "x", "ls\r\n"),
        (Ui::IDLE, "back", "\x1b[2J\x1b[H$ "),
        (Ui { hover: 2, ..Ui::IDLE }, "back", "\x1b[31mred\x1b[0m"),
    ];
    let new = |s: &Soft| {
        let (cw, ch) = s.cell();
        let (cols, rows) = session::grid_size(W, H, cw, ch, s.pad, s.top, s.bottom);
        Term::new(Config::default(), &session::Size { cols, rows }, VoidListener)
    };
    let mut soft = Soft::new(&FontCfg::sized(PX), OPACITY);
    let mut term = new(&soft);
    let mut view = View::new();
    let mut fb = Vec::new();
    let mut input = String::new();
    for (n, (ui, title, out)) in steps.into_iter().enumerate() {
        input.push_str(out);
        let (ml, mt, mb) = chrome::margins(ui.maximized);
        let (w, h) = (W as usize + 2 * ml, H as usize + mt + mb);
        if fb.len() != w * h {
            fb = vec![0xdead_beef; w * h]; // a new buffer holds garbage until painted
        }
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut term, out.as_bytes());
        soft.set_title(title, "");
        soft.set_ui(ui);
        view.build(&mut term, ui.focused, &mut Target { soft: &mut soft, fb: &mut fb, w, h });

        let mut fresh = Soft::new(&FontCfg::sized(PX), OPACITY);
        let mut fresh_term = new(&fresh);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut fresh_term, input.as_bytes());
        fresh.set_title(title, "");
        fresh.set_ui(ui);
        let mut full = vec![0u32; w * h];
        View::new().build(&mut fresh_term, ui.focused, &mut Target { soft: &mut fresh, fb: &mut full, w, h });
        let diff = fb.iter().zip(&full).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 0, "step {n}: {diff} pixels differ from a full repaint");
    }
}

/// The spinners turn between full paints by painting only the rings (`side::paint_spin`): whatever the list shows,
/// that is the frame painting everything at the new phase would give.
#[test]
fn turning_the_spinners_matches_painting_the_list_again() {
    let names: Vec<&str> = scenes().iter().filter(|s| s.list.status.contains(&Status::Running) || s.panes.is_some_and(|p| p.n > 1)).map(|s| s.name).collect();
    assert!(names.len() >= 5, "{names:?}");
    for name in names {
        for (from, to) in [(0.0, 0.3), (0.125, 0.7), (0.9, 0.05)] {
            let scene = |phase, respin| {
                let mut s = scenes().into_iter().find(|s| s.name == name).unwrap();
                (s.list.phase, s.list.respin) = (phase, respin);
                render(&s)
            };
            let (again, whole) = (scene(from, Some(to)), scene(to, None));
            assert_eq!((again.1, again.2), (whole.1, whole.2));
            let diff = again.0.iter().zip(&whole.0).filter(|(a, b)| a != b).count();
            assert_eq!(diff, 0, "{name}: {diff} pixels differ after turning the spinners from {from} to {to}");
        }
    }
}

/// The settings panel is painted in its own box when the frame is whole or when it changed, over a grid
/// that is repainted row by row: whatever the order, the frame is the one painted from scratch.
#[test]
fn the_docked_panel_survives_the_grid_repainting_beside_it() {
    use crate::settings::{Page, panel_width};
    // (what the terminal prints, the page the panel shows, whether the panel changed in this step)
    let steps: [(&str, Page, bool); 6] = [
        (SHELL, Page::Font, true),
        ("\r\nmore output\r\n$ ", Page::Font, false),
        ("x", Page::Font, false),
        ("\x1b[2J\x1b[H$ ", Page::Input, true),
        ("\r\nls\r\n", Page::Input, false),
        ("\x1b[31mred\x1b[0m", Page::Windows, true),
    ];
    let pw = panel_width(W);
    let make = || {
        let mut soft = Soft::new(&FontCfg::sized(PX), OPACITY);
        let (cw, ch) = soft.cell();
        soft.set_docks(Dock::default(), Dock { w: pw, left: false });
        let (cols, rows) = session::grid_size(W - pw, H, cw, ch, soft.pad, soft.top, soft.bottom);
        let term = Term::new(Config::default(), &session::Size { cols, rows }, VoidListener);
        (soft, term)
    };
    let (ml, mt, mb) = chrome::margins(false);
    let (w, h) = (W as usize + 2 * ml, H as usize + mt + mb);
    let panel = |soft: &Soft, page| {
        let mut p = Settings::new(&FontCfg::sized(PX), OPACITY, Vec::new());
        p.place(soft.prefs_box((W, H)).unwrap(), false);
        p.page = page;
        p
    };
    let (mut soft, mut term) = make();
    let (mut view, mut fb, mut seen) = (View::new(), vec![0xdead_beefu32; w * h], String::new());
    for (n, (out, page, changed)) in steps.into_iter().enumerate() {
        seen.push_str(out);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut term, out.as_bytes());
        soft.set_ui(Ui { prefs: true, ..Ui::IDLE });
        view.build(&mut term, true, &mut Target { soft: &mut soft, fb: &mut fb, w, h });
        if soft.last_full || changed {
            soft.clear_rect(&mut fb, soft.prefs_rect().unwrap());
            panel(&soft, page).draw(&mut fb, w, h, (ml, mt));
        }

        let (mut fresh, mut fterm) = make();
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut fterm, seen.as_bytes());
        fresh.set_ui(Ui { prefs: true, ..Ui::IDLE });
        let mut full = vec![0u32; w * h];
        View::new().build(&mut fterm, true, &mut Target { soft: &mut fresh, fb: &mut full, w, h });
        fresh.clear_rect(&mut full, fresh.prefs_rect().unwrap());
        panel(&fresh, page).draw(&mut full, w, h, (ml, mt));
        let diff = fb.iter().zip(&full).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 0, "step {n}: {diff} pixels differ from a full repaint");
    }
}

/// A toast fades in, stays and fades out over the window, with the terminal printing under it. Under it the frame
/// gives back only the toast's box (`Soft::restore` and the rows it touches), and still the picture is the one painted
/// from scratch with the toast at that moment: the translucent pixels of the frame before are not piled up.
#[test]
fn a_toast_fading_over_the_window_repaints_only_its_box_like_a_full_frame() {
    use std::time::{Duration, Instant};

    use crate::overlay::{Extent, Geo, Stack};

    let g = Geo { w: W as i32, h: H as i32, square: false };
    let (ml, mt, mb) = chrome::margins(false);
    let (w, h) = (W as usize + 2 * ml, H as usize + mt + mb);
    let make = || {
        let mut soft = Soft::new(&FontCfg::sized(PX), OPACITY);
        let (cw, ch) = soft.cell();
        let (cols, rows) = session::grid_size(W, H, cw, ch, soft.pad, soft.top, soft.bottom);
        soft.set_status("zsh", "main", cols, rows);
        (soft, Term::new(Config::default(), &session::Size { cols, rows }, VoidListener))
    };
    let t0 = Instant::now();
    let mut stack = Stack::default();
    stack.toast("Closed \u{ab}arcton\u{bb}", Some("Undo"), 1, t0);
    // When (ms after the toast was shown) and what the terminal printed since the frame before.
    let steps: [(u64, &str); 8] = [(0, SHELL), (60, ""), (130, "\r\n$ ls\r\n"), (400, ""), (3000, "x"), (6300, ""), (6420, "\r\nmore output\r\n"), (6600, "")];
    let (mut soft, mut term) = make();
    let (mut view, mut fb, mut seen, mut extent) = (View::new(), vec![0xdead_beefu32; w * h], String::new(), Extent::Nothing);
    let mut partial = 0;
    for (n, (ms, out)) in steps.into_iter().enumerate() {
        let now = t0 + Duration::from_millis(ms);
        seen.push_str(out);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut term, out.as_bytes());
        stack.tick(now);
        // What the app does before painting a frame (`wlapp::give_back_to_overlays`).
        let now_extent = stack.extent(g);
        match std::mem::replace(&mut extent, now_extent).to_restore(now_extent) {
            None => {}
            Some(Extent::Part((x, y, bw, bh))) => {
                let r = panes::Rect::new(x, y, bw, bh);
                assert!(soft.restore(r), "step {n}: the toast's box is a part of the frame");
                for row in soft.rows_under(0, r.y, r.y2()) {
                    view.invalidate_row(row);
                }
            }
            Some(_) => soft.invalidate(),
        }
        view.build(&mut term, true, &mut Target { soft: &mut soft, fb: &mut fb, w, h });
        partial += !soft.last_full as usize;
        stack.draw(&mut fb, (w, h), (ml, mt), g, now);

        let (mut fresh, mut fterm) = make();
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut fterm, seen.as_bytes());
        let mut full = vec![0u32; w * h];
        View::new().build(&mut fterm, true, &mut Target { soft: &mut fresh, fb: &mut full, w, h });
        stack.draw(&mut full, (w, h), (ml, mt), g, now);
        let diff = fb.iter().zip(&full).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 0, "step {n} ({ms} ms): {diff} pixels differ from a full repaint");
    }
    assert!(partial >= 6, "the fade was painted part by part, not a window each time ({partial} of {} frames)", steps.len());
}

/// The prompt chips are drawn over rows the terminal did not change, so painting row by row must still
/// give exactly what painting everything gives: when a prompt moves or scrolls away, when text is typed after
/// it, when a command's result arrives, when the newest prompt's chips change and when they are switched
/// off and on. Old prompts keep their chips and their status.
#[test]
fn chips_repaint_incrementally_like_a_full_frame() {
    let prompt = |n: u32| format!("\x1b[8;38;2;0;0;{n}m \x1b[0m\r\nme$ \x1b[8m \x1b[0m");
    let mut other = sample_context();
    other.text[Chip::Dir as usize] = "~/elsewhere".into();
    let chips = [Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes];
    // (what the terminal prints, chips on, a prompt that came with the status of the command before it, the newest chips change)
    let steps: Vec<(String, bool, Option<(u32, i32)>, bool)> = vec![
        (format!("first\r\n{}", prompt(1)), true, Some((1, 0)), false),
        ("ls -la".into(), true, None, false),
        (format!("\r\nout-a\r\nout-b\r\n{}", prompt(2)), true, Some((2, 0)), false),
        (String::new(), true, None, true),
        ("\r\n".repeat(30) + &prompt(3), true, Some((3, 127)), false),
        (String::new(), false, None, false),
        ("x".into(), true, None, false),
        (format!("\r\n\x1b[2J\x1b[H{}", prompt(4)), true, Some((4, 0)), false),
    ];
    let make = || {
        let soft = Soft::new(&FontCfg::sized(PX), OPACITY);
        let (cw, ch) = soft.cell();
        let (cols, rows) = session::grid_size(W, H, cw, ch, soft.pad, soft.top, soft.bottom);
        (soft, Term::new(Config::default(), &session::Size { cols, rows }, VoidListener))
    };
    let (mut soft, mut term) = make();
    let (mut view, mut chips_state, mut history) = (View::new(), bar::Chips::default(), History::default());
    let (w, h) = (W as usize + 2 * chrome::margins(false).0, H as usize + chrome::margins(false).1 + chrome::margins(false).2);
    let mut fb = vec![0xdead_beefu32; w * h];
    let mut seen = String::new();
    let t0 = std::time::Instant::now();
    for (i, (out, on, came, change)) in steps.iter().enumerate() {
        seen.push_str(out);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut term, out.as_bytes());
        if let Some((n, status)) = *came {
            if n > 1 {
                history.start(t0);
            }
            history.prompt(n, status, t0 + std::time::Duration::from_millis(100 * n as u64), &sample_context());
        }
        if *change {
            history.set_ctx(&other);
        }
        chips_state.frame(*on, false, &history, &chips, &mut term, &mut view, &mut soft, &mut fb, (w, h), true);

        let (mut fsoft, mut fterm) = make();
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut fterm, seen.as_bytes());
        let mut full = vec![0u32; w * h];
        bar::Chips::default().frame(*on, true, &history, &chips, &mut fterm, &mut View::new(), &mut fsoft, &mut full, (w, h), true);
        let diff = fb.iter().zip(&full).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 0, "step {i}: {diff} pixels differ from a full repaint");
    }
}

/// Split panes are repainted row by row like one grid: output in one pane, the focus moving (which repaints the
/// frame: borders, ring and dimming change), a pane closing and the cross at a pane's corner coming and going
/// must still give exactly what painting everything from scratch gives.
#[test]
fn panes_repaint_incrementally_like_a_full_frame() {
    struct Pane {
        term: Term<VoidListener>,
        view: View,
        chips: bar::Chips,
    }
    let make_soft = || {
        let mut soft = Soft::new(&FontCfg::sized(PX), OPACITY);
        soft.set_docks(Dock { w: 220, left: true }, Dock::default());
        soft
    };
    let (ml, mt, mb) = chrome::margins(false);
    let (w, h) = (W as usize + 2 * ml, H as usize + mt + mb);
    // One frame of the panes `n` in `kind` with the focus on `focus` and the cross on pane `cross`, from the
    // texts each pane has been given: `keep` panes carry their state over, the others are built from scratch.
    let frame = |soft: &mut Soft, panes: &mut Vec<Pane>, fb: &mut Vec<u32>, kind: panes::Kind, focus: usize, cross: Option<usize>, last_cross: Option<usize>| {
        let n = panes.len();
        let ws = soft.workspace(W, H);
        let slots: Vec<Slot> = (0..n).map(|i| Slot { rect: panes::rect(kind, n, i, ws), focus: i == focus, framed: n > 1 }).collect();
        soft.set_layout(&slots);
        soft.set_status("zsh", "main", 10, 5);
        for i in [last_cross, cross].into_iter().flatten().filter(|&i| i < n) {
            for r in soft.close_rows(i) {
                panes[i].view.invalidate_row(r);
            }
        }
        for (i, p) in panes.iter_mut().enumerate() {
            soft.select(i);
            soft.set_dim(if i == focus { 255 } else { UNFOCUSED });
            p.chips.frame(false, false, &History::default(), &[], &mut p.term, &mut p.view, soft, fb, (w, h), i == focus);
        }
        soft.set_dim(255);
        soft.select(0);
        if let Some(i) = cross {
            soft.paint_pane_close(fb, i, false);
        }
    };
    let new_term = |soft: &Soft, kind: panes::Kind, n: usize, i: usize, text: &str| {
        let ws = soft.workspace(W, H);
        let g = soft.grid(panes::rect(kind, n, i, ws), n > 1);
        let mut t = Term::new(Config::default(), &session::Size { cols: g.cols, rows: g.rows }, VoidListener);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut t, text.as_bytes());
        t
    };
    // Three panes in a grid; each step is (the focus, the pane showing its cross, output for the pane two after
    // the focused one: never the focused pane, so a pane without the focus is shown changing too).
    let kind = panes::Kind::Grid;
    let scroll = "\r\n".repeat(30) + "scrolled\r\n";
    let plan: [(usize, Option<usize>, &str); 7] = [
        (0, None, ""),
        (0, None, "\r\nmore output\r\n$ "),
        (2, None, ""),
        (2, Some(0), "\x1b[31mred\x1b[0m"),
        (2, Some(1), "x"),
        (1, None, scroll.as_str()),
        (1, Some(2), ""),
    ];
    let base = ["$ one", "$ two", "$ three"];
    let mut soft = make_soft();
    let mut panes: Vec<Pane> = (0..3).map(|i| Pane { term: new_term(&soft, kind, 3, i, base[i]), view: View::new(), chips: bar::Chips::default() }).collect();
    let mut texts: Vec<String> = base.iter().map(|s| s.to_string()).collect();
    let mut fb = vec![0xdead_beefu32; w * h];
    let mut last_cross = None;
    for (n, (focus, cross, out)) in plan.into_iter().enumerate() {
        // The output goes to the pane before the focus: another pane than the one with the focus must repaint too.
        let to = (focus + 2) % 3;
        texts[to].push_str(out);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut panes[to].term, out.as_bytes());
        frame(&mut soft, &mut panes, &mut fb, kind, focus, cross, last_cross);
        last_cross = cross;

        let mut fresh_soft = make_soft();
        let mut fresh: Vec<Pane> = (0..3).map(|i| Pane { term: new_term(&fresh_soft, kind, 3, i, &texts[i]), view: View::new(), chips: bar::Chips::default() }).collect();
        let mut full = vec![0u32; w * h];
        frame(&mut fresh_soft, &mut fresh, &mut full, kind, focus, cross, None);
        let diff = fb.iter().zip(&full).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 0, "step {n}: {diff} pixels differ from a full repaint");
    }
}

/// Cells a program paints in the terminal's own background colour are glass, like default ones:
/// a theme that sets it explicitly must not leave an opaque block on the window.
#[test]
fn background_coloured_cells_are_glass() {
    let paint = |input: &str| {
        let s = Scene { input: Box::leak(input.to_owned().into_boxed_str()), ..scenes().remove(0) };
        render(&s).0
    };
    let bg = palette::BG;
    let explicit = format!("\x1b[48;2;{};{};{}m          \x1b[0m\r\n\x1b[41m          \x1b[0m", bg.r, bg.g, bg.b);
    let (glass, painted) = (paint("          \r\n          "), paint(&explicit));
    let diff = glass.iter().zip(&painted).filter(|(a, b)| a != b).count();
    // Only the red row differs (10 cells), and no pixel of the background-coloured row.
    let (cw, ch) = Soft::new(&FontCfg::sized(PX), OPACITY).cell();
    assert_eq!(diff, (10 * cw * ch) as usize);
}

#[test]
#[ignore = "writes design/frames; run through scripts/design.sh"]
fn frames() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("design/frames");
    std::fs::create_dir_all(&dir).unwrap();
    let mut list = String::new();
    for s in scenes() {
        let (fb, w, h) = render(&s);
        std::fs::write(dir.join(format!("{}.png", s.name)), png(&fb, w, h)).unwrap();
        // `ox`, `oy`: where the window starts in the frame (the shadow margin).
        let (ox, oy, _) = chrome::margins(s.ui.maximized);
        list += &format!(
            "    {{ name: {:?}, label: {:?}, group: {:?}, w: {w}, h: {h}, ox: {ox}, oy: {oy}, opacity: {} }},\n",
            s.name, s.label, s.group, s.opacity
        );
    }
    // The overlay kit's scenes (`design/overlay_scenes.rs`), over the same window.
    let (ox, oy, _) = chrome::margins(false);
    for f in overlay_scenes::frames() {
        std::fs::write(dir.join(format!("{}.png", f.name)), png(&f.fb, f.w, f.h)).unwrap();
        list += &format!(
            "    {{ name: {:?}, label: {:?}, group: \"overlays\", w: {}, h: {}, ox: {ox}, oy: {oy}, opacity: {OPACITY} }},\n",
            f.name, f.label, f.w, f.h
        );
    }
    // The overview of windows (`design/overview_scenes.rs`).
    for f in overview_scenes::frames() {
        std::fs::write(dir.join(format!("{}.png", f.name)), png(&f.fb, f.w, f.h)).unwrap();
        list += &format!(
            "    {{ name: {:?}, label: {:?}, group: \"overview\", w: {}, h: {}, ox: {ox}, oy: {oy}, opacity: {OPACITY} }},\n",
            f.name, f.label, f.w, f.h
        );
    }
    // The group dialog, the context menus, the rename field and the toasts of the window list (`design/group_scenes.rs`).
    for f in group_scenes::frames() {
        std::fs::write(dir.join(format!("{}.png", f.name)), png(&f.fb, f.w, f.h)).unwrap();
        list += &format!(
            "    {{ name: {:?}, label: {:?}, group: \"groups\", w: {}, h: {}, ox: {ox}, oy: {oy}, opacity: {OPACITY} }},\n",
            f.name, f.label, f.w, f.h
        );
    }
    std::fs::write(dir.join("frames.js"), tokens(&list)).unwrap();
}

/// `frames.js`: the scenes and the tokens the board shows, straight from the constants.
fn tokens(frames: &str) -> String {
    let hex = |c: alacritty_terminal::vte::ansi::Rgb| format!("\"#{:02x}{:02x}{:02x}\"", c.r, c.g, c.b);
    let ansi: Vec<String> = (0..16).map(|i| hex(palette::by_index(i))).collect();
    let soft = Soft::new(&FontCfg::sized(PX), OPACITY);
    let (cw, ch) = soft.cell();
    let stamp = std::env::var("AXON_DESIGN_STAMP").unwrap_or_default();
    format!(
        "// Generated by `scripts/design.sh` (src/design.rs). Do not edit.\n\
window.AXON_DESIGN = {{\n  stamp: {stamp:?},\n  window: {{ w: {W}, h: {H} }},\n  frames: [\n{frames}  ],\n  tokens: {{\n\
    radius: {}, titleBar: {}, opacity: {OPACITY}, fontPx: {PX}, cell: [{cw}, {ch}], pad: {}, top: {},\n\
    shadow: \"spread {} px, cast {} px down; margin {} px left and right, {} px above, {} px below\",\n\
    fg: {}, bg: {}, cursor: {}, selection: {},\n    ansi: [{}],\n  }},\n}};\n",
        chrome::RADIUS,
        chrome::TITLE_H,
        soft.pad,
        soft.top,
        chrome::SHADOW_BLUR,
        chrome::SHADOW_DY,
        chrome::margins(false).0,
        chrome::margins(false).1,
        chrome::margins(false).2,
        hex(palette::FG),
        hex(palette::BG),
        hex(palette::CURSOR),
        hex(palette::SELECTION_BG),
        ansi.join(", "),
    )
}

// ------------------------------------------------------------------- png --

/// PNG of a premultiplied `Argb8888` frame (PNG wants straight alpha).
fn png(fb: &[u32], w: usize, h: usize) -> Vec<u8> {
    let stride = w * 4;
    let mut rgba = Vec::with_capacity(stride * h);
    for &px in fb {
        let a = px >> 24;
        let un = |c: u32| if a == 0 { 0 } else { ((c * 255 + a / 2) / a).min(255) as u8 };
        rgba.extend_from_slice(&[un((px >> 16) & 255), un((px >> 8) & 255), un(px & 255), a as u8]);
    }
    // Per row, the filter (none, sub, up) with the smallest sum of magnitudes.
    let mut raw = Vec::with_capacity((stride + 1) * h);
    let mut row = vec![0u8; stride];
    for y in 0..h {
        let cur = &rgba[y * stride..][..stride];
        let mut best = (u64::MAX, 0u8);
        for f in 0..3u8 {
            filter(f, cur, if y > 0 { Some(&rgba[(y - 1) * stride..][..stride]) } else { None }, &mut row);
            let cost = row.iter().map(|&b| (b as i8).unsigned_abs() as u64).sum();
            if cost < best.0 {
                best = (cost, f);
            }
        }
        filter(best.1, cur, if y > 0 { Some(&rgba[(y - 1) * stride..][..stride]) } else { None }, &mut row);
        raw.push(best.1);
        raw.extend_from_slice(&row);
    }
    let mut z = vec![0x78, 0x01];
    z.extend(deflate(&raw));
    z.extend(adler32(&raw).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]); // 8 bits, RGBA, deflate, adaptive filters, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn filter(f: u8, cur: &[u8], prev: Option<&[u8]>, out: &mut [u8]) {
    for i in 0..cur.len() {
        let left = if i >= 4 { cur[i - 4] } else { 0 };
        let up = prev.map_or(0, |p| p[i]);
        out[i] = match f {
            1 => cur[i].wrapping_sub(left),
            2 => cur[i].wrapping_sub(up),
            _ => cur[i],
        };
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

/// Bits written least significant first, as deflate packs them.
struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn put(&mut self, v: u32, n: u32) {
        self.acc |= v << self.n;
        self.n += n;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// A Huffman code: most significant bit first.
    fn code(&mut self, code: u32, len: u32) {
        let rev = (0..len).fold(0, |r, i| r | ((code >> i) & 1) << (len - 1 - i));
        self.put(rev, len);
    }

    /// A literal/length symbol in the fixed code (RFC 1951 3.2.6).
    fn sym(&mut self, v: u32) {
        match v {
            0..=143 => self.code(0x30 + v, 8),
            144..=255 => self.code(0x190 + v - 144, 9),
            256..=279 => self.code(v - 256, 7),
            _ => self.code(0xc0 + v - 280, 8),
        }
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];

/// One fixed-Huffman block. The only matches are runs of the previous byte (distance 1): after
/// the row filters, flat UI areas are long runs of zeros, which is where nearly all the bytes are.
fn deflate(data: &[u8]) -> Vec<u8> {
    let mut b = Bits { out: Vec::with_capacity(data.len() / 8), acc: 0, n: 0 };
    b.put(1, 1); // final block
    b.put(1, 2); // fixed Huffman codes
    let mut i = 0;
    while i < data.len() {
        let mut run = 0;
        if i > 0 {
            while run < 258 && i + run < data.len() && data[i + run] == data[i - 1] {
                run += 1;
            }
        }
        if run >= 3 {
            let k = LEN_BASE.iter().rposition(|&l| l as usize <= run).unwrap();
            b.sym(257 + k as u32);
            b.put((run - LEN_BASE[k] as usize) as u32, LEN_EXTRA[k] as u32);
            b.code(0, 5); // distance code 0: distance 1
            i += run;
        } else {
            b.sym(data[i] as u32);
            i += 1;
        }
    }
    b.sym(256); // end of block
    if b.n > 0 {
        b.out.push(b.acc as u8);
    }
    b.out
}

#[test]
fn png_header_and_checksums() {
    // The deflate stream itself is checked against a real decoder when the frames are made
    // (scripts/design.sh); here the framing and the two checksums against their reference values.
    let fb = [0xff00_0000u32, 0xff00_0000, 0xff00_0000, 0x8080_8080, 0, 0xffff_ffff];
    let p = png(&fb, 3, 2);
    assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&p[12..16], b"IHDR");
    assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
}

/// The status bar costs rows, draws text and a hairline in the bottom strip, and leaves the rest alone.
#[test]
fn the_status_bar_takes_rows_and_draws_its_text_in_the_bottom_strip() {
    let (on, off) = (Soft::new(&FontCfg::sized(PX), OPACITY), {
        let mut s = Soft::new(&FontCfg::sized(PX), OPACITY);
        s.set_ui(Ui { status: false, ..Ui::IDLE });
        s
    });
    let (cw, ch) = on.cell();
    let rows = |s: &Soft| session::grid_size(W, H, cw, ch, s.pad, s.top, s.bottom).1;
    assert!(rows(&on) < rows(&off), "{} rows with the bar, {} without", rows(&on), rows(&off));
    assert_eq!(on.bottom as usize, chrome::STATUS_H + (PX * 0.45).round() as usize);

    let frame = |name: &str| {
        let s = scenes().into_iter().find(|s| s.name == name).unwrap();
        let (fb, w, _) = render(&s);
        (fb, w)
    };
    let (with, w) = frame("rest");
    let (without, _) = frame("no-status");
    let (_, _, mb) = chrome::margins(false);
    let h = with.len() / w;
    let strip = (h - mb - chrome::STATUS_H)..(h - mb);
    let diff = |rows: std::ops::Range<usize>| rows.flat_map(|y| (0..w).map(move |x| y * w + x)).filter(|&i| with[i] != without[i]).count();
    assert!(diff(strip.clone()) > 800, "the strip has text and keycaps: {} pixels differ", diff(strip.clone()));
    // The two windows have different grids, so the area above the bar is not compared; the title bar is the same.
    assert_eq!(diff(20..chrome::TITLE_H - 4), 0, "the title bar does not depend on the status bar");
}
