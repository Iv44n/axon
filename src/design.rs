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
    config::{Chip, FontCfg, Smoothing},
    context::Context,
    history::History,
    i18n::{self, Lang},
    palette, session,
    settings::{self, Settings},
    soft::{Soft, Target},
    view::View,
};

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

struct Scene {
    name: &'static str,
    label: &'static str,
    group: &'static str,
    ui: Ui,
    opacity: f32,
    font: FontCfg,
    /// Open the settings screen over the grid, with this row selected and this under the pointer.
    settings: Option<(usize, settings::Hit, &'static str)>,
    /// The settings page shown.
    page: settings::Page,
    /// A chip held mid-drag in the settings screen, and where the pointer is on the panel.
    hold: Option<(Chip, (i32, i32))>,
    /// The blur switch: on, and whether the compositor offers blur.
    blur: (bool, bool),
    title: &'static str,
    input: &'static str,
    /// The language of the interface texts.
    lang: Lang,
    /// Select `len` cells of this grid line, starting at column 1.
    select: Option<(i32, usize)>,
    /// The prompt chips: what they say, and which are listed. The last prompt of `input` is marked
    /// the way the shell's hook marks it (`prompt`).
    bar: Option<(Context, Vec<Chip>)>,
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

const TITLE: &str = "ivan@axon: ~/lab/apps/axon";

fn scenes() -> Vec<Scene> {
    let base = |name, label, group, ui| Scene {
        name,
        label,
        group,
        ui,
        opacity: OPACITY,
        font: FontCfg::sized(PX),
        settings: None,
        page: settings::Page::Font,
        blur: (false, true),
        hold: None,
        title: TITLE,
        input: SHELL,
        select: None,
        bar: None,
        lang: Lang::En,
    };
    let bar = |chips: &[Chip]| Some((sample_context(), chips.to_vec()));
    let hooked: &'static str = Box::leak(with_hook("").into_boxed_str());
    let typing: &'static str = Box::leak(with_hook("git commit -m \"wire the chips to the prompt\"").into_boxed_str());
    let bar_cursor: &'static str = Box::leak(format!("{SHELL}\x1b[6 q").into_boxed_str());
    let underline_cursor: &'static str = Box::leak(format!("{SHELL}\x1b[4 q").into_boxed_str());
    let font = |weight, letter_spacing, line_height| FontCfg { weight, letter_spacing, line_height, ..FontCfg::sized(PX) };
    vec![
        base("rest", "Focused, at rest", "window", Ui::IDLE),
        base("hover", "Pointer over the buttons (close)", "states", Ui { hover: 1, ..Ui::IDLE }),
        base("pressed", "Maximize held down", "states", Ui { hover: 3, down: true, ..Ui::IDLE }),
        base("unfocused", "Unfocused", "states", Ui { focused: false, ..Ui::IDLE }),
        base("maximized", "Maximized or tiled (square, no shadow)", "states", Ui { maximized: true, ..Ui::IDLE }),
        Scene { title: "~: palette", input: COLORS, select: Some((10, 13)), ..base("colors", "Palette, attributes and selection", "colour", Ui::IDLE) },
        Scene { opacity: 0.80, ..base("opacity-80", "Opacity 0.80", "opacity", Ui::IDLE) },
        Scene { opacity: 0.88, ..base("opacity-88", "Opacity 0.88 (default)", "opacity", Ui::IDLE) },
        Scene { opacity: 0.94, ..base("opacity-94", "Opacity 0.94", "opacity", Ui::IDLE) },
        base("gear-hover", "Pointer over the settings gear", "states", Ui { hover: 4, ..Ui::IDLE }),
        Scene { settings: Some((0, settings::Hit::Panel, "")), font: FontCfg { family: Some("JetBrains Mono".into()), ..FontCfg::sized(PX) }, ..base("settings", "Settings screen", "settings", Ui::IDLE) },
        Scene { settings: Some((2, settings::Hit::Slider(settings::Row::Weight, 0.4), "")), ..base("settings-slider", "Dragging the weight slider", "settings", Ui::IDLE) },
        Scene { settings: Some((4, settings::Hit::Panel, "-1.5")), ..base("settings-typing", "Typing a number into a box", "settings", Ui::IDLE) },
        Scene { settings: Some((6, settings::Hit::Save, "")), font: font(500, 1.0, 1.0), opacity: 0.80, ..base("settings-save", "Pointer over Save, other values changed", "settings", Ui::IDLE) },
        Scene { bar: bar(&Chip::ALL), input: hooked, ..base("chips", "Every chip: the longest are cut, then the last dropped", "prompt chips", Ui::IDLE) },
        Scene { bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes]), input: typing, ..base("chips-few", "Four chips, a command being typed", "prompt chips", Ui::IDLE) },
        Scene { bar: bar(&Chip::ALL), input: hooked, font: FontCfg::sized(20.0), ..base("chips-large", "At 20 px", "prompt chips", Ui::IDLE) },
        Scene { bar: bar(&[Chip::Dir, Chip::Branch]), input: typing, opacity: 0.80, ..base("chips-80", "Opacity 0.80", "prompt chips", Ui::IDLE) },
        Scene { ..base("chips-off", "Chips off: the prompt as the shell draws it", "prompt chips", Ui::IDLE) },
        Scene { input: bar_cursor, ..base("cursor-bar", "Cursor as a bar (DECSCUSR 6, or [input] cursor = \"bar\")", "window", Ui::IDLE) },
        Scene { input: underline_cursor, ..base("cursor-underline", "Cursor as an underline", "window", Ui::IDLE) },
        base("no-status", "Status bar off ([window] status_bar = false)", "window", Ui { status: false, ..Ui::IDLE }),
        Scene { settings: Some((7, settings::Hit::Toggle(settings::Row::Blur), "")), blur: (true, false), opacity: 0.80, ..base("settings-blur", "Blur on, but the compositor (GNOME) offers none", "settings", Ui::IDLE) },
        Scene { settings: Some((0, settings::Hit::Tab(settings::Page::Input), "")), page: settings::Page::Input, ..base("settings-input", "Settings, Input page (bar off)", "settings", Ui::IDLE) },
        Scene {
            settings: Some((2, settings::Hit::Chip(Chip::Dir), "")),
            page: settings::Page::Input,
            bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes]),
            input: hooked,
            ..base("settings-chips", "Input page: bar on, four chips, pointer over dir", "settings", Ui::IDLE)
        },
        Scene { lang: Lang::Es, settings: Some((0, settings::Hit::Panel, "")), ..base("settings-es", "Settings in Spanish: Font page", "language", Ui::IDLE) },
        Scene {
            lang: Lang::Es,
            settings: Some((2, settings::Hit::Chip(Chip::Dir), "")),
            page: settings::Page::Input,
            bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes]),
            input: hooked,
            ..base("settings-es-input", "Settings in Spanish: Input page", "language", Ui::IDLE)
        },
        Scene { lang: Lang::Es, bar: bar(&Chip::ALL), input: hooked, ..base("chips-es", "Status bar hints in Spanish", "language", Ui::IDLE) },
        Scene { font: FontCfg { smoothing: Smoothing::Gray, ..FontCfg::sized(PX) }, ..base("text-gray", "Grey smoothing, hinted", "font", Ui::IDLE) },
        Scene { font: FontCfg { hinting: false, ..FontCfg::sized(PX) }, ..base("text-unhinted", "LCD smoothing, no hinting", "font", Ui::IDLE) },
        Scene { font: FontCfg { smoothing: Smoothing::Gray, hinting: false, ..FontCfg::sized(PX) }, ..base("text-plain", "Grey smoothing, no hinting (how it was)", "font", Ui::IDLE) },
        Scene {
            settings: Some((2, settings::Hit::Panel, "")),
            page: settings::Page::Input,
            bar: bar(&[Chip::Time, Chip::Dir, Chip::Branch, Chip::Changes]),
            input: hooked,
            hold: Some((Chip::Branch, (330, 190))),
            ..base("settings-chip-drag", "Input page: a chip held mid-drag", "settings", Ui::IDLE)
        },
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
    soft.set_title(s.title);
    soft.set_ui(s.ui);
    let (cw, ch) = soft.cell();
    let (cols, rows) = session::grid_size(W, H, cw, ch, soft.pad, soft.top, soft.bottom);
    let mut term = Term::new(Config::default(), &session::Size { cols, rows }, VoidListener);
    let mut p: Processor = Processor::new();
    p.advance(&mut term, s.input.as_bytes());
    if let Some((line, len)) = s.select {
        let mut sel = Selection::new(SelectionType::Simple, Point::new(Line(line), Column(1)), Side::Left);
        sel.update(Point::new(Line(line), Column(len)), Side::Right);
        term.selection = Some(sel);
    }
    let (ml, mt, mb) = chrome::margins(s.ui.maximized);
    let (w, h) = (W as usize + 2 * ml, H as usize + mt + mb);
    let mut fb = vec![0u32; w * h];
    soft.set_status("zsh", "main", cols, rows);
    let (ctx, chips) = s.bar.clone().unwrap_or_default();
    bar::Chips::default().frame(s.bar.is_some(), true, &demo_history(&ctx), &chips, &mut term, &mut View::new(), &mut soft, &mut fb, (w, h), s.ui.focused);
    if let Some((sel, hover, typing)) = s.settings {
        // Fixed fonts, not whatever is installed: the board must look the same on every machine.
        let families = vec![("DejaVu Sans Mono".to_string(), "/x/a.ttf".into()), ("JetBrains Mono".to_string(), "/x/b.ttf".into())];
        let input = match &s.bar {
            Some((_, chips)) => crate::config::InputCfg { inline: true, chips: chips.clone(), clock24: false, ..Default::default() },
            None => crate::config::InputCfg::default(),
        };
        let mut panel = Settings::new(&s.font, s.opacity, families).with_input(&input).with_blur(s.blur.0, s.blur.1).with_ui_font();
        panel.page = s.page;
        if let Some((chip, at)) = s.hold {
            panel.hold(chip, at);
        }
        (panel.sel, panel.hover) = (sel, hover);
        for c in typing.chars() {
            panel.key(&crate::xkb::Key { text: c.to_string(), ..Default::default() });
        }
        panel.draw(&mut fb, w, h, (ml, mt), (W, H), s.ui.maximized);
        panel.draw_preview(&soft, &ctx, &mut fb, (W, H));
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
        soft.set_title(title);
        soft.set_ui(ui);
        view.build(&mut term, ui.focused, &mut Target { soft: &mut soft, fb: &mut fb, w, h });

        let mut fresh = Soft::new(&FontCfg::sized(PX), OPACITY);
        let mut fresh_term = new(&fresh);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut fresh_term, input.as_bytes());
        fresh.set_title(title);
        fresh.set_ui(ui);
        let mut full = vec![0u32; w * h];
        View::new().build(&mut fresh_term, ui.focused, &mut Target { soft: &mut fresh, fb: &mut full, w, h });
        let diff = fb.iter().zip(&full).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 0, "step {n}: {diff} pixels differ from a full repaint");
    }
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
