//! The app's side of the panel of SSH servers ([0048](../../knowledge/decisions/workspace/0048-ssh-servers-by-alias.md)), a
//! child of `wlapp.rs`: opening and closing it (`Alt+H`, the title bar's button, the palette; it takes the settings' place),
//! what its effects do (connect, save, delete with an *Undo*, import, export, copy, its *More* menu), and what the shell's hook
//! says about ssh: a window whose shell runs `ssh web` is in a session to `web` (its label is the alias, its status a cyan
//! dot, the panel says "connected"), and an ssh to an address that is not saved offers to save it.
//!
//! Connecting does what makes sense (the reference's `connectHere`): a window that has a session to that server already is
//! brought on screen; else the pane with the focus types `ssh <alias>` if it is at an empty prompt; else a new window in the
//! group *Servers* runs it.
use std::time::Instant;

use alacritty_terminal::event::Notify;

use super::{App, groups_ui::Spawn};
use crate::{
    chrome::Hit,
    commands, font,
    groups::Group,
    hosts::{Effect, Hosts},
    i18n::{Str, tr},
    menu::Menu,
    panes,
    ssh::{self, Book, Dest, Form},
    view,
};

/// The tag the *More* menu is opened under, and its commands.
pub const MENU_TAG: u32 = 0x484f_5354;
const MENU_IMPORT: u32 = 1;
const MENU_EXPORT: u32 = 2;
const MENU_COPY: u32 = 3;
/// The commands of the panel's toasts' buttons (`toast_ran`): put a deleted server back, save an ssh the shell ran.
pub const CMD_UNDO_HOST: u32 = 0x4855;
pub const CMD_SAVE_SSH: u32 = 0x4853;
/// The colour of the servers (the reference's `cyan`): their group, their dot.
const CYAN: u8 = 2;

impl App {
    /// The servers, read the first time they are needed.
    pub(super) fn book(&mut self) -> &mut Book {
        self.book.get_or_insert_with(Book::load)
    }

    /// Whether each saved server has a session open in some window now.
    fn live_hosts(&self) -> Vec<bool> {
        let Some(b) = &self.book else { return Vec::new() };
        let mut live = vec![false; b.hosts.len()];
        for p in self.wins.iter().flat_map(|w| &w.panes) {
            if let Some(i) = p.history.remote().and_then(|d| ssh::find(&b.hosts, d)) {
                live[i] = true;
            }
        }
        live
    }

    /// The panel shows the servers and the sessions as they are now.
    pub(super) fn hosts_refresh(&mut self) {
        let live = self.live_hosts();
        if let (Some(h), Some(b)) = (self.hosts.as_mut(), self.book.as_ref()) {
            h.sync(&b.hosts, Some(live));
            self.prefs_dirty = true;
            self.dirty = true;
        }
    }

    /// `Alt+H` and the title bar's button: opens the panel (the settings close: they share the place) or closes it.
    pub(super) fn toggle_hosts(&mut self) {
        if self.hosts.is_some() {
            self.close_hosts();
        } else {
            self.open_hosts();
        }
    }

    /// Opens the panel on the list, or gives it the keyboard when it is open (the palette's *SSH servers*).
    pub(super) fn open_hosts(&mut self) {
        if let Some(h) = self.hosts.as_mut() {
            h.focused = true;
            self.prefs_dirty = true;
            self.dirty = true;
            return;
        }
        if self.panel.is_some() {
            self.close_settings();
        }
        // Read again: the file may have been edited by hand since.
        self.book = Some(Book::load());
        let live = self.live_hosts();
        let b = self.book.as_ref().expect("just loaded");
        let mut h = Hosts::new(&b.hosts, live);
        h.start_faded();
        self.hosts = Some(Box::new(h));
        self.ui.hosts = true;
        self.repeat = None;
        self.relayout();
    }

    /// Closes the panel: the grid takes its room back and the keyboard goes to the terminal.
    pub(super) fn close_hosts(&mut self) {
        if self.hosts.take().is_none() {
            return;
        }
        font::release_memory();
        self.ui.hosts = false;
        if self.hover == Hit::Prefs {
            self.hover = Hit::Client;
        }
        self.relayout();
    }

    /// What an input to the panel asks of the app.
    pub(super) fn hosts_effect(&mut self, e: Effect) {
        if e == Effect::None {
            return;
        }
        self.prefs_dirty = true;
        self.dirty = true;
        match e {
            Effect::None | Effect::Redraw => {}
            Effect::Close => {
                self.repeat = None;
                self.close_hosts();
            }
            Effect::Connect { host, window } => self.connect_host(host, window),
            Effect::Save { at, host } => self.save_host(at, host),
            Effect::Delete(i) => self.delete_host(i),
            Effect::Import(new) => self.import_hosts(new),
            Effect::Menu(x, y) => {
                let m = Menu::at(x, y)
                    .item(tr(Str::HostsMenuImport), "", MENU_IMPORT, 0)
                    .item(tr(Str::HostsMenuExport), "", MENU_EXPORT, 0)
                    .item(tr(Str::HostsMenuCopy), "", MENU_COPY, 0)
                    .focus_first();
                self.open_overlay(MENU_TAG, Box::new(m));
            }
            Effect::CopyConfig => self.copy_config(),
            Effect::Clip(s) => self.plat.set_clipboard(&s),
            Effect::Paste => {
                let Some(text) = self.plat.clipboard_text() else { return };
                let e = match (self.hosts.as_mut(), self.book.as_ref()) {
                    (Some(h), Some(b)) => h.paste(&text, &b.hosts),
                    _ => Effect::None,
                };
                self.hosts_effect(e);
            }
        }
    }

    /// A key while the panel has the keyboard; it never reaches the shell. Returns whether the panel is still open.
    pub(super) fn hosts_key(&mut self, k: &crate::plat::Key) -> bool {
        let e = match (self.hosts.as_mut(), self.book.as_ref()) {
            (Some(h), Some(b)) => h.key(k, &b.hosts),
            _ => return false,
        };
        self.hosts_effect(e);
        self.hosts.is_some()
    }

    /// A primary-button press or release while the panel is open: whether the panel took it (as `panel_button`).
    pub(super) fn hosts_button(&mut self, pressed: bool, time: u32) -> bool {
        let over = self.in_prefs(self.mouse.0, self.mouse.1);
        let (Some(h), Some(b)) = (self.hosts.as_mut(), self.book.as_ref()) else { return false };
        if !pressed {
            h.release();
            return false;
        }
        if !over {
            if h.focused {
                h.focused = false;
                self.prefs_dirty = true;
                self.dirty = true;
            }
            return false;
        }
        let clicks = self.hosts_clicks.press(time);
        let e = h.press(self.mouse.0, self.mouse.1, clicks, &b.hosts);
        self.prefs_dirty = true;
        self.dirty = true;
        self.hosts_effect(e);
        true
    }

    pub(super) fn hosts_pointer(&mut self, x: f32, y: f32) {
        let e = match (self.hosts.as_mut(), self.book.as_ref()) {
            (Some(h), Some(b)) => h.pointer(x, y, &b.hosts),
            _ => return,
        };
        self.hosts_effect(e);
    }

    pub(super) fn hosts_scroll(&mut self, px: i32) {
        let e = match (self.hosts.as_mut(), self.book.as_ref()) {
            (Some(h), Some(b)) => h.scroll_by(px, &b.hosts),
            _ => return,
        };
        self.hosts_effect(e);
    }

    /// The pointer's shape over the panel.
    pub(super) fn hosts_cursor(&self) -> u32 {
        use crate::plat::cursor;
        match self.hosts.as_deref() {
            Some(h) if h.over_text() => cursor::TEXT,
            Some(h) if h.over_button() => cursor::POINTER,
            _ => cursor::DEFAULT,
        }
    }

    // ------------------------------------------------------------ connecting --

    /// Connects to server `i`: to the window that has a session to it already, in the pane with the focus when it waits at
    /// an empty prompt, else in a new window of the group *Servers* (always there with `window`).
    pub(super) fn connect_host(&mut self, i: usize, window: bool) {
        let Some(alias) = self.book().hosts.get(i).map(|h| h.alias.clone()) else { return };
        self.book().touch(i, ssh::now());
        if let Some(h) = self.hosts.as_mut() {
            h.focused = false;
        }
        if !window {
            let open = {
                let b = self.book.as_ref().expect("read above");
                self.wins.iter().position(|w| w.panes.iter().any(|p| p.history.remote().and_then(|d| ssh::find(&b.hosts, d)) == Some(i)))
            };
            if let Some(wi) = open {
                self.go_to(wi);
                self.hosts_refresh();
                return;
            }
            let pane = self.wins[self.cur].pane();
            let free = !pane.history.running() && view::input_empty(&pane.term.lock());
            if free {
                self.type_line(&format!("ssh {alias}\r"));
                self.hosts_refresh();
                return;
            }
        }
        self.host_window(&alias);
        self.hosts_refresh();
    }

    /// Types `text` into the shell of the pane with the focus, the view back at the bottom.
    fn type_line(&mut self, text: &str) {
        let pane = self.wins[self.cur].pane();
        let mut t = pane.term.lock();
        if t.grid().display_offset() > 0 {
            t.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        }
        t.selection = None;
        drop(t);
        pane.pty.notify(text.as_bytes().to_vec());
        self.dirty = true;
    }

    /// A new window in the group *Servers* (made, in cyan, when there is none) whose shell runs `ssh <alias>`, in the home
    /// folder, on screen.
    fn host_window(&mut self, alias: &str) {
        let name = tr(Str::HostsGroup);
        let g = match self.groups.iter().position(|g| g.name == name) {
            Some(g) => g,
            None => {
                let id = self.next_group_id();
                self.groups.push(Group { id, name: name.to_string(), color: CYAN, collapsed: false, kind: panes::Kind::Single, tracks: panes::Tracks::EVEN });
                self.groups.len() - 1
            }
        };
        let after = self.wins.iter().rev().find(|w| w.group == Some(g)).map(|w| w.id);
        let o = Spawn { dir: crate::context::home_dir(), group: Some(g), cmd: Some(format!("ssh {alias}")), after, activate: true, ..Spawn::default() };
        self.spawn_win(o);
    }

    // ---------------------------------------------------------------- saving --

    /// The panel's form was saved: in place of server `at` or as a new one, written to axon's file (and the `Include` into
    /// `~/.ssh/config` the first time), and the panel back on the list.
    fn save_host(&mut self, at: Option<usize>, host: ssh::Host) {
        let alias = host.alias.clone();
        let b = self.book();
        match at {
            Some(i) if i < b.hosts.len() => b.hosts[i] = host,
            _ => b.hosts.push(host),
        }
        let saved = b.save();
        b.save_used();
        let msg = match saved {
            Ok(s) => {
                let mut m = tr(Str::ToastHostSaved).replacen("{}", &alias, 1).replacen("{}", &alias, 1);
                if s == ssh::Saved::Included {
                    m.push_str(tr(Str::ToastHostIncluded));
                }
                m
            }
            Err(e) => format!("{}{e}", tr(Str::NotSaved)),
        };
        self.after_book_change();
        self.toast(&msg);
    }

    /// Takes server `i` out, with an *Undo* in the toast.
    fn delete_host(&mut self, i: usize) {
        let b = self.book();
        if i >= b.hosts.len() {
            return;
        }
        let gone = b.hosts.remove(i);
        let saved = b.save();
        b.save_used();
        let msg = match saved {
            Ok(_) => tr(Str::ToastHostDeleted).replace("{}", &gone.alias),
            Err(e) => format!("{}{e}", tr(Str::NotSaved)),
        };
        self.host_undo = Some((i, gone));
        self.after_book_change();
        self.overlays.toast(&msg, Some(tr(Str::OvUndo)), CMD_UNDO_HOST, Instant::now());
        self.dirty = true;
    }

    fn undo_host_delete(&mut self) {
        let Some((i, h)) = self.host_undo.take() else { return };
        let b = self.book();
        let at = i.min(b.hosts.len());
        b.hosts.insert(at, h);
        if let Err(e) = b.save() {
            self.toast(&format!("{}{e}", tr(Str::NotSaved)));
        }
        self.book().save_used();
        self.after_book_change();
    }

    fn import_hosts(&mut self, new: Vec<ssh::Host>) {
        let n = new.len();
        let b = self.book();
        b.hosts.extend(new);
        let msg = match b.save() {
            Ok(_) => {
                if n == 1 {
                    tr(Str::ToastImportedOne).to_string()
                } else {
                    tr(Str::ToastImportedMany).replace("{}", &n.to_string())
                }
            }
            Err(e) => format!("{}{e}", tr(Str::NotSaved)),
        };
        self.after_book_change();
        self.toast(&msg);
    }

    fn copy_config(&mut self) {
        let text = ssh::write_config(&self.book().hosts);
        self.plat.set_clipboard(&text);
        self.toast(tr(Str::ToastConfigCopied));
    }

    /// The servers changed: the panel goes back to its list, and the windows in a session say the alias they have now.
    fn after_book_change(&mut self) {
        let live = self.live_hosts();
        if let (Some(h), Some(b)) = (self.hosts.as_mut(), self.book.as_ref()) {
            h.list(&b.hosts);
            h.sync(&b.hosts, Some(live));
        }
        self.relabel_remotes();
        self.prefs_dirty = true;
        self.dirty = true;
    }

    /// Each window in a session is labelled with its server's alias, or its address when it is not saved.
    fn relabel_remotes(&mut self) {
        let Some(b) = self.book.as_ref() else { return };
        let mut changed = Vec::new();
        for (wi, w) in self.wins.iter_mut().enumerate() {
            for p in &mut w.panes {
                let Some(d) = p.history.remote().cloned() else { continue };
                let label = ssh::find(&b.hosts, &d).map_or(d.host.clone(), |i| b.hosts[i].alias.clone());
                if p.history.command() != Some(label.as_str()) {
                    p.history.set_label(&label);
                    changed.push(wi);
                }
            }
        }
        for wi in changed {
            self.titled(wi);
        }
        self.side_dirty = true;
    }

    // ------------------------------------------------------------ the menu --

    /// A command of the *More* menu.
    pub(super) fn hosts_menu_ran(&mut self, id: u32) {
        let text = match id {
            MENU_IMPORT => Some(self.book().read_ssh_config().map_err(|e| e.to_string())),
            _ => None,
        };
        let export = (id == MENU_EXPORT).then(|| {
            let b = self.book();
            let path = b.file().map(|p| tidy(&p.to_string_lossy())).unwrap_or_default();
            (ssh::write_config(&b.hosts), path)
        });
        match id {
            MENU_IMPORT => {
                if let (Some(h), Some(t)) = (self.hosts.as_mut(), text) {
                    h.import(t);
                    h.focused = true;
                }
            }
            MENU_EXPORT => {
                if let (Some(h), Some((t, p))) = (self.hosts.as_mut(), export) {
                    h.export(t, p);
                    h.focused = true;
                }
            }
            MENU_COPY => self.copy_config(),
            _ => {}
        }
        self.prefs_dirty = true;
        self.dirty = true;
    }

    /// A button of one of the panel's toasts.
    pub(super) fn hosts_toast_ran(&mut self, id: u32) {
        match id {
            CMD_UNDO_HOST => self.undo_host_delete(),
            CMD_SAVE_SSH => {
                let Some(form) = self.host_offer.take() else { return };
                if self.hosts.is_none() {
                    self.open_hosts();
                }
                if let (Some(h), Some(b)) = (self.hosts.as_mut(), self.book.as_ref()) {
                    h.edit(&b.hosts, None, Some(form), true);
                    h.focused = true;
                }
                self.prefs_dirty = true;
                self.dirty = true;
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------- sessions --

    /// The shell of pane `j` of window `i` started an ssh session to `d`: its label is the server's alias (else its
    /// address), the server was used now, and an ssh typed to an address says whether it has an alias already, or offers
    /// to save it as one (the reference's footers, here in a toast, for a window on screen).
    pub(super) fn ssh_session(&mut self, i: usize, j: usize, d: Dest) {
        let b = self.book();
        let found = ssh::find(&b.hosts, &d);
        let label = found.map_or_else(|| d.host.clone(), |k| b.hosts[k].alias.clone());
        if let Some(k) = found {
            b.touch(k, ssh::now());
        }
        let offer = (found.is_none() && (ssh::looks_address(&d.host) || !d.user.is_empty())).then(|| {
            let mut f = Form { hostname: d.host.clone(), user: d.user.clone(), identity: d.identity.clone(), ..Form::default() };
            if d.port != 0 && d.port != 22 {
                f.port = d.port.to_string();
            }
            f.alias = ssh::suggest_alias(&f, &b.hosts, tr(Str::HostsDefaultAlias));
            f
        });
        let has_alias = found.filter(|&k| !b.hosts[k].alias.eq_ignore_ascii_case(&d.host)).map(|k| b.hosts[k].alias.clone());
        let p = &mut self.wins[i].panes[j];
        p.history.set_label(&label);
        p.history.set_remote(Some(d));
        let seen = self.shown().run().contains(&i);
        if seen {
            if let Some(f) = offer {
                let msg = tr(Str::ToastUseAgain);
                let act = tr(Str::ToastSaveAs).replace("{}", &f.alias);
                self.host_offer = Some(f);
                self.overlays.toast(msg, Some(&act), CMD_SAVE_SSH, Instant::now());
            } else if let Some(a) = has_alias {
                self.toast(&tr(Str::ToastHasAlias).replace("{}", &a));
            }
        }
        if self.hosts.is_some() {
            self.hosts_refresh();
        }
    }

    // --------------------------------------------------------- the palette --

    /// The servers as the palette lists them, the last used first.
    pub(super) fn host_infos(&mut self) -> Vec<commands::HostInfo> {
        let local = ssh::local_user();
        let b = self.book();
        ssh::by_recent(&b.hosts)
            .into_iter()
            .map(|i| commands::HostInfo { index: i as u32, alias: b.hosts[i].alias.clone(), target: b.hosts[i].target(&local), hostname: b.hosts[i].hostname.clone() })
            .collect()
    }

    // ------------------------------------------------------- debugging aid --

    /// `AXON_HOSTS=list|new|edit|import|export` starts with the panel open on that view (screenshots and measurements).
    /// False if the name is none of them.
    pub(super) fn hosts_ui(&mut self, kind: &str) -> bool {
        if !matches!(kind, "" | "1" | "list" | "new" | "edit" | "import" | "export") {
            return false;
        }
        self.open_hosts();
        match kind {
            "import" => self.hosts_menu_ran(MENU_IMPORT),
            "export" => self.hosts_menu_ran(MENU_EXPORT),
            "new" | "edit" => {
                if let (Some(h), Some(b)) = (self.hosts.as_mut(), self.book.as_ref()) {
                    h.edit(&b.hosts, (kind == "edit" && !b.hosts.is_empty()).then_some(0), None, false);
                }
            }
            _ => {}
        }
        true
    }
}

/// A path with the home folder written `~`.
fn tidy(p: &str) -> String {
    let home = crate::context::home_dir();
    crate::context::tidy_dir(p, home.as_deref().and_then(std::path::Path::to_str))
}
