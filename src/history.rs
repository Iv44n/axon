//! What each prompt said, and how its command ended: the data behind the status on the right of a
//! prompt's chips (the exit mark and how long the command ran).
//!
//! The shell's hook (`context.rs`) talks to axon through window-title escapes that the terminal
//! thread recognises and swallows (`parse`): `S` when a command starts (bash adds a token, which arms its own
//! `DEBUG` trap), `L <line>` with the command line that starts (the window is labelled with its first word while it
//! runs), `E <n> <status>` at every prompt, where `n` counts the prompts. The start and the next prompt are timed when the PTY thread
//! parses them, so the time is the command's, not the UI's. The prompt's reserved line carries its `n`
//! in the colour of its concealed cell (`view::prompt_rows`), so a row on screen, in the scrollback or
//! after a scroll finds its record again. A bounded ring: the oldest prompts forget their chips.
use std::{collections::VecDeque, time::Instant};

use crate::context::Context;

/// How many prompts are remembered.
const KEEP: usize = 256;

/// What the shell's hook says, as a window title that starts with this.
const PREFIX: &str = "axon!hook ";

/// How many characters of a command's name a window's label keeps.
const LABEL_MAX: usize = 24;

/// What a hook escape means.
#[derive(Clone, Debug, PartialEq)]
pub enum Hook {
    /// A command is about to run.
    Start,
    /// The command that runs, as a window's label: its first word (`cargo` of `cargo build --release`).
    Label(String),
    /// A new prompt, number `n`; the command before it ended with `status`.
    Prompt { n: u32, status: i32 },
}

/// The hook a window title carries, if it is one.
pub fn parse(title: &str) -> Option<Hook> {
    let rest = title.strip_prefix(PREFIX)?;
    let (kind, arg) = rest.split_once(' ').unwrap_or((rest, ""));
    match kind {
        // What follows an `S` is bash's arming token: nothing to read.
        "S" if !arg.contains(' ') => Some(Hook::Start),
        "L" => Some(Hook::Label(command_word(arg).to_string())),
        "E" => match arg.split(' ').collect::<Vec<_>>()[..] {
            [n, status] => Some(Hook::Prompt { n: n.parse().ok()?, status: status.parse().ok()? }),
            _ => None,
        },
        _ => None,
    }
}

/// The word of a command line that names what runs: the first that is not a `VAR=value` assignment, without its
/// directory, cut to `LABEL_MAX` characters. A whole line would put arguments (a password given on the command line)
/// in the window's title, which other programs can read.
pub fn command_word(line: &str) -> &str {
    let word = line.split_whitespace().find(|w| !w.contains('=')).unwrap_or("");
    let name = word.rsplit('/').find(|n| !n.is_empty()).unwrap_or(word);
    match name.char_indices().nth(LABEL_MAX) {
        Some((i, _)) => &name[..i],
        None => name,
    }
}

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub status: i32,
    pub ms: u64,
}

/// One prompt: its number, the chips it showed (frozen when the next prompt came) and how its command ended.
#[derive(Clone, Debug)]
pub struct Record {
    pub n: u32,
    pub ctx: Context,
    pub run: Option<Run>,
    /// Changes whenever the record does, so a row drawn from it knows to be drawn again.
    pub rev: u32,
}

#[derive(Default)]
pub struct History {
    recs: VecDeque<Record>,
    /// When the command now running started.
    started: Option<Instant>,
    /// What the command now running is called (`command_word`), empty when the shell has not said.
    label: String,
    rev: u32,
}

impl History {
    /// A command is about to run, at `at`.
    pub fn start(&mut self, at: Instant) {
        self.started = Some(at);
        self.label.clear();
    }

    /// The command that started is called `word`; said when none runs, it is ignored.
    pub fn set_label(&mut self, word: &str) {
        if self.started.is_some() {
            self.label.clear();
            self.label.push_str(word);
        }
    }

    /// What the command now running is called, while one runs and the shell has said.
    pub fn command(&self) -> Option<&str> {
        self.started.filter(|_| !self.label.is_empty()).map(|_| self.label.as_str())
    }

    /// A prompt, number `n`, at `at`: the command before it (if one ran) ended with `status`, and `ctx`
    /// is what the new prompt shows for now.
    pub fn prompt(&mut self, n: u32, status: i32, at: Instant, ctx: &Context) {
        self.label.clear();
        if let (Some(t0), Some(last)) = (self.started.take(), self.recs.back_mut()) {
            last.run = Some(Run { status, ms: at.saturating_duration_since(t0).as_millis() as u64 });
            self.rev += 1;
            last.rev = self.rev;
        }
        // A shell that started over (a new `bash` inside) counts from 1 again: the old rows keep their
        // records, and the new ones must not find them, so they are dropped.
        if self.recs.back().is_some_and(|r| r.n >= n) {
            self.recs.clear();
        }
        self.rev += 1;
        self.recs.push_back(Record { n, ctx: ctx.clone(), run: None, rev: self.rev });
        if self.recs.len() > KEEP {
            self.recs.pop_front();
        }
    }

    /// The newest prompt shows `ctx` until the next one comes. True when that changed the record.
    pub fn set_ctx(&mut self, ctx: &Context) -> bool {
        match self.recs.back_mut() {
            Some(r) if r.ctx != *ctx => {
                r.ctx = ctx.clone();
                self.rev += 1;
                r.rev = self.rev;
                true
            }
            _ => false,
        }
    }

    pub fn get(&self, n: u32) -> Option<&Record> {
        // Prompts are numbered in order: look from the newest, which is where the screen is.
        self.recs.iter().rev().find(|r| r.n == n)
    }

    /// The newest prompt.
    pub fn latest(&self) -> Option<&Record> {
        self.recs.back()
    }

    /// A command is running: it started and no prompt has come since.
    pub fn running(&self) -> bool {
        self.started.is_some()
    }

    /// The last command that ran failed (interrupting one with Ctrl+C, status 130, is not a failure).
    pub fn failed(&self) -> bool {
        self.recs.iter().rev().find_map(|r| r.run).is_some_and(|r| r.status != 0 && r.status != 130)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.recs.len()
    }
}

/// `ms` as the status shows it: `12ms`, `1.4s`, `2m 5s`.
pub fn duration_text(ms: u64) -> String {
    match ms {
        0..=999 => format!("{}ms", ms.max(1)),
        1000..=59_999 => format!("{:.1}s", ms as f64 / 1000.0),
        _ => format!("{}m {}s", ms / 60_000, ms % 60_000 / 1000),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(dir: &str) -> Context {
        let mut c = Context::default();
        c.text[crate::config::Chip::Dir as usize] = dir.to_string();
        c
    }

    #[test]
    fn hooks_are_recognised_in_titles_and_nothing_else_is() {
        assert_eq!(parse("axon!hook S"), Some(Hook::Start));
        assert_eq!(parse("axon!hook E 12 0"), Some(Hook::Prompt { n: 12, status: 0 }));
        assert_eq!(parse("axon!hook E 3 127"), Some(Hook::Prompt { n: 3, status: 127 }));
        for t in ["user@host: ~", "axon!hook", "axon!hook E 1", "axon!hook E x 0", "axon!hook S 1 2", "axon!hook X", "axon!hook E 1 0 9", ""] {
            assert_eq!(parse(t), None, "{t:?}");
        }
        // Bash's start carries the token that arms its trap.
        assert_eq!(parse("axon!hook S 1"), Some(Hook::Start));
    }

    #[test]
    fn the_label_is_the_commands_first_word_without_its_directory_or_assignments() {
        for (line, word) in [
            ("cargo build --release", "cargo"),
            ("  sleep 90", "sleep"),
            ("FOO=1 BAR=x npm run dev", "npm"),
            ("/usr/bin/sleep 5", "sleep"),
            ("./run.sh --token=abc", "run.sh"),
            ("\tvim\tfile", "vim"),
            ("", ""),
            ("A=1", ""),
            ("a_command_name_that_is_far_too_long_for_a_tab", "a_command_name_that_is_f"),
        ] {
            assert_eq!(command_word(line), word, "{line:?}");
        }
        assert_eq!(parse("axon!hook L cargo build"), Some(Hook::Label("cargo".into())));
        assert_eq!(parse("axon!hook L"), Some(Hook::Label(String::new())));
    }

    #[test]
    fn a_running_command_has_its_label_until_the_next_prompt() {
        let (mut h, t0) = (History::default(), Instant::now());
        h.prompt(1, 0, t0, &ctx("~/a"));
        assert_eq!(h.command(), None);
        h.set_label("sleep");
        assert_eq!(h.command(), None, "no command runs: a stray label is ignored");
        h.start(t0);
        assert_eq!(h.command(), None, "the shell has not said yet");
        h.set_label("sleep");
        assert_eq!(h.command(), Some("sleep"));
        h.prompt(2, 0, t0 + Duration::from_millis(5), &ctx("~/a"));
        assert_eq!(h.command(), None);
        // The next command does not inherit it.
        h.start(t0 + Duration::from_millis(9));
        assert_eq!(h.command(), None);
    }

    #[test]
    fn a_command_is_timed_from_its_start_to_the_next_prompt() {
        let (mut h, t0) = (History::default(), Instant::now());
        h.prompt(1, 0, t0, &ctx("~/a"));
        assert_eq!(h.get(1).unwrap().run, None, "no command has run yet");
        h.start(t0 + Duration::from_millis(100));
        h.prompt(2, 127, t0 + Duration::from_millis(1600), &ctx("~/b"));
        assert_eq!(h.get(1).unwrap().run, Some(Run { status: 127, ms: 1500 }));
        assert_eq!(h.get(2).unwrap().run, None);
        // A bare Enter: a prompt without a start leaves the record as it is.
        h.prompt(3, 0, t0 + Duration::from_secs(5), &ctx("~/b"));
        assert_eq!(h.get(2).unwrap().run, None);
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn a_running_and_a_failed_command_are_told_apart_for_the_panels_status() {
        let (mut h, t0) = (History::default(), Instant::now());
        assert!(!h.running() && !h.failed());
        h.prompt(1, 0, t0, &ctx("~/a"));
        h.start(t0);
        assert!(h.running() && !h.failed(), "it runs, and nothing has failed yet");
        h.prompt(2, 127, t0 + Duration::from_millis(50), &ctx("~/a"));
        assert!(!h.running() && h.failed());
        // The status is of the last command that ran: a bare Enter does not clear it, a success does.
        h.prompt(3, 0, t0 + Duration::from_millis(60), &ctx("~/a"));
        assert!(h.failed());
        h.start(t0 + Duration::from_millis(70));
        h.prompt(4, 0, t0 + Duration::from_millis(80), &ctx("~/a"));
        assert!(!h.failed());
        // Ctrl+C is not a failure.
        h.start(t0 + Duration::from_millis(90));
        h.prompt(5, 130, t0 + Duration::from_millis(95), &ctx("~/a"));
        assert!(!h.failed());
    }

    #[test]
    fn the_newest_prompt_follows_the_chips_and_the_older_ones_keep_theirs() {
        let (mut h, t0) = (History::default(), Instant::now());
        h.prompt(1, 0, t0, &ctx("~/a"));
        let rev = h.get(1).unwrap().rev;
        assert!(!h.set_ctx(&ctx("~/a")), "the same chips change nothing");
        assert!(h.set_ctx(&ctx("~/b")));
        assert_ne!(h.get(1).unwrap().rev, rev, "the row has to be drawn again");
        h.prompt(2, 0, t0, &ctx("~/b"));
        h.set_ctx(&ctx("~/c"));
        assert_eq!(h.get(1).unwrap().ctx.get(crate::config::Chip::Dir), "~/b");
        assert_eq!(h.get(2).unwrap().ctx.get(crate::config::Chip::Dir), "~/c");
        assert_eq!(h.latest().unwrap().n, 2);
    }

    #[test]
    fn the_ring_forgets_the_oldest_and_a_shell_that_starts_over_forgets_all() {
        let (mut h, t0) = (History::default(), Instant::now());
        for n in 1..=(KEEP as u32 + 10) {
            h.prompt(n, 0, t0, &Context::default());
        }
        assert_eq!(h.len(), KEEP);
        assert!(h.get(10).is_none() && h.get(11).is_some());
        h.prompt(1, 0, t0, &ctx("~/new"));
        assert_eq!(h.len(), 1, "a nested shell counts from 1 again");
        assert!(h.get(11).is_none());
    }

    #[test]
    fn durations_read_like_the_reference() {
        assert_eq!(duration_text(0), "1ms");
        assert_eq!(duration_text(12), "12ms");
        assert_eq!(duration_text(999), "999ms");
        assert_eq!(duration_text(1000), "1.0s");
        assert_eq!(duration_text(1450), "1.4s");
        assert_eq!(duration_text(125_000), "2m 5s");
    }
}
