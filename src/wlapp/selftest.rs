//! `AXON_SELFTEST=<folder>` (ADR 0039): the app tests itself, for CI, where nobody is there to look. It runs the real
//! window and shell: types a command through the same path a key takes, waits for what the shell prints, moves to
//! another directory, puts text on the clipboard and reads it back, maximizes and restores the window, then writes
//! `selftest.txt` (what happened, and `RESULT: ok` or `RESULT: FAILED`) and `frame.argb` (the window's pixels: the
//! width and the height as two little-endian `u32`, then each pixel's `u32`) into the folder and exits with 0 or 1.
//! Nothing here runs unless the variable is set.
use std::{
    fmt::Write as _,
    path::PathBuf,
    time::{Duration, Instant},
};

use alacritty_terminal::{
    grid::Dimensions,
    index::{Column, Line, Point},
};

use super::*;

/// A command whose output only exists if the shell ran it: the sum is the shell's, not part of what is typed.
const COMMAND: &str = if cfg!(windows) { "echo (\"axon-selftest-\" + 6 * 7)" } else { "echo axon-selftest-$((6*7))" };
const MARK: &str = "axon-selftest-42";
/// Where the second command takes the shell, and what the pane's directory ends in once it is there.
const CD: &str = if cfg!(windows) { "Set-Location $env:TEMP" } else { "cd /tmp" };
const CD_ENDS: [&str; 2] = ["temp", "tmp"];
/// How long the whole run may take before it is a failure.
const LIMIT: Duration = Duration::from_secs(60);
const STEP: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    /// Let the shell print its first prompt and the window its first frame.
    Start,
    WaitOutput,
    WaitDir,
    WaitMaximized,
    WaitRestored,
}

pub struct Run {
    dir: PathBuf,
    start: Instant,
    /// When `tick` looks again.
    pub next: Instant,
    phase: Phase,
    phase_at: Instant,
    log: String,
}

impl Run {
    /// A run when `AXON_SELFTEST` names a folder, none otherwise.
    pub fn from_env() -> Option<Run> {
        let dir = std::env::var_os("AXON_SELFTEST").filter(|d| !d.is_empty())?;
        let now = Instant::now();
        Some(Run { dir: PathBuf::from(dir), start: now, next: now + Duration::from_secs(2), phase: Phase::Start, phase_at: now, log: String::new() })
    }
}

impl App {
    /// The text of the pane on screen, history and all.
    fn selftest_text(&self) -> String {
        let t = self.wins[self.cur].pane().term.lock();
        let g = t.grid();
        let (top, bottom) = (Line(-(g.history_size() as i32)), Line(g.screen_lines() as i32 - 1));
        t.bounds_to_string(Point::new(top, Column(0)), Point::new(bottom, Column(g.columns().saturating_sub(1))))
    }

    /// Types `line` and Enter as keys, as a keyboard would.
    fn selftest_type(&mut self, line: &str) {
        for c in line.chars() {
            self.handle_key(&Key { sym: c as u32, text: c.to_string(), mods: Mods::default() });
        }
        self.handle_key(&Key { sym: 0xff0d, text: "\r".into(), mods: Mods::default() });
    }

    /// One step of the run: called by the loop whenever `Run::next` is due.
    pub(super) fn tick_selftest(&mut self) {
        let Some(mut run) = self.selftest.take() else { return };
        let now = Instant::now();
        if now < run.next {
            self.selftest = Some(run);
            return;
        }
        run.next = now + STEP;
        let ms = |t: Instant| now.duration_since(t).as_millis();
        let phase = run.phase;
        match phase {
            Phase::Start => {
                let _ = writeln!(run.log, "window: {}x{} (grid {}x{}), shell: {}", self.w, self.h, self.cols, self.rows, session::shell_path());
                self.selftest_type(COMMAND);
                (run.phase, run.phase_at) = (Phase::WaitOutput, now);
            }
            Phase::WaitOutput if self.selftest_text().contains(MARK) => {
                let _ = writeln!(run.log, "shell: ran `{COMMAND}` and printed {MARK} after {} ms", ms(run.phase_at));
                self.selftest_type(CD);
                (run.phase, run.phase_at) = (Phase::WaitDir, now);
            }
            Phase::WaitDir if CD_ENDS.iter().any(|e| self.wins[self.cur].pane().dir.to_lowercase().ends_with(e)) => {
                let _ = writeln!(run.log, "directory: the pane says {:?} after {} ms", self.wins[self.cur].pane().dir, ms(run.phase_at));
                let text = "axon-clip \u{e9}\u{4e2d}\nsecond line";
                self.plat.set_clipboard(text);
                match self.plat.clipboard_text() {
                    Some(back) if back == text => {
                        let _ = writeln!(run.log, "clipboard: written and read back");
                    }
                    other => return self.selftest_end(run, Some(format!("clipboard: wrote {text:?}, read {other:?}"))),
                }
                self.plat.toggle_maximized();
                (run.phase, run.phase_at) = (Phase::WaitMaximized, now);
            }
            Phase::WaitMaximized if self.ui.maximized => {
                let _ = writeln!(run.log, "maximize: now {}x{} after {} ms", self.w, self.h, ms(run.phase_at));
                self.plat.toggle_maximized();
                (run.phase, run.phase_at) = (Phase::WaitRestored, now);
            }
            Phase::WaitRestored if !self.ui.maximized => {
                let _ = writeln!(run.log, "restore: back to {}x{} after {} ms", self.w, self.h, ms(run.phase_at));
                return self.selftest_end(run, None);
            }
            _ => {}
        }
        if now.duration_since(run.start) > LIMIT {
            let what = format!("no progress past {phase:?} in {} s", LIMIT.as_secs());
            return self.selftest_end(run, Some(what));
        }
        self.selftest = Some(run);
    }

    /// Writes the report and the frame, and ends the process: 0 when `failure` is none.
    fn selftest_end(&mut self, mut run: Run, failure: Option<String>) {
        let (ml, mt, mb) = chrome::margins(self.ui.maximized);
        let (w, h) = (self.w as usize + 2 * ml, self.h as usize + mt + mb);
        if let Some(fb) = self.plat.framebuffer(w, h) {
            let lit = fb.iter().filter(|&&p| p != 0).count();
            let _ = writeln!(run.log, "frame: {w}x{h}, {lit} of {} pixels drawn", fb.len());
            let mut raw = Vec::with_capacity(8 + fb.len() * 4);
            raw.extend((w as u32).to_le_bytes());
            raw.extend((h as u32).to_le_bytes());
            raw.extend(fb.iter().flat_map(|p| p.to_le_bytes()));
            let _ = std::fs::write(run.dir.join("frame.argb"), raw);
        }
        let ok = failure.is_none();
        match failure {
            Some(f) => {
                let _ = writeln!(run.log, "FAILED: {f}\n--- the pane's text ---\n{}", self.selftest_text());
            }
            None => {}
        }
        let _ = writeln!(run.log, "RESULT: {}", if ok { "ok" } else { "FAILED" });
        let _ = std::fs::create_dir_all(&run.dir);
        let _ = std::fs::write(run.dir.join("selftest.txt"), &run.log);
        std::process::exit(if ok { 0 } else { 1 });
    }
}
