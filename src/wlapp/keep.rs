//! The session between runs, the app's side (`restore.rs` is the file): the snapshot of the windows and groups as they are,
//! when it is written, and how it is put back at startup. A session of windows, each in the folder its shell was in, with
//! its name and group; what a shell was running is not kept.
//!
//! Nothing here is on the paint path: a change schedules a write (`note_session`), the loop wakes for it once (the one
//! timer that exists only while a change is waiting, so idle stays at 0 % CPU), and the write is a few hundred bytes.
use std::path::PathBuf;

use super::*;
use crate::restore::{self, Saved, SavedGroup, SavedWin};

/// How long after a change the session is written: the first change starts the wait and the ones that follow join it,
/// so a burst of `cd`s is one write.
const DELAY: Duration = Duration::from_millis(1500);

/// The debugging aids that start a screen or a state for screenshots and measurements: a run with one of them is not
/// the user's session, so it neither reads nor writes it.
const AIDS: [&str; 9] = ["AXON_WINDOWS", "AXON_GROUPS", "AXON_PANES", "AXON_RUNNING", "AXON_SETTINGS", "AXON_OVERVIEW", "AXON_LIST_UI", "AXON_DEMO_OVERLAY", "AXON_SELFTEST"];

/// Whether `pid` is another axon that runs: a process with that id exists and, where the system says what a process is called,
/// it is called as this one. An id that another program took after the owner ended must not keep the session from being
/// opened again.
fn axon_runs(pid: u32) -> bool {
    if !context::process_alive(pid) {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let comm = |p: &str| std::fs::read_to_string(format!("/proc/{p}/comm")).ok();
        comm(&pid.to_string()).is_some_and(|c| Some(c) == comm("self"))
    }
    #[cfg(windows)]
    true
}

/// What this run does with the session.
pub(super) struct Plan {
    /// This axon keeps the session (reads it now, writes it as things change and at the end).
    pub keep: bool,
    /// The session to open again, if there is one.
    pub saved: Option<Saved>,
}

/// Decides what to do with the session: nothing when `[windows] restore` is off or an aid is in use or another axon that
/// still runs owns it (`restore::claims`); else read it and keep it.
pub(super) fn plan(cfg: &Config) -> Plan {
    let aids = AIDS.iter().any(|k| std::env::var_os(k).is_some());
    if !cfg.windows.restore || aids || restore::path().is_none() {
        return Plan { keep: false, saved: None };
    }
    let saved = restore::load();
    let mine = saved.as_ref().is_none_or(|s| restore::claims(s.owner, std::process::id(), axon_runs));
    Plan { keep: mine, saved: saved.filter(|_| mine) }
}

/// The folder a saved window opens in: where it was, if that is still a folder, else the home.
pub(super) fn folder(saved: &str) -> Option<PathBuf> {
    Some(PathBuf::from(saved)).filter(|p| !saved.is_empty() && p.is_dir()).or_else(context::home_dir)
}

impl App {
    /// The session as it is now: the windows in the order of the list, each in the folder its shell was last seen in.
    fn snapshot(&self) -> Saved {
        Saved {
            owner: std::process::id(),
            cur: self.cur,
            groups: self.groups.iter().map(|g| SavedGroup { name: g.name.clone(), color: g.color, collapsed: g.collapsed, kind: g.kind }).collect(),
            wins: self.wins.iter().map(|w| SavedWin { dir: w.pane().place.to_string_lossy().into_owned(), name: w.name.clone(), group: w.group }).collect(),
        }
    }

    /// Something the session keeps changed (a window came or went, a folder, a name, a group, the window on screen): it is
    /// written in a moment.
    pub(super) fn note_session(&mut self) {
        if self.keep_session && self.session_at.is_none() {
            self.session_at = Some(Instant::now() + DELAY);
        }
    }

    /// Writes the session when its moment has come.
    pub(super) fn tick_session(&mut self) {
        if self.session_at.is_some_and(|t| Instant::now() >= t) {
            self.save_session();
        }
    }

    /// Writes the session now (the end of the run does, whatever was waiting). A write that fails is not worth a message:
    /// the next change tries again.
    pub(super) fn save_session(&mut self) {
        self.session_at = None;
        if self.keep_session {
            let _ = restore::save(&self.snapshot());
        }
    }

    /// Opens the windows of `saved`: the first window is the one the app started with (it was made in the first folder), the
    /// others open after it, in their folders and groups. The one that was on screen is shown.
    pub(super) fn restore_session(&mut self, saved: &Saved) {
        for g in &saved.groups {
            self.groups_made += 1;
            self.groups.push(Group { id: self.groups_made as u32, name: g.name.clone(), color: g.color, collapsed: g.collapsed, kind: g.kind });
        }
        if let (Some(first), Some(w)) = (saved.wins.first(), self.wins.first_mut()) {
            (w.group, w.name) = (first.group, first.name.clone());
        }
        for w in saved.wins.iter().skip(1) {
            self.spawn_win(groups_ui::Spawn { dir: folder(&w.dir), group: w.group, name: w.name.clone(), ..groups_ui::Spawn::default() });
        }
        self.prune_groups();
        self.rebuild_side();
        self.switch_to(saved.cur);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_that_is_gone_opens_in_the_home_and_one_that_exists_opens_there() {
        let here = std::env::temp_dir();
        assert_eq!(folder(&here.to_string_lossy()), Some(here));
        let home = context::home_dir();
        assert_eq!(folder("/this/folder/does/not/exist"), home);
        assert_eq!(folder(""), home);
    }

    #[test]
    fn a_run_with_a_debugging_aid_or_the_setting_off_keeps_nothing() {
        let off = Config { windows: config::WindowsCfg { restore: false, ..Default::default() }, ..Default::default() };
        let p = plan(&off);
        assert!(!p.keep && p.saved.is_none());
    }
}
