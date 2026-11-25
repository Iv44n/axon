//! The app's side of groups, context menus, inline rename and toasts
//! ([0029](../../knowledge/decisions/workspace/0029-group-dialog-menus-rename-toasts.md)): what the screens in `group_dialog.rs`
//! and `ctxmenu.rs` and the rename field in `side.rs` do to the windows. The rules (names, colours, what a close
//! takes out and puts back) are `groups.rs`, pure and tested; this is the part that owns the shells.
//!
//! A close keeps the closed windows (shells running, hidden) until the toast that offers to undo it is gone: undo
//! puts them back where they were with their groups, and when the time is up the stash is dropped, which ends the
//! shells (`Pane::drop`). Nothing runs on a timer for it but the one wake at the stash's end.
use std::time::Instant;

use super::*;
use crate::{
    ctxmenu::{self, Cmd, GroupEntry},
    field::{Edit, Field},
    group_dialog::{self, GroupDialog, Init, Submit, WinRow},
    groups::{self, fill},
    overlay::Clicks,
    toast,
};

/// The command the undo button of a close's toast runs (`Ran { tag: TOAST, id }`).
pub const CMD_UNDO: u32 = 0x4752_0001;
/// F2, the reference's key for renaming the window under the pointer.
const KS_F2: u32 = 0xffbf;

/// What a menu is about, kept while it is open: a window or a group by its stable id (its index can shift under an
/// open menu when a shell ends), a pane by its window's id and its place in it, or the empty part of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Win(u64),
    Group(u32),
    Pane(u64, usize),
    Strip,
}

/// A window's name being edited in its row.
pub struct Rename {
    pub win: u64,
    pub field: Field,
    clicks: Clicks,
}

/// What a close took out, until its toast is gone: each window with the place it had and the group it was in, and the
/// fresh window that was opened so that the app was not left with none.
pub struct Undo {
    entries: Vec<(usize, Win, Option<Group>)>,
    placeholder: Option<u64>,
    pub until: Instant,
}

/// How a window is opened.
#[derive(Default)]
pub struct Spawn {
    /// The folder its shell starts in (none: where the window on screen is).
    pub dir: Option<PathBuf>,
    pub group: Option<usize>,
    pub name: String,
    /// A command typed into its shell as soon as it starts.
    pub cmd: Option<String>,
    /// The window it goes right after (none: last in its group).
    pub after: Option<u64>,
    pub activate: bool,
}

impl App {
    // --------------------------------------------------------------- lookups --

    /// The index of the window with `id`.
    pub(super) fn win_index(&self, id: u64) -> Option<usize> {
        self.wins.iter().position(|w| w.id == id)
    }

    /// Shows `msg` in a toast for a few seconds.
    pub(super) fn toast(&mut self, msg: &str) {
        self.overlays.toast(msg, None, 0, Instant::now());
        self.dirty = true;
    }

    /// The folder of window `wi`'s shell.
    fn win_dir(&self, wi: usize) -> Option<PathBuf> {
        std::fs::read_link(format!("/proc/{}/cwd", self.wins[wi].pane().shell)).ok()
    }

    fn next_group_id(&mut self) -> u32 {
        self.groups_made += 1;
        self.groups_made as u32
    }

    /// A group for window `wi` alone, made the way the dialog would suggest (the name of its folder, the next colour):
    /// what a split of a window in no group starts from (ADR 0034). Returns its index; the window is in it.
    pub(super) fn group_around(&mut self, wi: usize) -> usize {
        let taken: Vec<&str> = self.groups.iter().map(|g| g.name.as_str()).collect();
        let base = win_place(&self.wins[wi]).rsplit('/').next().unwrap_or("");
        let name = groups::suggest_name(base, tr(Str::GroupName), &taken);
        let color = groups::next_color(&self.groups.iter().map(|g| g.color).collect::<Vec<_>>());
        let id = self.next_group_id();
        self.groups.push(Group { id, name, color, collapsed: false, kind: panes::Kind::Single });
        let g = self.groups.len() - 1;
        self.wins[wi].group = Some(g);
        g
    }

    // ------------------------------------------------------------- windows --

    /// Opens a window with a shell of its own (`Spawn` says where, in which group, named how, with which command);
    /// returns the number it was given.
    pub(super) fn spawn_win(&mut self, o: Spawn) -> u64 {
        let n = self.next_win;
        self.next_win += 1;
        let dir = o.dir.or_else(|| self.win_dir(self.cur));
        let pane = Pane::spawn(&self.wake, n, (self.cols, self.rows, self.cw, self.ch), self.cfg.input.cursor, self.chips_on(), dir);
        if let Some(cmd) = o.cmd {
            // The shell reads it as soon as it is up: what is typed ahead waits in the terminal.
            pane.pty.notify(format!("{cmd}\r").into_bytes());
        }
        let mut w = Win::new(pane, n);
        (w.group, w.name) = (o.group, o.name);
        let at = o.after.and_then(|id| self.win_index(id)).map_or(self.wins.len(), |i| i + 1);
        let on_screen = self.wins[self.cur].id;
        self.wins.insert(at, w);
        self.cur = self.win_index(on_screen).unwrap_or(0);
        self.rebuild_side();
        if o.activate
            && let Some(i) = self.win_index(n)
        {
            self.switch_to(i);
        }
        self.sync_flag();
        n
    }

    /// A copy of window `wi`: a new shell in the same folder, in the same group, right after it.
    fn duplicate_win(&mut self, wi: usize) {
        let o = Spawn { dir: self.win_dir(wi), group: self.wins[wi].group, after: Some(self.wins[wi].id), activate: true, ..Spawn::default() };
        self.spawn_win(o);
    }

    /// What a close of the windows at `idx` says in its toast.
    fn closed_label(&self, idx: &[usize]) -> String {
        match idx {
            [i] => fill(tr(Str::ToastClosed), &[win_title(&self.wins[*i])]),
            _ => fill(tr(Str::ToastClosedMany), &[&idx.len().to_string()]),
        }
    }

    /// A middle click on a window's row or tab closes it, with the Undo toast (the reference's `auxclick` on the strip).
    /// Returns whether it was on one.
    pub(super) fn middle_click_close(&mut self) -> bool {
        let Some(side::Part::Row(i) | side::Part::Close(i)) = self.side_part_at(self.mouse.0, self.mouse.1) else { return false };
        self.close_wins(&[self.wins[i].id], None);
        true
    }

    /// Closes the windows with `ids` and offers to undo it: they leave the list, the toast says so with an Undo button,
    /// and they stay alive, out of sight, until it is gone. Closing every window leaves a fresh one (the reference's
    /// placeholder); closing the only window quits, as before.
    pub(super) fn close_wins(&mut self, ids: &[u64], label: Option<String>) {
        let mut idx: Vec<usize> = ids.iter().filter_map(|&id| self.win_index(id)).collect();
        idx.sort_unstable();
        idx.dedup();
        if idx.is_empty() {
            return;
        }
        if idx.len() == self.wins.len() && self.wins.len() == 1 {
            self.quit = true;
            return;
        }
        self.finish_rename(false);
        let label = label.unwrap_or_else(|| self.closed_label(&idx));
        let placeholder = (idx.len() == self.wins.len()).then(|| self.spawn_win(Spawn::default()));
        let was = self.wins[self.cur].id;
        let of: Vec<Option<usize>> = self.wins.iter().map(|w| w.group).collect();
        let cur = groups::after_close_in(&of, self.cur, &idx);
        // Out of the list, last first so that the indices stay right; each remembers its group.
        let mut entries = Vec::with_capacity(idx.len());
        for &i in idx.iter().rev() {
            let w = self.wins.remove(i);
            let snap = w.group.and_then(|g| self.groups.get(g).cloned());
            entries.push((i, w, snap));
        }
        let mut of: Vec<Option<usize>> = self.wins.iter().map(|w| w.group).collect();
        groups::prune(&mut self.groups, &mut of);
        for (w, g) in self.wins.iter_mut().zip(of) {
            w.group = g;
        }
        self.cur = cur;
        self.rebuild_side();
        self.show_after_close(was);
        let now = Instant::now();
        self.undo = Some(Undo { entries, placeholder, until: now + toast::TTL + Duration::from_millis(250) });
        self.overlays.toast(&label, Some(tr(Str::OvUndo)), CMD_UNDO, now);
        self.dirty = true;
    }

    /// Puts back what the last close took out, where it was, with its groups.
    fn undo_close(&mut self) {
        let Some(u) = self.undo.take() else { return };
        let restored: Vec<u64> = u.entries.iter().map(|(_, w, _)| w.id).collect();
        let mut back = Vec::with_capacity(u.entries.len());
        for (i, mut w, snap) in u.entries {
            w.group = snap.map(|s| groups::resolve(&mut self.groups, &s));
            back.push((i, w));
        }
        groups::reinsert(&mut self.wins, back);
        // The fresh window that was opened in their place goes, unless something else was opened beside it.
        if let Some(pid) = u.placeholder {
            let only_it = self.wins.iter().all(|w| w.id == pid || restored.contains(&w.id));
            if only_it && let Some(pi) = self.win_index(pid) {
                self.wins.remove(pi);
            }
        }
        self.cur = self.cur.min(self.wins.len() - 1);
        self.rebuild_side();
        let first = restored.iter().filter_map(|&id| self.win_index(id)).min().unwrap_or(0);
        self.relayout();
        self.sync_flag();
        self.switch_to(first);
    }

    /// Drops the stash of a close once its toast is gone (the shells end).
    pub(super) fn tick_undo(&mut self, now: Instant) {
        if self.undo.as_ref().is_some_and(|u| now >= u.until) {
            self.undo = None;
        }
    }

    /// Moves window `wi` into the group with id `to`, or out of its group (none).
    pub(super) fn move_to_group(&mut self, wi: usize, to: Option<u32>) {
        let name = win_title(&self.wins[wi]).to_string();
        match to {
            Some(gid) => {
                let Some(g) = groups::index_of(&self.groups, gid) else { return };
                if self.wins[wi].group == Some(g) {
                    return;
                }
                self.wins[wi].group = Some(g);
                self.groups[g].collapsed = false;
                let msg = fill(tr(Str::ToastMoved), &[&name, &self.groups[g].name]);
                self.toast(&msg);
            }
            None => {
                if self.wins[wi].group.take().is_none() {
                    return;
                }
                let msg = fill(tr(Str::ToastUngrouped), &[&name]);
                self.toast(&msg);
            }
        }
        self.prune_groups();
        self.rebuild_side();
    }

    /// Dissolves group `gid`: its windows stay, in none.
    fn dissolve_group(&mut self, gid: u32) {
        let Some(g) = groups::index_of(&self.groups, gid) else { return };
        let name = self.groups[g].name.clone();
        let mut of: Vec<Option<usize>> = self.wins.iter().map(|w| w.group).collect();
        groups::dissolve(&mut of, g);
        for (w, g) in self.wins.iter_mut().zip(of) {
            w.group = g;
        }
        self.prune_groups();
        self.rebuild_side();
        let msg = fill(tr(Str::ToastDissolved), &[&name]);
        self.toast(&msg);
    }

    /// Closes group `gid` with all its windows (with the undo toast).
    fn close_group(&mut self, gid: u32) {
        let Some(g) = groups::index_of(&self.groups, gid) else { return };
        let ids: Vec<u64> = self.wins.iter().filter(|w| w.group == Some(g)).map(|w| w.id).collect();
        let label = fill(tr(Str::ToastGroupClosed), &[&self.groups[g].name]);
        self.close_wins(&ids, Some(label));
    }

    // ------------------------------------------------------- the group dialog --

    /// Opens the group dialog: to make a group (with `members`, the ids of the windows ticked at first) or, with
    /// `edit`, to change the name and colour of the group with that id.
    pub(super) fn open_group_dialog(&mut self, edit: Option<u32>, members: Vec<u64>) {
        if self.overlays.has(group_dialog::TAG) {
            return;
        }
        self.finish_rename(true);
        let edited = edit.and_then(|id| self.groups.iter().find(|g| g.id == id));
        if edit.is_some() && edited.is_none() {
            return;
        }
        let used: Vec<u8> = self.groups.iter().map(|g| g.color).collect();
        let taken: Vec<&str> = self.groups.iter().filter(|g| Some(g.id) != edit).map(|g| g.name.as_str()).collect();
        let (name, color) = match edited {
            Some(g) => (g.name.clone(), g.color),
            None => {
                let first = members.iter().find_map(|&id| self.win_index(id)).unwrap_or(self.cur);
                let place = win_place(&self.wins[first]);
                let base = place.rsplit('/').next().unwrap_or("");
                (groups::suggest_name(base, tr(Str::GroupName), &taken), groups::next_color(&used))
            }
        };
        let wins: Vec<WinRow> = self
            .wins
            .iter()
            .map(|w| WinRow { id: w.id, title: win_title(w).to_string(), place: win_place(w).to_string(), group: w.group.and_then(|g| self.groups.get(g)).map(|g| (g.name.clone(), g.color)) })
            .collect();
        let taken = taken.iter().map(|s| s.to_string()).collect();
        self.answer.borrow_mut().take();
        let dialog = GroupDialog::new(Init { edit, name, color, wins, picked: members, taken }, self.answer.clone());
        self.open_overlay(group_dialog::TAG, Box::new(dialog));
    }

    /// The Alt+G dialog: the window on screen is ticked when it is in no group.
    pub(super) fn group_dialog_here(&mut self) {
        let w = &self.wins[self.cur];
        let members = if w.group.is_none() { vec![w.id] } else { Vec::new() };
        self.open_group_dialog(None, members);
    }

    /// A valid form: make the group (with its windows and the template's new ones) or change the group.
    fn apply_form(&mut self, s: Submit) {
        if let Some(gid) = s.edit {
            if let Some(g) = groups::index_of(&self.groups, gid) {
                (self.groups[g].name, self.groups[g].color) = (s.name.clone(), s.color);
                self.rebuild_side();
                let msg = fill(tr(Str::ToastGroupSaved), &[&s.name]);
                self.toast(&msg);
            }
            return;
        }
        let id = self.next_group_id();
        self.groups.push(Group { id, name: s.name.clone(), color: s.color, collapsed: false, kind: panes::Kind::Single });
        let g = self.groups.len() - 1;
        let mut ticked = Vec::new();
        for &m in &s.members {
            if let Some(wi) = self.win_index(m) {
                self.wins[wi].group = Some(g);
                ticked.push(m);
            }
        }
        // The template's windows start where the one on screen is, as the reference's do.
        let dir = self.win_dir(self.cur);
        let project = dir.as_deref().map(groups::project_at).unwrap_or_default();
        let names = [tr(Str::TplServer), tr(Str::TplTests), tr(Str::TplGit)];
        let mut created = Vec::new();
        for nw in groups::template_wins(s.template, project, names) {
            let o = Spawn { dir: dir.clone(), group: Some(g), name: nw.title, cmd: nw.cmd, ..Spawn::default() };
            created.push(self.spawn_win(o));
        }
        self.prune_groups();
        self.rebuild_side();
        let total = ticked.len() + created.len();
        if let Some(i) = created.first().or(ticked.first()).and_then(|&id| self.win_index(id)) {
            self.switch_to(i);
        }
        let n = total.to_string();
        let msg = fill(tr(Str::ToastGroupMade), &[&s.name, &n, tr(if total == 1 { Str::WinOne } else { Str::WinMany })]);
        self.toast(&msg);
    }

    // ------------------------------------------------------------------ menus --

    /// The panel's or the tabs' box at window pixel `(x, y)`, when it is over the list but not over a row.
    fn in_list(&self, x: f32, y: f32) -> bool {
        if self.tabs_on_top() {
            let (x0, x1) = tabs_span(self.w);
            return y >= 0.0 && y < chrome::TITLE_H as f32 - 1.0 && x >= x0 as f32 && x < x1 as f32;
        }
        self.side_box().is_some_and(|(bx, by, bw, bh)| x >= bx as f32 && x < (bx + bw) as f32 && y >= by as f32 && y < (by + bh) as f32)
    }

    /// A right click at window pixel `(x, y)`: opens the menu of what is under it in the list of windows (a window, a
    /// group's header, a pane's row, or the empty part). False when the pointer is not on the list.
    pub(super) fn open_context_menu(&mut self, x: f32, y: f32) -> bool {
        let part = self.side_part_at(x, y);
        if part.is_none() && !self.in_list(x, y) {
            return false;
        }
        self.finish_rename(true);
        let at = (x as i32, y as i32);
        let (target, menu) = match part {
            Some(side::Part::Row(i) | side::Part::Close(i)) if i < self.wins.len() => {
                let w = &self.wins[i];
                let list: Vec<GroupEntry> = self.groups.iter().map(|g| GroupEntry { id: g.id, name: g.name.clone(), color: g.color }).collect();
                let member = w.group.and_then(|g| self.groups.get(g)).map(|g| g.id);
                (Target::Win(w.id), ctxmenu::window_menu(at, &list, member, self.wins.len() > 1))
            }
            Some(side::Part::Head(g)) if g < self.groups.len() => (Target::Group(self.groups[g].id), ctxmenu::group_menu(at, self.groups[g].collapsed)),
            Some(side::Part::Pane(wi, pi) | side::Part::ClosePane(wi, pi)) if wi < self.wins.len() => (Target::Pane(self.wins[wi].id, pi), ctxmenu::pane_menu(at)),
            _ => (Target::Strip, ctxmenu::strip_menu(at, true)),
        };
        self.menu_for = Some(target);
        // The first item is lit, as the reference's `first.focus()`: Enter chooses it, the arrows move from it.
        self.open_overlay(ctxmenu::TAG, Box::new(menu.focus_first()));
        true
    }

    /// A menu item was chosen: does it to what the menu was opened on.
    pub(super) fn menu_ran(&mut self, id: u32) {
        let (Some(cmd), Some(target)) = (Cmd::from_id(id), self.menu_for.take()) else { return };
        match (cmd, target) {
            (Cmd::Rename, Target::Win(w)) => {
                if let Some(wi) = self.win_index(w) {
                    self.switch_to(wi);
                    self.start_rename(wi);
                }
            }
            (Cmd::Duplicate, Target::Win(w)) => {
                if let Some(wi) = self.win_index(w) {
                    self.duplicate_win(wi);
                }
            }
            (Cmd::SplitCols | Cmd::SplitRows, Target::Win(w)) => {
                if let Some(wi) = self.win_index(w) {
                    self.switch_to(wi);
                    self.split(if cmd == Cmd::SplitCols { panes::Dir::Cols } else { panes::Dir::Rows });
                }
            }
            (Cmd::MoveTo(gid), Target::Win(w)) => {
                if let Some(wi) = self.win_index(w) {
                    self.move_to_group(wi, Some(gid));
                }
            }
            (Cmd::RemoveFromGroup, Target::Win(w)) => {
                if let Some(wi) = self.win_index(w) {
                    self.move_to_group(wi, None);
                }
            }
            (Cmd::NewGroupWith, Target::Win(w)) => self.open_group_dialog(None, vec![w]),
            (Cmd::CloseWin, Target::Win(w)) => self.close_wins(&[w], None),
            (Cmd::CloseOthers, Target::Win(w)) => {
                let others: Vec<u64> = self.wins.iter().map(|x| x.id).filter(|&id| id != w).collect();
                self.close_wins(&others, None);
            }
            (Cmd::EditGroup, Target::Group(g)) => self.open_group_dialog(Some(g), Vec::new()),
            (Cmd::ToggleFold, Target::Group(g)) => {
                if let Some(gi) = groups::index_of(&self.groups, g) {
                    self.fold(gi);
                }
            }
            (Cmd::NewWinInGroup, Target::Group(g)) => {
                if let Some(gi) = groups::index_of(&self.groups, g) {
                    self.groups[gi].collapsed = false;
                    self.spawn_win(Spawn { group: Some(gi), activate: true, ..Spawn::default() });
                }
            }
            (Cmd::Dissolve, Target::Group(g)) => self.dissolve_group(g),
            (Cmd::CloseGroup, Target::Group(g)) => self.close_group(g),
            (Cmd::GoPane, Target::Pane(w, pi)) => {
                if let Some(wi) = self.win_index(w) {
                    self.switch_to(wi);
                    self.set_focus(pi);
                }
            }
            (Cmd::ClosePane, Target::Pane(w, pi)) => {
                if let Some(wi) = self.win_index(w) {
                    self.close_pane_asked(wi, pi);
                }
            }
            (Cmd::NewWin, _) => self.new_win(),
            (Cmd::NewGroup, _) => self.group_dialog_here(),
            (Cmd::Overview, _) => self.toggle_overview(),
            _ => {}
        }
    }

    /// The group dialog ran its command: a valid form is waiting in the shared cell.
    pub(super) fn dialog_ran(&mut self, id: u32) {
        if id == group_dialog::CMD_SUBMIT {
            let answer = self.answer.borrow_mut().take();
            if let Some(s) = answer {
                self.apply_form(s);
            }
        }
    }

    /// A toast's button.
    pub(super) fn toast_ran(&mut self, id: u32) {
        if id == CMD_UNDO {
            self.undo_close();
        }
    }

    // --------------------------------------------------------- debugging aid --

    /// `AXON_LIST_UI=dialog|edit|menu|group-menu|pane-menu|strip-menu|rename|close|toast` starts with one of this
    /// file's screens open (screenshots and smoke tests of a window that cannot be driven by hand). False if the name
    /// is none of them, or the screen needs a group and there is none (`AXON_GROUPS=1`).
    pub(super) fn list_ui(&mut self, kind: &str) -> bool {
        let id = self.wins[self.cur].id;
        let at = (80, 160);
        let group = self.groups.first().map(|g| g.id);
        match (kind, group) {
            ("dialog", _) => self.group_dialog_here(),
            ("edit", Some(g)) => self.open_group_dialog(Some(g), Vec::new()),
            ("menu", _) => {
                let list: Vec<GroupEntry> = self.groups.iter().map(|g| GroupEntry { id: g.id, name: g.name.clone(), color: g.color }).collect();
                self.menu_for = Some(Target::Win(id));
                self.open_overlay(ctxmenu::TAG, Box::new(ctxmenu::window_menu(at, &list, None, self.wins.len() > 1)));
            }
            ("group-menu", Some(g)) => {
                self.menu_for = Some(Target::Group(g));
                self.open_overlay(ctxmenu::TAG, Box::new(ctxmenu::group_menu(at, false)));
            }
            ("pane-menu", _) => {
                self.menu_for = Some(Target::Pane(id, 0));
                self.open_overlay(ctxmenu::TAG, Box::new(ctxmenu::pane_menu(at)));
            }
            ("strip-menu", _) => {
                self.menu_for = Some(Target::Strip);
                self.open_overlay(ctxmenu::TAG, Box::new(ctxmenu::strip_menu(at, true)));
            }
            ("rename", _) => self.start_rename(self.cur),
            ("close", _) if self.wins.len() > 1 => {
                let other = self.wins.iter().map(|w| w.id).find(|&w| w != id).unwrap_or(id);
                self.close_wins(&[other], None);
            }
            ("toast", _) => self.toast(tr(Str::ToastNoRoom)),
            _ => return false,
        }
        true
    }

    // ----------------------------------------------------------------- rename --

    /// Starts renaming window `wi` in its row (the field is the reference's `.t-edit`): its name, all selected.
    pub(super) fn start_rename(&mut self, wi: usize) {
        if self.rename.is_some() || wi >= self.wins.len() || !(self.tabs_on_top() || self.side_box().is_some()) {
            return;
        }
        let mut field = Field::new(groups::RENAME_MAX, "");
        field.set(win_title(&self.wins[wi]));
        field.select_all();
        self.rename = Some(Rename { win: self.wins[wi].id, field, clicks: Clicks::default() });
        self.side_dirty = true;
        self.dirty = true;
    }

    /// Ends the rename: with `commit` the window gets the typed name (the automatic one again when it is empty or the
    /// automatic name itself); without, nothing changes.
    pub(super) fn finish_rename(&mut self, commit: bool) {
        let Some(r) = self.rename.take() else { return };
        if commit && let Some(wi) = self.win_index(r.win) {
            let name = groups::rename_result(r.field.text(), auto_title(&self.wins[wi]));
            self.wins[wi].name = name;
            self.titled(wi);
        }
        self.side_dirty = true;
        self.dirty = true;
    }

    /// A key while a rename is open: it is the field's (Enter commits, Esc cancels), the shell hears nothing. None
    /// when no rename is open (or its window went): the key is the app's. Some says whether the key may repeat.
    pub(super) fn rename_key(&mut self, k: &Key) -> Option<bool> {
        self.rename.as_ref()?;
        // Its window went (a shell ended), or the list is not on screen (`Alt+B`): there is nothing to edit.
        if self.rename_box().is_none() {
            self.rename = None;
            self.side_dirty = true;
            return None;
        }
        let r = self.rename.as_mut()?;
        match k.sym {
            input::sym::RETURN | input::sym::KP_ENTER => {
                self.finish_rename(true);
                return Some(false);
            }
            input::sym::ESCAPE => {
                self.finish_rename(false);
                return Some(false);
            }
            _ => {}
        }
        match r.field.key(k) {
            Edit::Paste => {
                if let Some(text) = self.wl.clipboard_text() {
                    r.field.insert(&text);
                }
            }
            Edit::Copy(t) | Edit::Cut(t) => self.wl.set_clipboard(&t),
            Edit::Ignored | Edit::Same | Edit::Moved | Edit::Changed => {}
        }
        self.side_dirty = true;
        self.dirty = true;
        Some(true)
    }

    /// The field's box in window pixels.
    fn rename_box(&self) -> Option<overlay::Rect> {
        let r = self.rename.as_ref()?;
        let wi = self.win_index(r.win)?;
        let (lay, _) = self.list_layout()?;
        lay.edit_rect(wi)
    }

    /// A press at window pixel `(x, y)` while a rename is open. On the field it places the caret (a double click
    /// selects a word, a third everything) and is taken; anywhere else it ends the rename, as the reference's blur, and
    /// is the app's.
    pub(super) fn rename_press(&mut self, x: f32, y: f32, time: u32) -> bool {
        let Some(b) = self.rename_box() else {
            self.finish_rename(true);
            return false;
        };
        if !overlay::inside(b, x, y) {
            self.finish_rename(true);
            return false;
        }
        let soft = &self.soft;
        let Some(r) = self.rename.as_mut() else { return false };
        let n = r.clicks.press(time);
        let scroll = side::edit_scroll(soft, r.field.text(), r.field.caret(), b.2);
        r.field.press(x as i32 - (b.0 + side::EDIT_PAD) + scroll, &|s: &str| soft.ui_width(s, side::TITLE_PX), n, false);
        self.side_dirty = true;
        self.dirty = true;
        true
    }

    /// The button came up: a drag in the field ends.
    pub(super) fn rename_release(&mut self) {
        if let Some(r) = self.rename.as_mut() {
            r.field.release();
        }
    }

    /// The pointer moved to window pixel `(x, y)` with a drag in the rename field: the selection grows to it. True if
    /// a drag is in progress (the move is then the field's alone).
    pub(super) fn rename_drag(&mut self, x: f32) -> bool {
        let Some(b) = self.rename_box() else { return false };
        let soft = &self.soft;
        let Some(r) = self.rename.as_mut().filter(|r| r.field.dragging()) else { return false };
        let scroll = side::edit_scroll(soft, r.field.text(), r.field.caret(), b.2);
        if r.field.drag(x as i32 - (b.0 + side::EDIT_PAD) + scroll, &|s: &str| soft.ui_width(s, side::TITLE_PX)) {
            self.side_dirty = true;
            self.dirty = true;
        }
        true
    }

    /// F2 over a window's row or tab renames it (the reference's key for the tab with the keyboard on it). True if it
    /// did; over nothing the key is the shell's, which may use F2 itself.
    pub(super) fn f2_rename(&mut self, k: &Key) -> bool {
        let m = k.mods;
        if k.sym != KS_F2 || m.ctrl || m.alt || m.shift {
            return false;
        }
        match self.side_hover {
            Some(side::Part::Row(i) | side::Part::Close(i)) if i < self.wins.len() => {
                self.switch_to(i);
                self.start_rename(i);
                true
            }
            _ => false,
        }
    }
}
