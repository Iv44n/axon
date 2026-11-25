//! The context menus of the window list ([0029](../knowledge/decisions/workspace/0029-group-dialog-menus-rename-toasts.md)),
//! after the reference's `tabMenu`, `groupMenu`, `paneMenu` and `stripMenu`: what each has, in which order, with
//! which shortcut and which danger colour. They are `menu::Menu`s (the kit's overlay); this module only decides
//! their items and numbers the commands, so the app and the design board build the same menus.
//!
//! A menu's command ids are small numbers (`Cmd::id`); the group a window is moved to is part of the id
//! (`MOVE_BASE + group id`), so a menu needs no list of targets next to it. What the menu is about (which window,
//! which group, which pane) is the app's: it keeps it while the menu is open (`wlapp/groups_ui.rs`).
use crate::{
    i18n::{Str, tr},
    menu::{self, Menu},
};

/// The tag the menus are opened under on the overlay stack.
pub const TAG: u32 = 0x4d45_4e55;

/// A menu's commands. `MoveTo` carries the id of the group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    // A window.
    Rename,
    Duplicate,
    SplitCols,
    SplitRows,
    MoveTo(u32),
    NewGroupWith,
    RemoveFromGroup,
    CloseWin,
    CloseOthers,
    // A group.
    EditGroup,
    ToggleFold,
    NewWinInGroup,
    Dissolve,
    CloseGroup,
    // A pane.
    GoPane,
    ClosePane,
    // The empty part of the list.
    NewWin,
    NewGroup,
    Overview,
}

/// The ids that are not `MoveTo`, in the order of `Cmd`; `MoveTo` takes the numbers from `MOVE_BASE` up.
const FIXED: [Cmd; 18] = [
    Cmd::Rename,
    Cmd::Duplicate,
    Cmd::SplitCols,
    Cmd::SplitRows,
    Cmd::NewGroupWith,
    Cmd::RemoveFromGroup,
    Cmd::CloseWin,
    Cmd::CloseOthers,
    Cmd::EditGroup,
    Cmd::ToggleFold,
    Cmd::NewWinInGroup,
    Cmd::Dissolve,
    Cmd::CloseGroup,
    Cmd::GoPane,
    Cmd::ClosePane,
    Cmd::NewWin,
    Cmd::NewGroup,
    Cmd::Overview,
];
const MOVE_BASE: u32 = 1000;

impl Cmd {
    pub fn id(self) -> u32 {
        match self {
            Cmd::MoveTo(g) => MOVE_BASE + g,
            c => 1 + FIXED.iter().position(|&f| f == c).unwrap_or(0) as u32,
        }
    }

    pub fn from_id(id: u32) -> Option<Cmd> {
        match id {
            0 => None,
            n if n >= MOVE_BASE => Some(Cmd::MoveTo(n - MOVE_BASE)),
            n => FIXED.get(n as usize - 1).copied(),
        }
    }
}

/// A group as a window's menu lists it.
pub struct GroupEntry {
    pub id: u32,
    pub name: String,
    pub color: u8,
}

fn item(m: Menu, label: &str, keys: &str, c: Cmd, flags: u8) -> Menu {
    m.item(label, keys, c.id(), flags)
}

/// The menu of a window: rename, duplicate, split, the groups (a check on the window's own), a new group around
/// it, out of its group, close it, close the others (`others`: there is another window to close).
pub fn window_menu(at: (i32, i32), groups: &[GroupEntry], member: Option<u32>, others: bool) -> Menu {
    let mut m = Menu::at(at.0, at.1);
    m = item(m, tr(Str::OvMenuRename), "F2", Cmd::Rename, 0);
    m = item(m, tr(Str::OvMenuDuplicate), "", Cmd::Duplicate, 0);
    m = item(m, tr(Str::OvActSplit), "Alt+\\", Cmd::SplitCols, 0);
    m = item(m, tr(Str::MenuSplitRows), "Alt+-", Cmd::SplitRows, 0);
    m = m.sep().head(tr(Str::OvMenuGroup));
    for g in groups {
        m = m.dot(&g.name, g.color, Cmd::MoveTo(g.id).id(), if member == Some(g.id) { menu::CHECKED } else { 0 });
    }
    m = item(m, tr(Str::OvMenuNewGroupWith), "Alt+G", Cmd::NewGroupWith, 0);
    if member.is_some() {
        m = item(m, tr(Str::MenuRemoveFromGroup), "", Cmd::RemoveFromGroup, 0);
    }
    m = m.sep();
    m = item(m, tr(Str::MenuCloseWin), "Alt+W", Cmd::CloseWin, menu::DANGER);
    item(m, tr(Str::OvMenuCloseOthers), "", Cmd::CloseOthers, menu::DANGER | if others { 0 } else { menu::DISABLED })
}

/// The menu of a group's header: edit it, fold it, a new window in it, dissolve it, close it with its windows.
pub fn group_menu(at: (i32, i32), collapsed: bool) -> Menu {
    let m = Menu::at(at.0, at.1);
    let m = item(m, tr(Str::MenuEditGroup), "", Cmd::EditGroup, 0);
    let m = item(m, tr(if collapsed { Str::MenuExpand } else { Str::MenuCollapse }), "", Cmd::ToggleFold, 0);
    let m = item(m, tr(Str::MenuNewWinInGroup), "", Cmd::NewWinInGroup, 0).sep();
    let m = item(m, tr(Str::MenuDissolve), "", Cmd::Dissolve, 0);
    item(m, tr(Str::MenuCloseGroup), "", Cmd::CloseGroup, menu::DANGER)
}

/// The menu of a pane's row: go to it, close it.
pub fn pane_menu(at: (i32, i32)) -> Menu {
    let m = item(Menu::at(at.0, at.1), tr(Str::MenuGoPane), "", Cmd::GoPane, 0).sep();
    item(m, tr(Str::MenuClosePane), "Alt+Shift+W", Cmd::ClosePane, menu::DANGER)
}

/// The menu of the empty part of the list: a new window, a new group, and the overview when the app has one.
pub fn strip_menu(at: (i32, i32), overview: bool) -> Menu {
    let m = item(Menu::at(at.0, at.1), tr(Str::OvActNewWindow), "Alt+N", Cmd::NewWin, 0);
    let m = item(m, tr(Str::OvActNewGroup), "Alt+G", Cmd::NewGroup, 0);
    if overview { item(m.sep(), tr(Str::OvActOverview), "Alt+O", Cmd::Overview, 0) } else { m }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::Geo;

    fn groups() -> Vec<GroupEntry> {
        vec![GroupEntry { id: 7, name: "arcton".into(), color: 1 }, GroupEntry { id: 9, name: "docs".into(), color: 3 }]
    }

    #[test]
    fn every_command_has_its_own_number_and_comes_back_from_it() {
        let mut seen = std::collections::HashSet::new();
        for c in FIXED {
            assert!(seen.insert(c.id()), "{c:?}");
            assert_eq!(Cmd::from_id(c.id()), Some(c));
            assert!(c.id() > 0 && c.id() < MOVE_BASE);
        }
        assert_eq!(seen.len(), FIXED.len());
        for g in [0, 1, 7, 500] {
            assert_eq!(Cmd::from_id(Cmd::MoveTo(g).id()), Some(Cmd::MoveTo(g)));
        }
        assert_eq!(Cmd::from_id(0), None);
        assert_eq!(Cmd::from_id(MOVE_BASE - 1), None, "past the fixed ones, before the moves");
    }

    #[test]
    fn a_windows_menu_is_the_references_in_the_references_order() {
        let m = window_menu((100, 100), &groups(), Some(9), true);
        // Rename, Duplicate, two splits, sep, heading, two groups, new group, remove, sep, close, close others.
        assert_eq!(m.len(), 4 + 1 + 1 + 2 + 1 + 1 + 1 + 2);
        // Not in a group: no "remove from group" item; alone: the last item is there but cannot be chosen.
        let m = window_menu((100, 100), &groups(), None, false);
        assert_eq!(m.len(), 4 + 1 + 1 + 2 + 1 + 1 + 2);
        // No groups at all: the heading stays with the new-group item under it.
        assert_eq!(window_menu((0, 0), &[], None, true).len(), 4 + 1 + 1 + 1 + 1 + 2);
    }

    #[test]
    fn the_items_of_a_windows_menu_run_these_commands_with_these_marks() {
        let m = window_menu((100, 100), &groups(), Some(9), true);
        let ids = |ids: &[Cmd]| ids.iter().map(|c| c.id()).collect::<Vec<_>>();
        let run: Vec<u32> = m.entries().iter().map(|e| e.0).collect();
        assert_eq!(
            run,
            ids(&[
                Cmd::Rename, Cmd::Duplicate, Cmd::SplitCols, Cmd::SplitRows, Cmd::MoveTo(7), Cmd::MoveTo(9), Cmd::NewGroupWith,
                Cmd::RemoveFromGroup, Cmd::CloseWin, Cmd::CloseOthers,
            ])
        );
        // The window's own group is checked; closing is red; the others are plain.
        let flags: Vec<u8> = m.entries().iter().map(|e| e.1).collect();
        assert_eq!(flags, [0, 0, 0, 0, 0, menu::CHECKED, 0, 0, menu::DANGER, menu::DANGER]);
        // Alone, and in no group: nothing to remove, and the last item is red and cannot be chosen.
        let m = window_menu((100, 100), &groups(), None, false);
        let e = m.entries();
        assert!(!e.iter().any(|x| x.0 == Cmd::RemoveFromGroup.id()));
        assert_eq!(e.last(), Some(&(Cmd::CloseOthers.id(), menu::DANGER | menu::DISABLED)));
        assert!(e.iter().all(|x| x.1 & menu::CHECKED == 0));
    }

    #[test]
    fn what_cannot_be_chosen_is_not_found_and_enter_runs_what_is_selected() {
        use crate::{input::sym, overlay::{Cx, Out, Overlay, Text}, xkb::Key};
        let t = Text::load();
        let g = Geo { w: 900, h: 620, square: false };
        let cx = Cx { g, text: &t };
        let key = |s| Key { sym: s, ..Key::default() };
        // Alone: "Close the others" is disabled, so Up from the first item wraps to "Close window".
        let mut m = window_menu((100, 100), &[], None, false).focus_first();
        assert_eq!(m.key(&key(sym::RETURN), &cx), Out::Run(Cmd::Rename.id()));
        assert_eq!(m.key(&key(sym::UP), &cx), Out::Redraw);
        assert_eq!(m.key(&key(sym::RETURN), &cx), Out::Run(Cmd::CloseWin.id()), "the disabled one is skipped");
        // With another window the last item is a command.
        let mut m = window_menu((100, 100), &[], None, true).focus_first();
        m.key(&key(sym::UP), &cx);
        assert_eq!(m.key(&key(sym::RETURN), &cx), Out::Run(Cmd::CloseOthers.id()));
        // A group's id is in the id of its item.
        let mut m = window_menu((100, 100), &groups(), None, true).focus_first();
        for _ in 0..4 {
            m.key(&key(sym::DOWN), &cx);
        }
        assert_eq!(m.key(&key(sym::RETURN), &cx), Out::Run(Cmd::MoveTo(7).id()), "Rename, Duplicate, two splits (the separator and heading are skipped), then the first group");
    }

    #[test]
    fn the_group_pane_and_list_menus_have_their_items() {
        assert_eq!(group_menu((0, 0), false).len(), 3 + 1 + 2);
        assert_eq!(pane_menu((0, 0)).len(), 3);
        assert_eq!(strip_menu((0, 0), false).len(), 2);
        assert_eq!(strip_menu((0, 0), true).len(), 4, "a separator and the overview");
    }
}
