//! The terminal session: `Term` plus its PTY thread, and the grid geometry for a window size.
//! Independent of the windowing backend; the listener is how the PTY thread reaches the UI.
use std::{
    collections::HashMap,
    ffi::OsStr,
    path::Path,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use alacritty_terminal::{
    Term,
    event::{EventListener, WindowSize},
    event_loop::{EventLoop as PtyLoop, Msg, Notifier},
    grid::Dimensions,
    sync::FairMutex,
    term::{Config as TermConfig, Osc52},
    tty,
    vte::ansi::{CursorShape, CursorStyle},
};

use crate::config::CursorKind;

pub struct Size {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize { self.rows }
    fn screen_lines(&self) -> usize { self.rows }
    fn columns(&self) -> usize { self.cols }
}

/// Terminal policy. OSC 52 may only write the clipboard: letting program output read it would hand
/// the clipboard to anything that can print to the terminal (a remote host, a `cat`ed file). Set
/// explicitly so it does not hang on a library default.
fn term_config(cursor: CursorKind) -> TermConfig {
    let shape = match cursor {
        CursorKind::Block => CursorShape::Block,
        CursorKind::Bar => CursorShape::Beam,
        CursorKind::Underline => CursorShape::Underline,
    };
    TermConfig { osc52: Osc52::OnlyCopy, default_cursor_style: CursorStyle { shape, blinking: false }, ..TermConfig::default() }
}

/// Changes the cursor shape programs get until they ask for another.
pub fn set_cursor<L: EventListener>(term: &FairMutex<Term<L>>, cursor: CursorKind) {
    term.lock().set_options(term_config(cursor));
}

/// Columns and rows that fit in a `w` x `h` pixel window with `pad` pixels of margin left and right,
/// `top` pixels above and `bottom` below.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub fn grid_size(w: u32, h: u32, cw: u32, ch: u32, pad: u32, top: u32, bottom: u32) -> (usize, usize) {
    let cols = (w.saturating_sub(2 * pad) / cw).max(2) as usize;
    let rows = (h.saturating_sub(top + bottom) / ch).max(2) as usize;
    (cols, rows)
}

fn window_size(cols: usize, rows: usize, cw: u32, ch: u32) -> WindowSize {
    WindowSize { num_lines: rows as u16, num_cols: cols as u16, cell_width: cw as u16, cell_height: ch as u16 }
}

/// How long `ScopeProbe::works` waits for `systemd-run`: a user manager that does not answer must not hold the
/// first window back for the bus's own timeout (25 s).
const SCOPE_PROBE_WAIT: Duration = Duration::from_secs(2);

/// What `systemd-run` is given to start `shell` in a scope of its own (`[windows] scope`, ADR 0038): a transient
/// unit in `app.slice`, named as the freedesktop convention names an application's (`app-axon-<axon's pid>w<window>`),
/// so the system monitor and `systemd-cgtop` list it as what it is. `--scope` puts the process in the scope and then
/// `exec`s the command in that same process, so the shell keeps the pid axon reads `/proc` by. `--quiet` keeps the
/// "Running scope as unit" notice off the terminal, `--collect` lets the unit go even when it ends in failure.
/// systemd-run reads `$$` as an escaped `$`, so the program's own `$` are doubled.
fn scope_args(shell: &str, axon: u32, window: u64) -> Vec<String> {
    vec![
        "--user".into(),
        "--scope".into(),
        "--collect".into(),
        "--quiet".into(),
        "--slice=app.slice".into(),
        format!("--unit=app-axon-{axon}w{window}.scope"),
        format!("--description=axon window {window}"),
        "--".into(),
        shell.replace('$', "$$"),
    ]
}

/// The shell started through `systemd-run` (`scope_args`); None when `$SHELL` says nothing, and the PTY layer
/// takes the user's shell from the password file as it always did.
fn scoped_shell(window: u64) -> Option<tty::Shell> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty())?;
    Some(tty::Shell::new("systemd-run".into(), scope_args(&shell, std::process::id(), window)))
}

/// The shell the user's environment names, as a path: `$SHELL`. On Windows `AXON_SHELL`, else PowerShell 7
/// (`pwsh`) when it is installed, else Windows PowerShell, else `%COMSPEC%`; empty when nothing says.
pub fn shell_path() -> String {
    #[cfg(unix)]
    return std::env::var("SHELL").unwrap_or_default();
    #[cfg(windows)]
    return windows_shell();
}

/// The shell axon starts when it is not given a scope: the PTY layer's own choice on Linux (the user's from the
/// password file), the one `windows_shell` finds on Windows.
fn own_shell() -> Option<tty::Shell> {
    #[cfg(unix)]
    return None;
    #[cfg(windows)]
    {
        let shell = windows_shell();
        // PowerShell is started with axon's hook (`context::POWERSHELL_HOOK`) run after the profile, then stays interactive.
        let args = if crate::context::is_powershell(&shell) {
            vec!["-NoLogo".into(), "-NoExit".into(), "-Command".into(), crate::context::POWERSHELL_HOOK.to_string()]
        } else {
            Vec::new()
        };
        return Some(tty::Shell::new(quote_program(&shell), args));
    }
}

/// A program path as a command line carries it: quoted when it has a space.
#[cfg(windows)]
fn quote_program(p: &str) -> String {
    if p.contains(' ') && !p.starts_with('"') { format!("\"{p}\"") } else { p.to_string() }
}

/// What a Windows machine offers, in the order a terminal should prefer: `AXON_SHELL`, `pwsh.exe` (PowerShell 7, on
/// the `PATH` or in its usual folder), the `powershell.exe` every Windows has, `%COMSPEC%`.
#[cfg(windows)]
fn windows_shell() -> String {
    let on_path = |name: &str| std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|f| f.is_file()));
    let env_dir = |k: &str, rest: &str| std::env::var_os(k).map(|d| Path::new(&d).join(rest));
    if let Some(s) = std::env::var("AXON_SHELL").ok().filter(|s| !s.is_empty()) {
        return s;
    }
    let found = on_path("pwsh.exe")
        .or_else(|| env_dir("ProgramFiles", "PowerShell\\7\\pwsh.exe").filter(|f| f.is_file()))
        .or_else(|| on_path("powershell.exe"))
        .or_else(|| env_dir("SystemRoot", "System32\\WindowsPowerShell\\v1.0\\powershell.exe").filter(|f| f.is_file()));
    found.map(|f| f.to_string_lossy().into_owned()).or_else(|| std::env::var("COMSPEC").ok()).unwrap_or_else(|| "powershell.exe".into())
}

/// Whether this machine can give a shell a scope: systemd is the init system (`booted` is its `/run/systemd/system`),
/// its user manager listens (a private socket or the user bus under `runtime`) and `systemd-run` is on the `path`.
fn scope_possible(booted: &Path, runtime: Option<&OsStr>, path: Option<&OsStr>) -> bool {
    let Some(run) = runtime.map(Path::new) else { return false };
    booted.is_dir()
        && (run.join("systemd/private").exists() || run.join("bus").exists())
        && path.is_some_and(|p| std::env::split_paths(p).any(|dir| dir.join("systemd-run").is_file()))
}

/// A try of the scope, started early (`scope_probe`) so it runs while the window is being made, and awaited
/// (`works`) just before the first shell: a `systemd-run` that fails would leave a window with no shell, and the
/// last window closing quits axon, so a machine where it fails must be known before the first one.
pub struct ScopeProbe(Option<Child>);

/// Starts the probe when `wanted` and the machine could (`scope_possible`): `systemd-run` running `true` in a scope.
pub fn scope_probe(wanted: bool) -> ScopeProbe {
    let possible = wanted
        && scope_possible(
            Path::new("/run/systemd/system"),
            std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
            std::env::var_os("PATH").as_deref(),
        );
    let child = possible
        .then(|| {
            Command::new("systemd-run")
                .args(["--user", "--scope", "--collect", "--quiet", "--slice=app.slice", "--", "true"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok()
        })
        .flatten();
    ScopeProbe(child)
}

impl ScopeProbe {
    /// Whether a shell can have its scope here: the probe ended well. One that was not wanted, could not start or
    /// takes longer than `SCOPE_PROBE_WAIT` (it is killed) says no.
    pub fn works(mut self) -> bool {
        self.0.take().is_some_and(|child| ended_well(child, SCOPE_PROBE_WAIT))
    }
}

impl Drop for ScopeProbe {
    /// A probe nobody waited for (the window could not be made after all) does not outlive its use or stay a zombie.
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Whether `child` exits with success within `limit`; one still running then is killed and reaped.
fn ended_well(mut child: Child, limit: Duration) -> bool {
    let end = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(1)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Starts the shell on a new PTY and its reader thread. `id` identifies the window to the PTY layer.
/// `env` is added to the shell's environment (`context::shell_env`), `dir` is where it starts (None: the home).
/// `scope` starts the shell in a scope of its own (`ScopeProbe::works` said it can be done).
/// Also returns the shell's process id.
pub fn spawn<L: EventListener + Clone + Send + 'static>(
    listener: L,
    cols: usize,
    rows: usize,
    cw: u32,
    ch: u32,
    id: u64,
    env: HashMap<String, String>,
    cursor: CursorKind,
    dir: Option<std::path::PathBuf>,
    scope: bool,
) -> (Arc<FairMutex<Term<L>>>, Notifier, u32) {
    let term = Arc::new(FairMutex::new(Term::new(term_config(cursor), &Size { cols, rows }, listener.clone())));
    tty::setup_env();
    let shell = if scope { scoped_shell(id) } else { own_shell() };
    let opts = tty::Options {
        shell,
        working_directory: dir,
        drain_on_exit: false,
        env,
        #[cfg(windows)]
        escape_args: true,
    };
    let pty = tty::new(&opts, window_size(cols, rows, cw, ch), id).expect("pty");
    #[cfg(unix)]
    let pid = pty.child().id();
    #[cfg(windows)]
    let pid = pty.child_watcher().pid().map_or(0, |p| p.get());
    let lp = PtyLoop::new(term.clone(), listener, pty, false, false).expect("pty event loop");
    let notifier = Notifier(lp.channel());
    lp.spawn();
    (term, notifier, pid)
}

/// Tells the terminal and the PTY about a new grid size.
pub fn resize<L: EventListener>(term: &FairMutex<Term<L>>, pty: &Notifier, cols: usize, rows: usize, cw: u32, ch: u32) {
    term.lock().resize(Size { cols, rows });
    let _ = pty.0.send(Msg::Resize(window_size(cols, rows, cw, ch)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scope_is_a_transient_unit_named_after_the_app_with_the_shell_last() {
        let a = scope_args("/bin/zsh", 4242, 3);
        for want in ["--user", "--scope", "--quiet", "--collect", "--slice=app.slice", "--unit=app-axon-4242w3.scope"] {
            assert!(a.iter().any(|x| x == want), "{want} is missing from {a:?}");
        }
        assert_eq!(a[a.len() - 2..], ["--", "/bin/zsh"], "the shell comes after `--`, alone: no arguments, as the PTY layer starts it");
        assert!(a.iter().any(|x| x == "--description=axon window 3"));
        // Two windows of one axon, and two axons, never share a unit name.
        assert_ne!(scope_args("sh", 1, 0), scope_args("sh", 1, 1));
        assert_ne!(scope_args("sh", 1, 0), scope_args("sh", 2, 0));
    }

    #[test]
    fn a_dollar_in_the_shells_path_survives_systemd_runs_escaping() {
        // systemd-run turns `$$` into `$` even for a scope (measured), so `/opt/a$b/sh` has to go as `$$`.
        assert_eq!(scope_args("/opt/a$b/sh", 1, 0).last().unwrap(), "/opt/a$$b/sh");
        assert_eq!(scope_args("/bin/bash", 1, 0).last().unwrap(), "/bin/bash");
    }

    #[test]
    fn a_scope_needs_systemd_its_user_manager_and_systemd_run() {
        let root = std::env::temp_dir().join(format!("axon-scope-{}", std::process::id()));
        let (booted, run, bin, other) = (root.join("booted"), root.join("run"), root.join("bin"), root.join("other"));
        for dir in [&booted, &run, &bin, &other] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(bin.join("systemd-run"), "").unwrap();
        let path = std::env::join_paths([&other, &bin]).unwrap();
        let ok = |booted: &Path, runtime: Option<&OsStr>, path: Option<&OsStr>| scope_possible(booted, runtime, path);

        // No user manager yet: neither its private socket nor the bus.
        assert!(!ok(&booted, Some(run.as_os_str()), Some(&path)));
        // The bus alone is enough (`systemd-run --user` falls back to it), and so is the private socket.
        std::fs::write(run.join("bus"), "").unwrap();
        assert!(ok(&booted, Some(run.as_os_str()), Some(&path)));
        std::fs::remove_file(run.join("bus")).unwrap();
        std::fs::create_dir_all(run.join("systemd")).unwrap();
        std::fs::write(run.join("systemd/private"), "").unwrap();
        assert!(ok(&booted, Some(run.as_os_str()), Some(&path)));
        // Each missing piece says no: not booted with systemd, no runtime dir, no systemd-run on the PATH.
        assert!(!ok(&root.join("nothing"), Some(run.as_os_str()), Some(&path)));
        assert!(!ok(&booted, None, Some(&path)));
        assert!(!ok(&booted, Some(run.as_os_str()), Some(other.as_os_str())));
        assert!(!ok(&booted, Some(run.as_os_str()), None));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_probe_that_was_not_wanted_says_no_without_starting_anything() {
        assert!(scope_probe(false).0.is_none());
        assert!(!scope_probe(false).works());
    }

    #[test]
    fn a_probe_counts_only_a_clean_exit_and_never_waits_past_its_limit() {
        let run = |prog: &str, arg: &str| Command::new(prog).arg(arg).stdin(Stdio::null()).spawn().unwrap();
        assert!(ended_well(run("true", ""), Duration::from_secs(5)));
        assert!(!ended_well(run("false", ""), Duration::from_secs(5)));
        let start = Instant::now();
        assert!(!ended_well(run("sleep", "30"), Duration::from_millis(50)), "a hang is a no");
        assert!(start.elapsed() < Duration::from_secs(5), "and it is killed, not waited for");
    }
}
