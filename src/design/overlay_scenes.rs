//! The overlay kit's frames for the design board: each primitive in each state it has (a field empty, typed,
//! with a selection; a list unfiltered, filtered with its matches lit, scrolled, with nothing found; a menu;
//! a toast; a dialog), in English and Spanish, painted over the real window by the same `Stack` the app uses.
//! `design::frames` writes them next to the window's own scenes.
use std::time::{Duration, Instant};

use super::{H, W, Scene, render, scenes};
use crate::{
    chrome, commands,
    i18n::{self, Lang, Str, tr},
    input::sym,
    overlay::{Geo, Overlay, Stack},
    overlay_demo::{self, Dialog, Shape, picker, window_menu},
    picker::Picker,
    plat::{Key, Mods},
};

/// What a scene shows: an overlay built when it is drawn (the language is set by then), or a toast.
enum Show {
    Overlay(fn() -> Box<dyn Overlay>),
    Toast,
}

struct Def {
    name: &'static str,
    label: &'static str,
    lang: Lang,
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

fn shift(sym: u32) -> Key {
    Key { sym, mods: Mods { shift: true, ..Mods::default() }, ..Key::default() }
}

/// The three looks of a field with a text typed in each: a group's name, a filter, the palette's input.
fn typed(d: &mut Dialog) {
    d.field_mut(0).set("arcton");
    d.field_mut(1).set("arcton");
    d.field_mut(2).set("split columns");
}

/// The command palette over a window of three in a group and one apart, `query` typed and row `sel` selected. The
/// overview is listed as it will be with the overview screen (ADR 0027: the app lists it once it has one).
fn palette(query: &str, sel: usize) -> Box<dyn Overlay> {
    let (wins, groups) = commands::sample();
    let state = commands::State { wins: &wins, groups: &groups, cur: 0, panes: 1, overview: true };
    Box::new(Picker::new(commands::build(&state)).state(query, sel))
}

fn defs() -> Vec<Def> {
    let en = Lang::En;
    let d = |name, label, show| Def { name, label, lang: en, show };
    vec![
        d("field-empty", "Fields: empty, the placeholders, the first one focused", Show::Overlay(|| Box::new(Dialog::new(Shape::Fields).shown()))),
        d("field-typed", "Fields: text typed, the caret after it", Show::Overlay(|| {
            let mut d = Dialog::new(Shape::Fields).shown();
            typed(&mut d);
            Box::new(d)
        })),
        d("field-selected", "Fields: everything selected in the focused one", Show::Overlay(|| {
            let mut d = Dialog::new(Shape::Fields).shown();
            typed(&mut d);
            d.field_mut(0).select_all();
            Box::new(d)
        })),
        d("field-search", "Fields: the search box focused, two characters selected", Show::Overlay(|| {
            let mut d = Dialog::new(Shape::Fields).shown().focus(1);
            typed(&mut d);
            d.field_mut(1).key(&shift(sym::LEFT));
            d.field_mut(1).key(&shift(sym::LEFT));
            Box::new(d)
        })),
        d("field-long", "Fields: a text longer than the field shows its end, with the caret", Show::Overlay(|| {
            let mut d = Dialog::new(Shape::Fields).shown().focus(2);
            d.field_mut(2).set("a command that is far too long to fit the row of the palette's input, typed to the end");
            Box::new(d)
        })),
        d("list", "List: no query, under their headings, the first selected", Show::Overlay(|| Box::new(picker().state("", 0)))),
        d("list-filtered", "List: a query, ranked, the characters that matched lit, the group beside the shortcut", Show::Overlay(|| Box::new(picker().state("group", 0)))),
        d("list-scrolled", "List: more than fits, the last row selected, the scrollbar at the end", Show::Overlay(|| Box::new(picker().state("", 9)))),
        d("list-none", "List: nothing matches", Show::Overlay(|| Box::new(picker().state("zzzz", 0)))),
        d("menu", "Menu: shortcuts, a heading, groups with a check, a danger item, a disabled one, pointer over the second", Show::Overlay(|| Box::new(window_menu(300, 170).shown(Some(1))))),
        d("menu-danger", "Menu: the keyboard on the danger item", Show::Overlay(|| Box::new(window_menu(300, 170).shown(Some(9))))),
        d("toast", "Toast: a message and an action, at the bottom centre", Show::Toast),
        d("modal", "Modal frame: a dialog with a field, the scrim over the grid", Show::Overlay(|| {
            let mut d = Dialog::new(Shape::Group).shown();
            d.field_mut(0).set("arcton");
            Box::new(d)
        })),
        d("modal-empty", "Modal frame: the field empty, the pointer over the primary button", Show::Overlay(|| Box::new(Dialog::new(Shape::Group).shown().lit(1)))),
        d("palette-empty", "Command palette (Ctrl+K): no query, the actions under their heading, the first selected", Show::Overlay(|| palette("", 0))),
        d("palette-moved", "Command palette: two Down presses, the third row selected", Show::Overlay(|| palette("", 2))),
        d("palette-scrolled", "Command palette: the last row selected, the windows and groups reached by scrolling", Show::Overlay(|| palette("", usize::MAX))),
        d("palette-typed", "Command palette: \u{ab}group\u{bb} typed, ranked, the group of each row at the right, the matches lit", Show::Overlay(|| palette("group", 0))),
        d("palette-windows", "Command palette: a window's name typed, two-line rows with the directory and the group", Show::Overlay(|| palette("arcton", 1))),
        d("palette-none", "Command palette: nothing matches", Show::Overlay(|| palette("zzzz", 0))),
        Def { name: "palette-empty-es", label: "Command palette in Spanish: no query", lang: Lang::Es, show: Show::Overlay(|| palette("", 0)) },
        Def { name: "palette-moved-es", label: "Command palette in Spanish: the third row selected", lang: Lang::Es, show: Show::Overlay(|| palette("", 2)) },
        Def { name: "palette-typed-es", label: "Command palette in Spanish: \u{ab}grupo\u{bb} typed", lang: Lang::Es, show: Show::Overlay(|| palette("grupo", 0)) },
        Def { name: "palette-none-es", label: "Command palette in Spanish: nothing matches", lang: Lang::Es, show: Show::Overlay(|| palette("zzzz", 0)) },
        Def { name: "list-es", label: "List in Spanish: filtered", lang: Lang::Es, show: Show::Overlay(|| Box::new(picker().state("grupo", 0))) },
        Def { name: "menu-es", label: "Menu in Spanish", lang: Lang::Es, show: Show::Overlay(|| Box::new(window_menu(300, 170).shown(Some(0)))) },
        Def { name: "toast-es", label: "Toast in Spanish", lang: Lang::Es, show: Show::Toast },
        Def { name: "modal-es", label: "Modal frame in Spanish", lang: Lang::Es, show: Show::Overlay(|| Box::new(Dialog::new(Shape::Group).shown().lit(0))) },
    ]
}

/// Paints one scene over the window frame.
fn build(def: &Def) -> Frame {
    let base = Scene { lang: def.lang, ..scenes().remove(0) };
    let (mut fb, w, h) = render(&base);
    // The window's own render puts the language back; the overlay is built and drawn in this one.
    i18n::set(def.lang);
    let (ml, mt, _) = chrome::margins(false);
    let g = Geo { w: W as i32, h: H as i32, square: false };
    let (mut stack, t0) = (Stack::default(), Instant::now());
    match def.show {
        Show::Overlay(f) => stack.open(overlay_demo::TAG, f()),
        Show::Toast => stack.toast(tr(Str::OvToastClosed), Some(tr(Str::OvUndo)), overlay_demo::CMD_UNDO, t0),
    }
    // Two seconds in: every fade is over and a toast is at its full strength.
    stack.draw(&mut fb, (w, h), (ml, mt), g, t0 + Duration::from_secs(2));
    i18n::set(Lang::En);
    Frame { name: def.name, label: def.label, fb, w, h }
}

pub fn frames() -> Vec<Frame> {
    defs().iter().map(build).collect()
}

/// Every scene paints something over the window and nothing over the title bar, and every name is its own.
/// The Spanish ones are drawn only by `design::frames`: the language is one process-wide byte and a test
/// that switched it would change what the tests running beside it draw.
#[test]
fn every_overlay_scene_paints_over_the_window_and_leaves_the_title_bar_alone() {
    let (ml, mt, _) = chrome::margins(false);
    let plain = render(&scenes().remove(0));
    let mut names = std::collections::HashSet::new();
    for def in defs() {
        assert!(names.insert(def.name), "{} twice", def.name);
        if def.lang == Lang::Es {
            continue;
        }
        let f = build(&def);
        assert_eq!((f.w, f.h), (plain.1, plain.2));
        let changed = f.fb.iter().zip(&plain.0).filter(|(a, b)| a != b).count();
        assert!(changed > 3000, "{}: only {changed} pixels differ from the bare window", def.name);
        // The title bar is outside every overlay (the toast and the menus are well below it).
        let bar = |fb: &[u32]| (mt + 2..mt + chrome::TITLE_H - 2).flat_map(|y| fb[y * f.w + ml + 130..y * f.w + ml + 700].to_vec()).collect::<Vec<u32>>();
        assert_eq!(bar(&f.fb), bar(&plain.0), "{}: the title bar changed", def.name);
    }
}
