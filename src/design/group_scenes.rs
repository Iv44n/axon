//! The design board's frames of the window list's screens ([0029](../../knowledge/decisions/0029-group-dialog-menus-rename-toasts.md)):
//! the group dialog (new, with its error lines, with the windows ticked, the development template, edit mode, in
//! Spanish), the context menus of a window, a group, a pane and the empty list, a window's name being edited in its
//! row or tab, and the toasts of a close and of a group made. Each is painted over the real window and panel by
//! the same `Stack` the app uses; the menus and dialogs are the real ones with made-up windows.
//! `design::frames` writes them next to the others.
use std::time::{Duration, Instant};

use super::{H, W, Scene, render, scenes};
use crate::{
    chrome,
    config::TabsPos,
    ctxmenu::{self, GroupEntry},
    group_dialog::{self, Answer, GroupDialog, Init, WinRow, Zone},
    groups::{FormError, Template, fill},
    i18n::{self, Lang, Str, tr},
    overlay::{Geo, Overlay, Stack},
    side::Part,
};

/// What a scene shows over the window.
enum Show {
    Overlay(fn() -> Box<dyn Overlay>),
    Toast(fn() -> (String, Option<&'static str>)),
    /// Only what the scene itself draws (a field in a row).
    Plain,
}

struct Def {
    name: &'static str,
    label: &'static str,
    lang: Lang,
    /// The window scene of `design.rs` it is painted on, and what it changes in it.
    base: &'static str,
    tweak: fn(Scene) -> Scene,
    show: Show,
}

/// One rendered scene: the whole frame, premultiplied `Argb8888`.
pub struct Frame {
    pub name: &'static str,
    pub label: &'static str,
    pub fb: Vec<u32>,
    pub w: usize,
    pub h: usize,
}

/// The windows the dialog lists: the reference's session (a group of three, the home folder, the window on screen).
fn windows() -> Vec<WinRow> {
    let row = |id, title: &str, place: &str, group: Option<(&str, u8)>| WinRow { id, title: title.into(), place: place.into(), group: group.map(|(n, c)| (n.to_string(), c)) };
    vec![
        row(1, "arcton", "~/lab/apps/arcton", Some(("arcton", 1))),
        row(2, "servidor", "~/lab/apps/arcton", Some(("arcton", 1))),
        row(3, "pruebas", "~/lab/apps/arcton", Some(("arcton", 1))),
        row(4, "~", "~", None),
        row(5, "arcton", "~/lab/apps/arcton", None),
        row(6, "notas", "~/notas", Some(("docs", 3))),
    ]
}

fn new_group(picked: &[u64]) -> GroupDialog {
    let init = Init { edit: None, name: "arcton 2".into(), color: 0, wins: windows(), picked: picked.to_vec(), taken: vec!["arcton".into(), "docs".into()] };
    GroupDialog::new(init, Answer::default()).shown()
}

fn edit_group() -> GroupDialog {
    let init = Init { edit: Some(1), name: "arcton".into(), color: 1, wins: windows(), picked: vec![], taken: vec!["docs".into()] };
    GroupDialog::new(init, Answer::default()).shown()
}

fn groups() -> Vec<GroupEntry> {
    vec![GroupEntry { id: 1, name: "arcton".into(), color: 1 }, GroupEntry { id: 2, name: "docs".into(), color: 3 }]
}

fn defs() -> Vec<Def> {
    let d = |name, label, base, tweak: fn(Scene) -> Scene, show| Def { name, label, lang: Lang::En, base, tweak, show };
    let plain: fn(Scene) -> Scene = |s| s;
    let row3: fn(Scene) -> Scene = |s| Scene { side: Some((TabsPos::Left, Some(Part::Row(3)))), ..s };
    let row1: fn(Scene) -> Scene = |s| Scene { side: Some((TabsPos::Left, Some(Part::Row(1)))), ..s };
    let head0: fn(Scene) -> Scene = |s| Scene { side: Some((TabsPos::Left, Some(Part::Head(0)))), ..s };
    let pane: fn(Scene) -> Scene = |s| Scene { side: Some((TabsPos::Left, Some(Part::Pane(0, 1)))), ..s };
    vec![
        d("group-dialog", "Group dialog (Alt+G): the name selected, the eight colours, the windows (the one on screen ticked, out of view), the templates, the summary", "side", plain, Show::Overlay(|| Box::new(new_group(&[5])))),
        d("group-dialog-picked", "Group dialog: two windows ticked, the list scrolled to its end with its scrollbar, the keyboard on the list", "side", plain, Show::Overlay(|| Box::new(new_group(&[4, 5]).at(Zone::Members).scrolled_list(999)))),
        d("group-dialog-empty", "Group dialog: an empty name, the error line under the field (the card is as tall as the window, the body scrolls)", "side", plain, Show::Overlay(|| Box::new(new_group(&[5]).with_name("").with_error(FormError::NameEmpty)))),
        d("group-dialog-taken", "Group dialog: a name another group has", "side", plain, Show::Overlay(|| Box::new(new_group(&[5]).with_name("arcton").with_error(FormError::NameTaken)))),
        d("group-dialog-nothing", "Group dialog: no window and no template, the error under the windows' list, the pointer over Create", "side", plain, Show::Overlay(|| Box::new(new_group(&[]).with_template(Template::None).with_error(FormError::NothingChosen).over(group_dialog::Hover::Create)))),
        d("group-dialog-dev", "Group dialog: the development template, the cyan colour, a name typed; hovering a template", "side", plain, Show::Overlay(|| Box::new(new_group(&[]).with_template(Template::Dev).with_color(2).with_name("api").over(group_dialog::Hover::Tpl(1))))),
        d("group-dialog-edit", "Edit group: only the name and the colour, Save", "side", plain, Show::Overlay(|| Box::new(edit_group()))),
        Def { name: "group-dialog-es", label: "Group dialog in Spanish", lang: Lang::Es, base: "side", tweak: plain, show: Show::Overlay(|| Box::new(new_group(&[5]))) },
        Def { name: "group-dialog-edit-es", label: "Edit group in Spanish", lang: Lang::Es, base: "side", tweak: plain, show: Show::Overlay(|| Box::new(edit_group())) },
        d("menu-window", "Window menu (right click on a row): rename, duplicate, split, the groups, new group, close; the first item lit", "side", row3, Show::Overlay(|| Box::new(ctxmenu::window_menu((80, 300), &groups(), None, true).shown(Some(0))))),
        d("menu-window-grouped", "Window menu of a window in a group: its group checked, Remove from group, the danger items", "side", row1, Show::Overlay(|| Box::new(ctxmenu::window_menu((80, 150), &groups(), Some(1), true).shown(Some(11))))),
        d("menu-group", "Group menu (right click on its header): edit, fold, new window, dissolve, close", "side", head0, Show::Overlay(|| Box::new(ctxmenu::group_menu((80, 100), false).shown(Some(0))))),
        d("menu-pane", "Pane menu (right click on a pane's row): go to it, close it", "panes-2", pane, Show::Overlay(|| Box::new(ctxmenu::pane_menu((110, 158)).shown(Some(0))))),
        d("menu-strip", "Menu of the empty part of the list: a new window, a new group", "side", plain, Show::Overlay(|| Box::new(ctxmenu::strip_menu((100, 520), false).shown(Some(1))))),
        Def { name: "menu-window-es", label: "Window menu in Spanish", lang: Lang::Es, base: "side", tweak: row3, show: Show::Overlay(|| Box::new(ctxmenu::window_menu((80, 300), &groups(), None, true).shown(Some(0)))) },
        Def { name: "menu-group-es", label: "Group menu in Spanish", lang: Lang::Es, base: "side", tweak: head0, show: Show::Overlay(|| Box::new(ctxmenu::group_menu((80, 100), true).shown(Some(1)))) },
        d("rename-row", "Rename (double click on a row, F2): the name in a field in the row, all selected, the cross stays", "side", |s| Scene { side: Some((TabsPos::Left, Some(Part::Row(0)))), edit: Some((0, "arcton", true)), ..s }, Show::Plain),
        d("rename-typed", "Rename: a name typed, the caret after it", "side", |s| Scene { side: Some((TabsPos::Left, None)), edit: Some((0, "scratch", false)), ..s }, Show::Plain),
        d("rename-tab", "Rename in a tab at the top", "tabs-top", |s| Scene { edit: Some((1, "servidor", true)), ..s }, Show::Plain),
        d("rename-long", "Rename: a name longer than the field shows its end, with the caret", "side", |s| Scene { side: Some((TabsPos::Left, Some(Part::Row(0)))), edit: Some((0, "a long name for a window", false)), ..s }, Show::Plain),
        d("toast-closed", "Toast after closing a window: the message and the amber Undo button", "side", plain, Show::Toast(|| (fill(tr(Str::ToastClosed), &["servidor"]), Some(tr(Str::OvUndo))))),
        d("toast-group", "Toast after making a group", "side", plain, Show::Toast(|| (fill(tr(Str::ToastGroupMade), &["api", "4", tr(Str::WinMany)]), None))),
        Def { name: "toast-closed-es", label: "Toast of a close in Spanish", lang: Lang::Es, base: "side", tweak: plain, show: Show::Toast(|| (fill(tr(Str::ToastClosed), &["servidor"]), Some(tr(Str::OvUndo)))) },
        Def { name: "toast-moved-es", label: "Toast of a window moved to a group, in Spanish", lang: Lang::Es, base: "side", tweak: plain, show: Show::Toast(|| (fill(tr(Str::ToastMoved), &["notas", "arcton"]), None)) },
    ]
}

/// Paints one scene over the window frame.
fn build(def: &Def) -> Frame {
    let base = scenes().into_iter().find(|s| s.name == def.base).expect("a scene of the board");
    let scene = Scene { lang: def.lang, ..(def.tweak)(base) };
    let (mut fb, w, h) = render(&scene);
    // The window's own render puts the language back; the overlay is built and drawn in this one.
    i18n::set(def.lang);
    let (ml, mt, _) = chrome::margins(false);
    let g = Geo { w: W as i32, h: H as i32, square: false };
    let (mut stack, t0) = (Stack::default(), Instant::now());
    match &def.show {
        Show::Overlay(f) => stack.open(group_dialog::TAG, f()),
        Show::Toast(f) => {
            let (msg, action) = f();
            stack.toast(&msg, action, 1, t0);
        }
        Show::Plain => {}
    }
    // Two seconds in: every fade is over and a toast is at its full strength.
    stack.draw(&mut fb, (w, h), (ml, mt), g, t0 + Duration::from_secs(2));
    i18n::set(Lang::En);
    Frame { name: def.name, label: def.label, fb, w, h }
}

pub fn frames() -> Vec<Frame> {
    defs().iter().map(build).collect()
}

/// Every scene paints something its window scene does not, every name is its own, and the dialogs and menus leave the
/// title bar alone. The Spanish ones are drawn only by `design::frames`: the language is one process-wide byte and a
/// test that switched it would change what the tests running beside it draw.
#[test]
fn every_group_scene_paints_something_over_its_window_scene() {
    let (ml, mt, _) = chrome::margins(false);
    let mut names = std::collections::HashSet::new();
    for def in defs() {
        assert!(names.insert(def.name), "{} twice", def.name);
        if def.lang == Lang::Es {
            continue;
        }
        let base = scenes().into_iter().find(|s| s.name == def.base).unwrap();
        let plain = render(&Scene { edit: None, ..(def.tweak)(base) });
        let f = build(&def);
        assert_eq!((f.w, f.h), (plain.1, plain.2));
        let changed = f.fb.iter().zip(&plain.0).filter(|(a, b)| a != b).count();
        assert!(changed > 1500, "{}: only {changed} pixels differ from the window", def.name);
        if matches!(def.show, Show::Overlay(_)) {
            let bar = |fb: &[u32]| (mt + 2..mt + chrome::TITLE_H - 2).flat_map(|y| fb[y * f.w + ml + 130..y * f.w + ml + 700].to_vec()).collect::<Vec<u32>>();
            assert_eq!(bar(&f.fb), bar(&plain.0), "{}: the title bar changed", def.name);
        }
    }
}
