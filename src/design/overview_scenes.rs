//! The overview of windows on the design board: the same window the other scenes use, its panel of windows
//! beside the workspace, and the overview painted over the workspace by the same `Stack` the app uses, with the
//! terminals' text made up (`Fake` is the board's `overlay::Grid`). English and Spanish, the panel hidden, the
//! settings docked, a filter typed, the pointer over a card, the keyboard on another, the end of the list.
use std::time::{Duration, Instant};

use super::{H, OPACITY, PX, Scene, W, render, scenes};
use crate::{
    chrome::{self, Ui},
    config::FontCfg,
    i18n::{self, Lang},
    overlay::{Geo, Grid, Rect, Stack},
    overview::{self, Card, Group, Overview},
    paint::Paint,
    side::Status,
    soft::Soft,
};

/// What the board's window shows around the overview.
#[derive(Clone, Copy)]
enum Around {
    /// The panel of windows at the left, as in the `side` scene.
    Panel,
    /// The panel hidden: the overview has the whole width.
    Plain,
    /// The panel of windows and the settings docked: a narrow workspace.
    Docked,
}

impl Around {
    fn scene(self, lang: Lang) -> Scene {
        let named = |name: &str| scenes().into_iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no scene {name}"));
        let base = match self {
            Around::Panel => named("side"),
            Around::Plain => Scene { ui: Ui { side: 1, ..Ui::IDLE }, ..scenes().remove(0) },
            Around::Docked => named("prefs-with-side"),
        };
        Scene { lang, ..base }
    }

    /// The workspace in the window: what `Soft::workspace` says for these panels, and the status bar under it.
    fn area(self) -> Rect {
        let status = 28;
        let top = chrome::TITLE_H as i32;
        match self {
            Around::Panel => (220, top, W as i32 - 220, H as i32 - top - status),
            Around::Plain => (0, top, W as i32, H as i32 - top - status),
            Around::Docked => (220, top, W as i32 - 220 - crate::settings::panel_width(W) as i32, H as i32 - top - status),
        }
    }
}

/// The windows of the board's panel (arcton, servidor, pruebas, notas, ~) with what each says in its card.
fn windows() -> (Vec<Card>, Vec<Group>) {
    let card = |id, title: &str, group, status, unread, panes, meta: &str| Card {
        id,
        group,
        title: title.into(),
        meta: meta.into(),
        status,
        unread,
        panes,
        current: id == 1,
        text_hit: false,
    };
    let cards = vec![
        card(1, "arcton", Some(0), Status::Idle, false, 1, "~/lab/apps/arcton \u{b7} main"),
        card(2, "servidor", Some(0), Status::Running, true, 1, "~/lab/apps/arcton \u{b7} main"),
        card(3, "pruebas", Some(0), Status::Failed, true, 2, "~/lab/apps/arcton \u{b7} main"),
        card(4, "notas", Some(1), Status::Idle, false, 1, "~/docs \u{b7} main"),
        card(5, "~", None, Status::Idle, false, 1, "~"),
    ];
    (cards, vec![Group { name: "arcton".into(), color: 1 }, Group { name: "docs".into(), color: 3 }])
}

/// Ten windows in two groups and none (the reference's capture without its panel).
fn many() -> (Vec<Card>, Vec<Group>) {
    let (mut cards, mut groups) = windows();
    let more = [(6, "api", 2, Status::Running), (7, "tests", 2, Status::Idle), (8, "logs", 2, Status::Idle), (9, "scratch", 5, Status::Idle), (10, "git", 5, Status::Failed)];
    for (id, title, group, status) in more {
        let g = (group != 5).then_some(2u8);
        cards.push(Card { id, group: g, title: title.into(), meta: "~/lab/apps/api \u{b7} dev".into(), status, panes: 1, ..Card::default() });
    }
    groups.push(Group { name: "api".into(), color: 2 });
    // The panel keeps a group's windows together: the groups first, the windows in none last.
    cards.sort_by_key(|c| c.group.map_or(usize::MAX, usize::from));
    (cards, groups)
}

/// The last rows of each window's terminal, and whether a command runs in it.
fn terminal(id: u32) -> (&'static str, bool) {
    match id {
        1 => ("$ git log --oneline\na3f9c21 (HEAD -> main) adds the overview of windows\n91be07d improves the terminal's ui\n5c2d8e4 first structure of axon\n$", false),
        2 | 6 => ("$ dev\nVITE v6.0.3  ready in 312 ms\n  \u{279c}  Local:   http://localhost:5173/\n21:42:26 [hmr] updated src/terminal.css\n21:42:31 [hmr] updated src/tabs.ts\n21:42:36 [hmr] updated src/overview.ts\n21:42:41 [hmr] updated src/settings.ts", true),
        3 | 7 => ("bun test v1.1.38\n\u{2713} renders the prompt chips [2.1ms]\n\u{2713} respects the configured order [0.8ms]\n\u{2717} saves the settings to the file [3.4ms]\n  error: expected 16 but got 14\n 2 pass\n 1 fail\n$", false),
        4 | 9 => ("$", false),
        5 | 10 => ("$ ls\nDocuments  Downloads  lab  notes.txt\n$ git status\nOn branch main\nnothing to commit\n$", false),
        _ => ("$ tail -f app.log\n21:40:02 GET /api/users 200 12ms\n21:40:05 GET /api/orders 200 31ms\n21:40:09 POST /api/login 401 8ms\n$", false),
    }
}

/// The board's `overlay::Grid`: made-up text, painted with a `Soft`'s font (the app's reads its terminals).
struct Fake {
    soft: Soft,
    scratch: Vec<u8>,
}

impl Fake {
    /// A renderer for a `w` x `h` frame, the size the scene is drawn in.
    fn new(w: usize, h: usize, ui: Ui) -> Fake {
        let mut soft = Soft::new(&FontCfg::sized(PX), OPACITY);
        soft.set_ui(ui);
        // Only the framebuffer's size is wanted from the call; its picture is thrown away.
        soft.begin_frame(&mut vec![0u32; w * h], w, h, 1, 1);
        Fake { soft, scratch: Vec::new() }
    }
}

impl Grid for Fake {
    fn tail(&mut self, id: u32, rows: usize, out: &mut String) -> bool {
        let (text, running) = terminal(id);
        let lines: Vec<&str> = text.lines().collect();
        out.push_str(&lines[lines.len().saturating_sub(rows)..].join("\n"));
        running
    }

    fn text(&mut self, p: &mut Paint, x: i32, cy: i32, s: &str, scale: f32, c: [f32; 3]) {
        let ch = |v: f32| (v * 255.0 + 0.5) as u8;
        let m = self.soft.small(scale);
        self.soft.paint_text(p.fb, (x + p.ox) as usize, (cy + p.oy) as usize, s.chars(), crate::cell::pack(ch(c[0]), ch(c[1]), ch(c[2]), 255), false, &m, &mut self.scratch);
    }

    fn metrics(&self, scale: f32) -> (i32, i32) {
        let m = self.soft.small(scale);
        (m.cell_w as i32, m.cell_h as i32)
    }
}

struct Def {
    name: &'static str,
    label: &'static str,
    lang: Lang,
    around: Around,
    build: fn(Rect) -> Overview,
}

/// The frame of a scene: the whole window, premultiplied `Argb8888`.
pub struct Frame {
    pub name: &'static str,
    pub label: &'static str,
    pub fb: Vec<u32>,
    pub w: usize,
    pub h: usize,
}

fn open(area: Rect) -> Overview {
    let (cards, groups) = windows();
    Overview::new(cards, groups, area).shown()
}

fn defs() -> Vec<Def> {
    let d = |name, label, around, build| Def { name, label, lang: Lang::En, around, build };
    vec![
        d("overview", "Overview (Alt+O): the workspace under the title bar, the panel of windows beside it; a section per group, the windows in none, two columns of cards; the window on screen has the amber border", Around::Panel, open),
        d("overview-hover", "Overview: the pointer over a card lifts it, lights its border and shows its cross", Around::Panel, |a| open(a).state("", None, Some(3), 0)),
        d("overview-cross", "Overview: the pointer over a card's cross", Around::Panel, |a| open(a).over_close(3)),
        d("overview-keys", "Overview: the keyboard on another card (arrows, Tab), the list scrolled to show it", Around::Panel, |a| open(a).state("", Some(4), None, i32::MIN)),
        d("overview-filter", "Overview: a filter typed (`hmr`) narrows the cards to the windows whose name, directory, group or terminal text has it", Around::Panel, |a| {
            let (mut cards, groups) = windows();
            cards.iter_mut().filter(|c| c.id == 2).for_each(|c| c.text_hit = true);
            Overview::new(cards, groups, a).shown().state("hmr", None, None, 0)
        }),
        d("overview-none", "Overview: nothing matches the filter", Around::Panel, |a| open(a).state("hmrzz", None, None, 0)),
        d("overview-end", "Overview: scrolled to the end, the dashed tile that makes a group (pointer over it)", Around::Panel, |a| open(a).state("", None, None, i32::MAX).over_button(3)),
        d("overview-header", "Overview: the pointer over the header's new window button", Around::Panel, |a| open(a).over_button(1)),
        d("overview-wide", "Overview without the panel (Alt+B): ten windows in three groups, three columns", Around::Plain, |a| {
            let (cards, groups) = many();
            Overview::new(cards, groups, a).shown()
        }),
        d("overview-wide-end", "Overview without the panel, scrolled to the end", Around::Plain, |a| {
            let (cards, groups) = many();
            Overview::new(cards, groups, a).shown().state("", None, None, i32::MAX)
        }),
        d("overview-narrow", "Overview beside the panel and the settings: one column, the header keeps the filter and the cross", Around::Docked, open),
        Def { name: "overview-es", label: "Overview in Spanish", lang: Lang::Es, around: Around::Plain, build: |a| {
            let (cards, groups) = windows();
            Overview::new(cards, groups, a).shown().state("", None, Some(2), 0)
        } },
        Def { name: "overview-es-end", label: "Overview in Spanish: the tile and the empty filter message", lang: Lang::Es, around: Around::Panel, build: |a| open(a).state("", None, None, i32::MAX).over_button(3) },
        Def { name: "overview-es-none", label: "Overview in Spanish: nothing matches", lang: Lang::Es, around: Around::Panel, build: |a| open(a).state("hmrzz", None, None, 0) },
    ]
}

fn build(def: &Def) -> Frame {
    let base = def.around.scene(def.lang);
    let ui = base.ui;
    let (mut fb, w, h) = render(&base);
    // The window's own render puts the language back; the overview is built and drawn in this one.
    i18n::set(def.lang);
    let (ml, mt, _) = chrome::margins(false);
    let g = Geo { w: W as i32, h: H as i32, square: false };
    let (mut stack, t0) = (Stack::default(), Instant::now());
    stack.open(overview::TAG, Box::new((def.build)(def.around.area())));
    let mut grid = Fake::new(w, h, ui);
    // Two seconds in: the fade is over.
    stack.draw_with(&mut fb, (w, h), (ml, mt), g, t0 + Duration::from_secs(2), &mut grid);
    i18n::set(Lang::En);
    Frame { name: def.name, label: def.label, fb, w, h }
}

pub fn frames() -> Vec<Frame> {
    defs().iter().map(build).collect()
}

/// Every scene paints over the workspace and leaves the panel of windows, the title bar and the status bar
/// as the plain window has them. The Spanish ones are drawn only by `design::frames` (the language is one
/// process-wide byte).
#[test]
fn every_overview_scene_paints_over_its_workspace_only() {
    let mut names = std::collections::HashSet::new();
    for def in defs() {
        assert!(names.insert(def.name), "{} twice", def.name);
        if def.lang == Lang::Es {
            continue;
        }
        let plain = render(&def.around.scene(def.lang));
        let f = build(&def);
        assert_eq!((f.w, f.h), (plain.1, plain.2));
        let (ml, mt, _) = chrome::margins(false);
        let (ax, ay, aw, ah) = def.around.area();
        let (mut inside, mut outside) = (0, 0);
        for y in 0..f.h {
            for x in 0..f.w {
                let (wx, wy) = (x as i32 - ml as i32, y as i32 - mt as i32);
                let in_area = wx >= ax && wx < ax + aw && wy >= ay && wy < ay + ah;
                if f.fb[y * f.w + x] != plain.0[y * f.w + x] {
                    if in_area { inside += 1 } else { outside += 1 }
                }
            }
        }
        assert_eq!(outside, 0, "{}: the overview changed pixels beyond its workspace", def.name);
        assert!(inside > 40_000, "{}: only {inside} pixels differ inside the workspace", def.name);
    }
}
