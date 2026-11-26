//! The session kept between runs: which windows were open, in which folders, how they were grouped and which one was on
//! screen, so that closing the terminal and opening it again puts the user back where they were (`[windows] restore`).
//!
//! A small text file in the state directory (`$XDG_STATE_HOME/axon/session`, `~/.local/state/axon/session`;
//! `%LOCALAPPDATA%\axon\session` on Windows), own format, own parser (no dependencies, ADR 0010):
//!
//! ```text
//! axon-session 1
//! owner 4242
//! cur 1
//! group <colour> <folded 0|1> <layout>\t<name>
//! win <group index or ->\t<folder>\t<name>
//! ```
//!
//! What a shell was running is not kept: only where it was. Names and folders are escaped (`\\`, `\t`, `\n`, `\r`), so
//! whatever a folder is called fits on its line. Unknown lines are ignored, so a newer file still reads in an older axon.
//!
//! One axon owns the file at a time (`owner`, its process id): a second axon opened while the first runs starts fresh and
//! leaves the first's session alone, instead of opening a copy of it. The pure part is here (the format, the rules); the
//! app builds the snapshot and puts it back (`wlapp.rs`).
use std::path::PathBuf;

use crate::{groups::COLORS, panes::Kind};

/// The file's first line: what it is and which format.
const HEADER: &str = "axon-session 1";

/// More windows than this are not restored (a damaged file must not open a hundred shells).
pub const MAX_WINS: usize = 32;

/// The largest file read: a session of `MAX_WINS` windows with long folders is a few kilobytes.
const MAX_BYTES: u64 = 256 * 1024;

/// A window as it was: the folder its shell was in, the name the user gave it (empty: the automatic one) and its group.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SavedWin {
    pub dir: String,
    pub name: String,
    pub group: Option<usize>,
}

/// A group as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedGroup {
    pub name: String,
    pub color: u8,
    pub collapsed: bool,
    pub kind: Kind,
}

/// A whole session: the windows in the order of the list, the groups they index, the window that was on screen, and the
/// process that wrote it (0: nobody).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Saved {
    pub owner: u32,
    pub cur: usize,
    pub groups: Vec<SavedGroup>,
    pub wins: Vec<SavedWin>,
}

/// Where the session is kept: `AXON_SESSION` if it names a file, nowhere if it is `off` (benchmarks and tests: the user's
/// own session is not touched), else the state directory. None where there is no home to keep it in.
pub fn path() -> Option<PathBuf> {
    match std::env::var_os("AXON_SESSION") {
        Some(v) if v == "off" || v == "0" => return None,
        Some(v) if !v.is_empty() => return Some(PathBuf::from(v)),
        _ => {}
    }
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| crate::context::home_dir().map(|h| h.join(".local").join("state")))?;
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| crate::context::home_dir().map(|h| h.join("AppData").join("Local")))?;
    Some(base.join("axon").join("session"))
}

fn layout_name(k: Kind) -> &'static str {
    match k {
        Kind::Single => "single",
        Kind::Cols => "cols",
        Kind::Rows => "rows",
        Kind::Grid => "grid",
    }
}

fn layout_of(name: &str) -> Kind {
    match name {
        "cols" => Kind::Cols,
        "rows" => Kind::Rows,
        "grid" => Kind::Grid,
        _ => Kind::Single,
    }
}

/// `s` with what would break a line or a field written as `\\`, `\t`, `\n`, `\r`.
fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// The file's text for `s`.
pub fn encode(s: &Saved) -> String {
    use std::fmt::Write;
    let mut out = format!("{HEADER}\nowner {}\ncur {}\n", s.owner, s.cur);
    for g in &s.groups {
        let _ = write!(out, "group {} {} {}\t", g.color, g.collapsed as u8, layout_name(g.kind));
        escape(&g.name, &mut out);
        out.push('\n');
    }
    for w in &s.wins {
        match w.group {
            Some(g) => {
                let _ = write!(out, "win {g}\t");
            }
            None => out.push_str("win -\t"),
        }
        escape(&w.dir, &mut out);
        out.push('\t');
        escape(&w.name, &mut out);
        out.push('\n');
    }
    out
}

/// The session `text` holds; None when it is not one (another header, no windows). What is wrong inside a good file is
/// put right rather than refused: a group that does not exist is none, a colour out of range wraps, the window on screen
/// is one that exists, and only the first `MAX_WINS` windows are kept.
pub fn decode(text: &str) -> Option<Saved> {
    let mut lines = text.lines();
    if lines.next()? != HEADER {
        return None;
    }
    let mut s = Saved::default();
    for line in lines {
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        match kind {
            "owner" => s.owner = rest.trim().parse().unwrap_or(0),
            "cur" => s.cur = rest.trim().parse().unwrap_or(0),
            "group" => {
                let (head, name) = rest.split_once('\t').unwrap_or((rest, ""));
                let mut f = head.split(' ');
                let color: usize = f.next().and_then(|c| c.parse().ok()).unwrap_or(0);
                s.groups.push(SavedGroup { name: unescape(name), color: (color % COLORS) as u8, collapsed: f.next() == Some("1"), kind: layout_of(f.next().unwrap_or("")) });
            }
            "win" if s.wins.len() < MAX_WINS => {
                let mut f = rest.split('\t');
                let group = f.next().and_then(|g| g.parse().ok());
                s.wins.push(SavedWin { group, dir: unescape(f.next().unwrap_or("")), name: unescape(f.next().unwrap_or("")) });
            }
            _ => {}
        }
    }
    if s.wins.is_empty() {
        return None;
    }
    for w in &mut s.wins {
        w.group = w.group.filter(|&g| g < s.groups.len());
    }
    s.cur = s.cur.min(s.wins.len() - 1);
    Some(s)
}

/// Reads the session, if there is a good one.
pub fn load() -> Option<Saved> {
    let path = path()?;
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() > MAX_BYTES {
        return None;
    }
    decode(&std::fs::read_to_string(path).ok()?)
}

/// Writes `s`: to a file beside the session, then renamed over it, so a crash in the middle leaves the old one whole.
pub fn save(s: &Saved) -> std::io::Result<()> {
    let Some(path) = path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, encode(s))?;
    std::fs::rename(&tmp, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Whether the axon `me` may take over the session `owner` last wrote: nobody owned it, it is `me` already, or the
/// owner is gone (`alive` says whether a process is). A session whose owner still runs is that axon's, not for a second one.
pub fn claims(owner: u32, me: u32, alive: impl Fn(u32) -> bool) -> bool {
    owner == 0 || owner == me || !alive(owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Saved {
        Saved {
            owner: 4242,
            cur: 2,
            groups: vec![
                SavedGroup { name: "axon".into(), color: 3, collapsed: false, kind: Kind::Cols },
                SavedGroup { name: "docs and notes".into(), color: 0, collapsed: true, kind: Kind::Single },
            ],
            wins: vec![
                SavedWin { dir: "/home/ana/lab/axon".into(), name: String::new(), group: Some(0) },
                SavedWin { dir: "/home/ana/lab/axon/src".into(), name: "src".into(), group: Some(0) },
                SavedWin { dir: "/home/ana/notes".into(), name: String::new(), group: Some(1) },
                SavedWin { dir: "/tmp".into(), name: "scratch".into(), group: None },
            ],
        }
    }

    #[test]
    fn a_session_is_what_it_was_after_a_round_trip() {
        assert_eq!(decode(&encode(&sample())), Some(sample()));
    }

    #[test]
    fn names_and_folders_with_odd_characters_survive() {
        let mut s = sample();
        s.wins[0].dir = "/home/ana/with\ttab/and\nnewline/back\\slash\r".into();
        s.wins[1].name = "a \\t literal and\ttab".into();
        s.groups[0].name = "multi\nline".into();
        let text = encode(&s);
        assert_eq!(text.lines().count(), 1 + 2 + 2 + 4, "every record stays on its own line");
        assert_eq!(decode(&text), Some(s));
    }

    #[test]
    fn a_file_that_is_not_a_session_is_refused() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("hello\n"), None);
        assert_eq!(decode("axon-session 2\nwin -\t/tmp\t\n"), None, "another format");
        assert_eq!(decode("axon-session 1\nowner 1\ncur 0\n"), None, "no windows");
        assert_eq!(decode("win -\t/tmp\t\n"), None, "no header");
    }

    #[test]
    fn what_is_wrong_in_a_good_file_is_put_right() {
        let s = decode("axon-session 1\ncur 99\ngroup 11 1 grid\tg\nwin 7\t/a\t\nwin 0\t/b\t\nwin x\t/c\nfuture thing\nwin\n").unwrap();
        assert_eq!(s.cur, 3, "the window on screen exists");
        assert_eq!(s.groups[0].color, 3, "a colour wraps");
        assert_eq!(s.groups[0].kind, Kind::Grid);
        assert_eq!(s.wins.iter().map(|w| w.group).collect::<Vec<_>>(), [None, Some(0), None, None], "a group that does not exist is none");
        assert_eq!(s.wins.len(), 4, "the unknown line is ignored, the bare `win` is an empty window");
        assert_eq!(s.owner, 0);
        let many: String = std::iter::once("axon-session 1\n".to_string()).chain((0..100).map(|i| format!("win -\t/d{i}\t\n"))).collect();
        assert_eq!(decode(&many).unwrap().wins.len(), MAX_WINS);
    }

    #[test]
    fn a_second_axon_leaves_a_running_ones_session_alone() {
        let alive = |p: u32| p == 100;
        assert!(claims(0, 7, alive), "nobody owned it");
        assert!(claims(7, 7, alive), "it is mine");
        assert!(claims(55, 7, alive), "its owner is gone");
        assert!(!claims(100, 7, alive), "its owner runs");
    }

    #[test]
    fn the_file_is_written_whole_and_read_back() {
        let dir = std::env::temp_dir().join(format!("axon-restore-test-{}", std::process::id()));
        let file = dir.join("nested").join("session");
        // The environment is the process's: this test is the only one that sets it.
        unsafe { std::env::set_var("AXON_SESSION", &file) };
        assert_eq!(path(), Some(file.clone()));
        assert_eq!(load(), None, "nothing yet");
        save(&sample()).unwrap();
        assert_eq!(load(), Some(sample()));
        let mut s = sample();
        s.wins.truncate(1);
        s.groups.clear();
        s.wins[0].group = None;
        save(&s).unwrap();
        assert_eq!(load(), Some(Saved { cur: 0, ..s }), "saving again replaces it");
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1, "no temporary file is left");
        unsafe { std::env::set_var("AXON_SESSION", "off") };
        assert_eq!(path(), None);
        save(&sample()).unwrap();
        assert_eq!(load(), None, "off keeps nothing");
        unsafe { std::env::remove_var("AXON_SESSION") };
        let _ = std::fs::remove_dir_all(dir);
    }
}
