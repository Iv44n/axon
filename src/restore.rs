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
//! group <colour> <folded 0|1> <layout>[ cols=<w>,<w>...][ rows=<w>,<w>...]\t<name>
//! win <group index or ->\t<folder>\t<name>
//! ```
//!
//! What a shell was running is not kept: only where it was. Names and folders are escaped (`\\`, `\t`, `\n`, `\r`), so
//! whatever a folder is called fits on its line. Unknown lines are ignored, so a newer file still reads in an older axon;
//! so are the fields after a group's layout (the sizes of its split's columns and rows, ADR 0045, written only when the
//! user resized them), which an axon from before them never reads.
//!
//! One axon owns the file at a time (`owner`, its process id): a second axon opened while the first runs starts fresh and
//! leaves the first's session alone, instead of opening a copy of it. The pure part is here (the format, the rules); the
//! app builds the snapshot and puts it back (`wlapp.rs`).
use std::path::PathBuf;

use crate::{
    groups::COLORS,
    panes::{Axis, Kind, MAX_TRACKS, Tracks},
};

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
    pub tracks: Tracks,
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
    path_named("session")
}

/// A file of axon's in the state directory: `$XDG_STATE_HOME/axon/<name>` (`~/.local/state/axon/<name>`), or
/// `%LOCALAPPDATA%\axon\<name>` on Windows. None where there is no home.
pub fn path_named(name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| crate::context::home_dir().map(|h| h.join(".local").join("state")))?;
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| crate::context::home_dir().map(|h| h.join("AppData").join("Local")))?;
    Some(base.join("axon").join(name))
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

/// The names the sizes of a split's columns and rows go by in a `group` line.
const AXES: [(Axis, &str); 2] = [(Axis::Cols, "cols="), (Axis::Rows, "rows=")];

/// The sizes of the axes of `t` that have their own, as ` cols=612,400`, ` rows=...`.
fn write_tracks(t: &Tracks, out: &mut String) {
    use std::fmt::Write;
    for (a, name) in AXES {
        let n = t.n[a as usize] as usize;
        if t.sized(a, n) {
            out.push(' ');
            out.push_str(name);
            for (k, w) in t.weights(a, n).iter().enumerate() {
                let _ = write!(out, "{}{w}", if k == 0 { "" } else { "," });
            }
        }
    }
}

/// The sizes the fields `f` (after a group's layout) give; an axis that is not there, or not well formed, is even.
fn read_tracks<'a>(f: impl Iterator<Item = &'a str>) -> Tracks {
    let mut t = Tracks::EVEN;
    for field in f {
        let Some((a, list)) = AXES.iter().find_map(|&(a, name)| field.strip_prefix(name).map(|l| (a, l))) else { continue };
        let mut w = [0u16; MAX_TRACKS];
        let mut n = 0;
        for v in list.split(',') {
            match v.parse::<u16>() {
                Ok(x) if x > 0 && n < MAX_TRACKS => (w[n], n) = (x, n + 1),
                _ => {
                    n = 0;
                    break;
                }
            }
        }
        if n >= 2 {
            (t.n[a as usize], t.w[a as usize]) = (n as u8, w);
        }
    }
    t
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
        let _ = write!(out, "group {} {} {}", g.color, g.collapsed as u8, layout_name(g.kind));
        write_tracks(&g.tracks, &mut out);
        out.push('\t');
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
                let (collapsed, kind) = (f.next() == Some("1"), layout_of(f.next().unwrap_or("")));
                s.groups.push(SavedGroup { name: unescape(name), color: (color % COLORS) as u8, collapsed, kind, tracks: read_tracks(f) });
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

    /// Sizes for a split of two columns (612 and 400 px when set) and three rows.
    fn resized() -> Tracks {
        let mut t = Tracks { n: [2, 3], ..Tracks::EVEN };
        t.w[0][..2].copy_from_slice(&[612, 400]);
        t.w[1][..3].copy_from_slice(&[100, 250, 120]);
        t
    }

    fn sample() -> Saved {
        Saved {
            owner: 4242,
            cur: 2,
            groups: vec![
                SavedGroup { name: "axon".into(), color: 3, collapsed: false, kind: Kind::Grid, tracks: resized() },
                SavedGroup { name: "docs and notes".into(), color: 0, collapsed: true, kind: Kind::Single, tracks: Tracks::EVEN },
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
    fn the_sizes_of_a_split_are_extra_fields_an_older_axon_skips() {
        let text = encode(&sample());
        assert!(text.contains("group 3 0 grid cols=612,400 rows=100,250,120\taxon\n"), "{text}");
        assert!(text.contains("group 0 1 single\tdocs and notes\n"), "an even split writes nothing");
        // A file from before the sizes: even.
        let old = decode("axon-session 1\ngroup 1 0 cols\tg\nwin 0\t/a\t\n").unwrap();
        assert_eq!(old.groups[0].tracks, Tracks::EVEN);
        // What an older axon reads of the line is the same three fields, the name after the tab.
        let head = text.lines().find(|l| l.starts_with("group 3")).unwrap().split_once('\t').unwrap().0;
        assert_eq!(head.split(' ').take(4).collect::<Vec<_>>(), ["group", "3", "0", "grid"]);
        // Damaged sizes are even; one good axis is kept without the other; an unknown field is skipped.
        for bad in ["cols=1", "cols=0,5", "cols=a,b", "cols=", "cols=70000,1", "cols=1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17"] {
            let s = decode(&format!("axon-session 1\ngroup 1 0 cols {bad}\tg\nwin 0\t/a\t\n")).unwrap();
            assert_eq!(s.groups[0].tracks, Tracks::EVEN, "{bad}");
        }
        let s = decode("axon-session 1\ngroup 1 0 grid later=9 rows=5,7 cols=x\tg\nwin 0\t/a\t\n").unwrap();
        assert!(s.groups[0].tracks.sized(Axis::Rows, 2) && !s.groups[0].tracks.sized(Axis::Cols, 2));
        assert_eq!(s.groups[0].tracks.weights(Axis::Rows, 2), [5, 7]);
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
