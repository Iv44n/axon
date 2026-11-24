//! What the prompt chips say: the time, the user, the directory the shell is in, the git branch
//! and its changes, the Python and Kubernetes environments. Reading all of it touches the disk and
//! may run `git`, so `gather` is meant for a short-lived thread, never the UI thread; the pieces
//! that decide what the text is are pure functions with their own tests.
//!
//! Where each piece comes from:
//! - directory: `/proc/<shell>/cwd`, right without any help from the shell;
//! - branch: `.git/HEAD`, read up the directory tree (worktrees and submodules included);
//! - changes: `git diff HEAD --numstat`, run at most every few seconds and when the directory changes;
//! - Python and Kubernetes: files (`.python-version`, `~/.pyenv/version`, the kubeconfig's
//!   `current-context`), and the shell's own variables (`CONDA_DEFAULT_ENV`, `VIRTUAL_ENV`,
//!   `PYENV_VERSION`, `KUBECONFIG`), which only the shell knows: `axon shell-init` makes it write them
//!   to the file named by `$AXON_CONTEXT` at every prompt.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::config::Chip;

/// The longest a chip's text gets, in characters; anything beyond is cut with `…`.
const MAX_CHARS: usize = 40;
/// `git diff` runs when the directory changes and at most this often otherwise.
pub const GIT_EVERY: Duration = Duration::from_secs(3);

/// The text of every chip, indexed by `Chip as usize`. Empty means there is nothing to show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    pub text: [String; Chip::COUNT],
    /// The shell's directory as the kernel reports it, to tell when it changed.
    pub cwd: String,
    /// Whether `git diff` ran for this value (else `Changes` was carried over from the previous one).
    pub git_ran: bool,
}

impl Context {
    pub fn get(&self, c: Chip) -> &str {
        &self.text[c as usize]
    }

    /// This context with an example for every chip that has nothing to say: what the settings screen's
    /// preview shows, so a chip can be judged before the machine has anything for it.
    pub fn with_examples(&self) -> Context {
        let mut c = self.clone();
        for (chip, t) in [
            (Chip::Time, "03:48 pm"),
            (Chip::User, "you"),
            (Chip::Env, "condaenv"),
            (Chip::Pyenv, "3.12.1"),
            (Chip::Host, "you@axon"),
            (Chip::Dir, "~/lab/axon"),
            (Chip::Branch, "main"),
            (Chip::Changes, "3 \u{2022} +10 -2"),
            (Chip::Kube, "kind-dev"),
        ] {
            if c.text[chip as usize].is_empty() {
                c.text[chip as usize] = t.to_string();
            }
        }
        c
    }
}

/// What a refresh needs to know, owned so it can move to a thread.
pub struct Probe {
    /// The shell's process id.
    pub pid: u32,
    /// The file `axon shell-init` makes the shell write, if the shell was started with one.
    pub report: Option<PathBuf>,
    pub clock24: bool,
    /// The previous result: the directory it was for, the changes text and its age.
    pub prev: Context,
    pub git_age: Duration,
}

/// Reads everything. Slow: a thread, not the UI.
pub fn gather(p: &Probe) -> Context {
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let cwd = std::fs::read_link(format!("/proc/{}/cwd", p.pid)).unwrap_or_default();
    let report = p.report.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).map(|t| parse_report(&t)).unwrap_or_default();
    let mut c = Context { cwd: cwd.to_string_lossy().into_owned(), ..Context::default() };
    let set = |c: &mut Context, chip: Chip, v: String| c.text[chip as usize] = clean(&v);

    let (h, m) = local_hm();
    set(&mut c, Chip::Time, clock_text(h, m, p.clock24));
    let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default();
    set(&mut c, Chip::User, user.clone());
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    if !user.is_empty() && !host.trim().is_empty() {
        set(&mut c, Chip::Host, format!("{user}@{}", host.trim()));
    }
    if !cwd.as_os_str().is_empty() {
        set(&mut c, Chip::Dir, tidy_dir(&cwd.to_string_lossy(), home.as_deref().and_then(Path::to_str)));
    }
    let branch = (!cwd.as_os_str().is_empty()).then(|| git_branch(&cwd)).flatten();
    if let Some(b) = &branch {
        set(&mut c, Chip::Branch, b.clone());
    }
    // `git diff` only inside a repository, when the directory changed or the last run is old.
    if branch.is_some() {
        if c.cwd != p.prev.cwd || p.git_age >= GIT_EVERY {
            c.git_ran = true;
            if let Some(out) = git_diff_numstat(&cwd) {
                c.text[Chip::Changes as usize] = changes_text(parse_numstat(&out));
            }
        } else {
            c.text[Chip::Changes as usize] = p.prev.get(Chip::Changes).to_string();
        }
    }
    let env = if report.conda.is_empty() { report.venv.as_str() } else { report.conda.as_str() };
    set(&mut c, Chip::Env, env.to_string());
    let pyenv = if report.pyenv.is_empty() { pyenv_version(&cwd, home.as_deref()) } else { Some(report.pyenv.clone()) };
    if let Some(v) = pyenv.filter(|v| v != "system") {
        set(&mut c, Chip::Pyenv, v);
    }
    let kubeconfig = if report.kubeconfig.is_empty() { None } else { Some(PathBuf::from(&report.kubeconfig)) };
    let kubeconfig = kubeconfig.or_else(|| std::env::var_os("KUBECONFIG").and_then(|v| std::env::split_paths(&v).next())).or_else(|| home.map(|h| h.join(".kube/config")));
    if let Some(ctx) = kubeconfig.and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| kube_context(&t)) {
        set(&mut c, Chip::Kube, ctx);
    }
    c
}

/// The file a window's shell reports to: `$XDG_RUNTIME_DIR/axon-<pid>.ctx`, `pid` being axon's own.
pub fn report_path(pid: u32) -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty())?;
    Some(PathBuf::from(dir).join(format!("axon-{pid}.ctx")))
}

/// Removes the report files of windows that are gone (killed, so their `Drop` never ran).
pub fn remove_stale_reports(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        let pid = name.to_str().and_then(|n| n.strip_prefix("axon-")).and_then(|n| n.strip_suffix(".ctx").or_else(|| n.strip_suffix(".chips"))).and_then(|n| n.parse::<u32>().ok());
        if pid.is_some_and(|p| !Path::new(&format!("/proc/{p}")).exists()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

// ---------------------------------------------------------- shell hooks --
//
// axon cannot see where the shell's prompt is, so the shell marks it: when the flag file `$AXON_CHIPS`
// exists, the prompt starts with a line of its own holding one concealed space (SGR 8) and ends with a
// concealed space right where the user types (`view::prompt_row` finds them), and at every prompt the
// shell writes what only it knows to `$AXON_CONTEXT`. axon creates the flag while the chips are on and
// removes it when they are off, so the setting takes effect at the next prompt, with no restart. The
// hook puts the shell's own prompt back when the flag is gone. Concealed spaces are invisible, count as
// one column each and are copied as spaces. While the chips are on they are the prompt's context, so the
// shell's own prompt text (`user@host:~/dir`) is replaced by a bare `$`, in bold amber (the accent colour): showing both would say everything twice.

/// The bash hook as statements. For the shells axon starts it is the whole `PROMPT_COMMAND`
/// (`shell_env`); `axon shell-init bash` wraps it in a function. It keeps `$?`: see `BASH_KEEP_STATUS`.
pub const BASH_HOOK: &str = r##"__axon_s=$?
__axon_n=$((${__axon_n:-0}+1))
printf '\e]2;axon!hook E %s %s\a' "$__axon_n" "$__axon_s"
case $PS0 in *'axon!hook S'*) ;; *) PS0=$PS0$'\e]2;axon!hook S\a' ;; esac
__axon_pfx=$'\\[\e[8;38;2;'
__axon_pre=$__axon_pfx$((__axon_n>>16&255))';'$((__axon_n>>8&255))';'$((__axon_n&255))$'m\\] \\[\e[0m\\]\n'
__axon_post=$'\\[\e[8m\\] \\[\e[0m\\]'
if [ -e "$AXON_CHIPS" ]; then
  # The chips say what the prompt used to: it becomes the reserved line (which carries the prompt's
  # number in the colour of its concealed cell), the window title (kept, as Debian's own prompt sets
  # it) and a bold amber $ or #. The shell's own prompt is kept to be put back.
  case $PS1 in "$__axon_pfx"*) ;; *) __axon_ps1=$PS1 ;; esac
  PS1=$__axon_pre$'\\[\e]0;\\u@\\h: \\w\a\\]\\[\e[1;38;2;255;180;84m\\]\\$\\[\e[0m\\]'$__axon_post
else
  case $PS1 in "$__axon_pfx"*) PS1=$__axon_ps1 ;; esac
fi
[ -n "$AXON_CONTEXT" ] && printf 'conda=%s\nvenv=%s\npyenv=%s\nkubeconfig=%s\n' "$CONDA_DEFAULT_ENV" "${VIRTUAL_ENV##*/}" "$PYENV_VERSION" "${KUBECONFIG%%:*}" > "$AXON_CONTEXT""##;

/// Makes the status of the command that just ran (`$?`) what it was, after the hook.
const BASH_KEEP_STATUS: &str = "(exit $__axon_s)";

const ZSH_HOOK: &str = r##"__axon_prompt() {
  local s=$? pfx=$'%{\e[8;38;2;'
  __axon_n=$(( ${__axon_n:-0} + 1 ))
  printf '\e]2;axon!hook E %s %s\a' $__axon_n $s
  local pre=$pfx$((__axon_n>>16&255))';'$((__axon_n>>8&255))';'$((__axon_n&255))$'m%} %{\e[0m%}\n' post=$'%{\e[8m%} %{\e[0m%}'
  if [[ -e $AXON_CHIPS ]]; then
    [[ $PS1 == "$pfx"* ]] || __axon_ps1=$PS1
    PS1=$pre$'%{\e]0;%n@%m: %~\a%}%{\e[1;38;2;255;180;84m%}%#%{\e[0m%}'$post
  elif [[ $PS1 == "$pfx"* ]]; then
    PS1=$__axon_ps1
  fi
  [[ -n $AXON_CONTEXT ]] && printf 'conda=%s\nvenv=%s\npyenv=%s\nkubeconfig=%s\n' "$CONDA_DEFAULT_ENV" "${VIRTUAL_ENV:t}" "$PYENV_VERSION" "${KUBECONFIG%%:*}" > $AXON_CONTEXT
  return $s
}
__axon_start() { printf '\e]2;axon!hook S\a' }
(( ${precmd_functions[(Ie)__axon_prompt]} )) || precmd_functions+=(__axon_prompt)
(( ${preexec_functions[(Ie)__axon_start]} )) || preexec_functions+=(__axon_start)
"##;

const FISH_HOOK: &str = r##"if not functions -q __axon_fish_prompt
    functions -c fish_prompt __axon_fish_prompt
    function fish_prompt
        if test -e "$AXON_CHIPS"
            printf '\e[8;38;2;%d;%d;%dm \e[0m\n' (math "floor($__axon_n / 65536) % 256") (math "floor($__axon_n / 256) % 256") (math "$__axon_n % 256")
            printf '\e[1;38;2;255;180;84m'
            if fish_is_root_user; printf '#'; else; printf '$'; end
            printf '\e[0m'
            printf '\e[8m \e[0m'
        else
            __axon_fish_prompt
        end
    end
end
function __axon_start --on-event fish_preexec
    printf '\e]2;axon!hook S\a'
end
function __axon_report --on-event fish_prompt
    set -l s $status
    set -q __axon_n; or set -g __axon_n 0
    set -g __axon_n (math $__axon_n + 1)
    printf '\e]2;axon!hook E %s %s\a' $__axon_n $s
    test -n "$AXON_CONTEXT"; or return
    printf 'conda=%s\nvenv=%s\npyenv=%s\nkubeconfig=%s\n' "$CONDA_DEFAULT_ENV" (string replace -r '.*/' '' -- "$VIRTUAL_ENV") "$PYENV_VERSION" (string split -f1 : -- "$KUBECONFIG") > $AXON_CONTEXT
end
"##;

/// What `axon shell-init SHELL` prints, for a shell's rc file.
pub fn shell_init_script(shell: &str) -> Option<String> {
    Some(match shell {
        "bash" => format!(
            "__axon_prompt() {{\n{BASH_HOOK}\n  return $__axon_s\n}}\n\
             case \";$PROMPT_COMMAND;\" in *\";__axon_prompt;\"*) ;; *) PROMPT_COMMAND=\"__axon_prompt${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\" ;; esac\n"
        ),
        "zsh" => ZSH_HOOK.to_string(),
        "fish" => FISH_HOOK.to_string(),
        _ => return None,
    })
}

/// The flag file next to the report: its existence is the chips being on.
pub fn flag_path(pid: u32) -> Option<PathBuf> {
    report_path(pid).map(|p| p.with_extension("chips"))
}

/// The variables for the shell axon starts: where to report to, the flag, and, for bash, the hook itself
/// as `PROMPT_COMMAND` (added before one the environment already has), so it works with no rc-file line.
/// Other shells need `axon shell-init`. A shell whose rc file replaces `PROMPT_COMMAND` loses the hook;
/// the rc-file line brings it back.
pub fn shell_env(shell: &str, report: Option<&Path>, flag: Option<&Path>, inherited: Option<&str>) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(r) = report {
        env.push(("AXON_CONTEXT".to_string(), r.to_string_lossy().into_owned()));
    }
    if let Some(f) = flag {
        env.push(("AXON_CHIPS".to_string(), f.to_string_lossy().into_owned()));
    }
    let name = shell.rsplit('/').next().unwrap_or(shell);
    if name == "bash" && report.is_some() && flag.is_some() {
        let rest = inherited.filter(|v| !v.trim().is_empty()).map(|v| format!("\n{v}")).unwrap_or_default();
        env.push(("PROMPT_COMMAND".to_string(), format!("{BASH_HOOK}\n{BASH_KEEP_STATUS}{rest}")));
    }
    env
}

// ------------------------------------------------------------------ pure --

/// `03:48 pm`, or `15:48` on a 24-hour clock.
pub fn clock_text(hour: u32, min: u32, clock24: bool) -> String {
    if clock24 {
        return format!("{hour:02}:{min:02}");
    }
    let h12 = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{h12:02}:{min:02} {}", if hour < 12 { "am" } else { "pm" })
}

/// Control characters out, the length capped: a directory or a branch can be named anything.
pub fn clean(s: &str) -> String {
    let mut out: String = s.trim().chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_CHARS + 1).collect();
    if out.chars().count() > MAX_CHARS {
        out = out.chars().take(MAX_CHARS - 1).collect();
        out.push('…');
    }
    out
}

/// `/home/me/code/axon/src` -> `~/code/axon/src`; more than three components keep the last three:
/// `…/axon/src/bin`. `home` is the home directory, when known.
pub fn tidy_dir(path: &str, home: Option<&str>) -> String {
    let (prefix, rest) = match home.filter(|h| !h.is_empty() && *h != "/") {
        Some(h) if path == h => return "~".into(),
        Some(h) if path.strip_prefix(h).is_some_and(|r| r.starts_with('/')) => ("~", &path[h.len()..]),
        _ => ("", path),
    };
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    match (prefix, parts.len()) {
        (_, 0) => if prefix.is_empty() { "/".into() } else { prefix.into() },
        (_, n) if n > 3 => format!("…/{}", parts[n - 3..].join("/")),
        ("~", _) => format!("~/{}", parts.join("/")),
        _ => format!("/{}", parts.join("/")),
    }
}

/// The branch of the repository `dir` is in, or a short hash when HEAD is detached.
pub fn git_branch(dir: &Path) -> Option<String> {
    let mut at = Some(dir);
    while let Some(d) = at {
        let dot = d.join(".git");
        if dot.is_dir() {
            return head_name(&dot.join("HEAD"));
        }
        if dot.is_file() {
            // A worktree or a submodule: `gitdir: <path>`.
            let text = std::fs::read_to_string(&dot).ok()?;
            let target = text.trim().strip_prefix("gitdir:")?.trim();
            let gitdir = if Path::new(target).is_absolute() { PathBuf::from(target) } else { d.join(target) };
            return head_name(&gitdir.join("HEAD"));
        }
        at = d.parent();
    }
    None
}

fn head_name(head: &Path) -> Option<String> {
    let text = std::fs::read_to_string(head).ok()?;
    let text = text.trim();
    match text.strip_prefix("ref:") {
        Some(r) => {
            let r = r.trim();
            Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string())
        }
        None => Some(text.chars().take(7).collect()),
    }
}

/// Files, added lines and removed lines from `git diff --numstat`; a binary file counts as a file only.
pub fn parse_numstat(out: &str) -> (usize, usize, usize) {
    let (mut files, mut add, mut del) = (0, 0, 0);
    for line in out.lines() {
        let mut f = line.split('\t');
        let (Some(a), Some(d), Some(_)) = (f.next(), f.next(), f.next()) else { continue };
        files += 1;
        add += a.parse::<usize>().unwrap_or(0);
        del += d.parse::<usize>().unwrap_or(0);
    }
    (files, add, del)
}

/// `3 • +10 -2`; nothing when nothing changed.
pub fn changes_text((files, add, del): (usize, usize, usize)) -> String {
    if files == 0 { String::new() } else { format!("{files} • +{add} -{del}") }
}

/// `git diff HEAD --numstat` in `dir`. The repository may be somebody else's (a clone just entered),
/// and git runs whatever its own config names: `core.fsmonitor` is a command run by `git diff`, and
/// `diff.external` and textconv filters run programs on the files, and a `clean` filter runs when the
/// repository's own `.gitattributes` names one. All of these are switched off here (attributes are read
/// from the empty tree, not the work tree), so merely having `cd`ed into a repository never executes
/// anything it brought along. Left open: attributes in `.git/info`, which only whoever owns `.git` can write.
fn git_diff_numstat(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null"])
        .args(["--attr-source=4b825dc642cb6eb9a060e54bf8d69288fbee4904", "diff", "HEAD", "--numstat", "--no-ext-diff", "--no-textconv"])
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What `axon shell-init` makes the shell write at every prompt.
#[derive(Default, Debug, PartialEq)]
pub struct Report {
    pub conda: String,
    pub venv: String,
    pub pyenv: String,
    pub kubeconfig: String,
}

pub fn parse_report(text: &str) -> Report {
    let mut r = Report::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let slot = match k {
            "conda" => &mut r.conda,
            "venv" => &mut r.venv,
            "pyenv" => &mut r.pyenv,
            "kubeconfig" => &mut r.kubeconfig,
            _ => continue,
        };
        *slot = v.trim().to_string();
    }
    r
}

/// The Python version pyenv would pick in `dir`: the nearest `.python-version` up the tree, else
/// `~/.pyenv/version` when pyenv is installed. Only the first word (a file may list several).
pub fn pyenv_version(dir: &Path, home: Option<&Path>) -> Option<String> {
    let first = |t: String| t.split_whitespace().next().map(str::to_string);
    let mut at = Some(dir).filter(|d| !d.as_os_str().is_empty());
    while let Some(d) = at {
        if let Some(v) = std::fs::read_to_string(d.join(".python-version")).ok().and_then(first) {
            return Some(v);
        }
        at = d.parent();
    }
    let root = home?.join(".pyenv");
    root.is_dir().then(|| std::fs::read_to_string(root.join("version")).ok().and_then(first)).flatten()
}

/// The `current-context:` of a kubeconfig.
pub fn kube_context(text: &str) -> Option<String> {
    let line = text.lines().find_map(|l| l.strip_prefix("current-context:"))?;
    let v = line.trim().trim_matches(|c| c == '"' || c == '\'');
    (!v.is_empty()).then(|| v.to_string())
}

// ------------------------------------------------------------------ time --

#[repr(C)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    gmtoff: i64,
    zone: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
}

fn epoch() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The local hour and minute (the C library knows the time zone; `std` does not).
fn local_hm() -> (u32, u32) {
    let t = epoch();
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: `tm` has the layout of glibc's and musl's `struct tm` on 64-bit Linux, and both pointers are valid.
    let ok = unsafe { !localtime_r(&t, &mut tm).is_null() };
    if ok { (tm.hour.clamp(0, 23) as u32, tm.min.clamp(0, 59) as u32) } else { ((t / 3600 % 24) as u32, (t / 60 % 60) as u32) }
}

/// How long until the minute changes: when the time chip has to be repainted.
pub fn until_next_minute() -> Duration {
    Duration::from_secs(60 - epoch().rem_euclid(60) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_has_two_formats() {
        assert_eq!(clock_text(15, 48, false), "03:48 pm");
        assert_eq!(clock_text(15, 48, true), "15:48");
        assert_eq!(clock_text(0, 5, false), "12:05 am");
        assert_eq!(clock_text(12, 0, false), "12:00 pm");
        assert_eq!(clock_text(0, 5, true), "00:05");
    }

    #[test]
    fn the_local_time_is_a_real_time() {
        let (h, m) = local_hm();
        assert!(h < 24 && m < 60);
        assert!((1..=60).contains(&until_next_minute().as_secs()));
    }

    #[test]
    fn paths_lose_the_home_and_keep_the_last_three_parts() {
        let home = Some("/home/ana");
        assert_eq!(tidy_dir("/home/ana", home), "~");
        assert_eq!(tidy_dir("/home/ana/Desktop", home), "~/Desktop");
        assert_eq!(tidy_dir("/home/ana/code/axon/src/bin", home), "…/axon/src/bin");
        assert_eq!(tidy_dir("/home/anabel/x", home), "/home/anabel/x", "a longer name is not the home");
        assert_eq!(tidy_dir("/", home), "/");
        assert_eq!(tidy_dir("/usr/share", home), "/usr/share");
        assert_eq!(tidy_dir("/a/b/c/d", None), "…/b/c/d");
        assert_eq!(tidy_dir("/home/ana/x", Some("/")), "/home/ana/x");
    }

    #[test]
    fn text_is_cleaned_and_capped() {
        assert_eq!(clean("  main\n"), "main");
        assert_eq!(clean("a\x1b[31mb"), "a [31mb");
        let long = "x".repeat(100);
        let c = clean(&long);
        assert_eq!(c.chars().count(), MAX_CHARS);
        assert!(c.ends_with('…'));
        assert_eq!(clean(&"y".repeat(MAX_CHARS)).chars().count(), MAX_CHARS);
    }

    #[test]
    fn numstat_adds_files_and_lines() {
        assert_eq!(parse_numstat("10\t2\tsrc/a.rs\n0\t0\tb\n-\t-\timg.png\nnonsense\n"), (3, 10, 2));
        assert_eq!(parse_numstat(""), (0, 0, 0));
        assert_eq!(changes_text((3, 10, 2)), "3 • +10 -2");
        assert_eq!(changes_text((0, 0, 0)), "");
    }

    #[test]
    fn the_shell_report_is_read_by_key() {
        let r = parse_report("conda=base\nvenv=\npyenv=3.12.1\nkubeconfig=/k/cfg\nother=1\ngarbage\n");
        assert_eq!(r, Report { conda: "base".into(), venv: String::new(), pyenv: "3.12.1".into(), kubeconfig: "/k/cfg".into() });
        assert_eq!(parse_report(""), Report::default());
    }

    #[test]
    fn the_kube_context_is_the_current_one() {
        assert_eq!(kube_context("apiVersion: v1\ncurrent-context: prod-eu\nkind: Config\n").as_deref(), Some("prod-eu"));
        assert_eq!(kube_context("current-context: \"gke_a_b\"\n").as_deref(), Some("gke_a_b"));
        assert_eq!(kube_context("current-context: \"\"\n"), None);
        assert_eq!(kube_context("clusters: []\n"), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axon-ctx-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_branch_is_read_up_the_tree_for_plain_repos_worktrees_and_detached_heads() {
        let d = scratch("git");
        std::fs::create_dir_all(d.join("repo/.git")).unwrap();
        std::fs::create_dir_all(d.join("repo/src/deep")).unwrap();
        std::fs::write(d.join("repo/.git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(git_branch(&d.join("repo/src/deep")).as_deref(), Some("feature/x"));

        std::fs::write(d.join("repo/.git/HEAD"), "0123456789abcdef0123456789abcdef01234567\n").unwrap();
        assert_eq!(git_branch(&d.join("repo")).as_deref(), Some("0123456"));

        // A worktree: `.git` is a file pointing at the real directory.
        std::fs::create_dir_all(d.join("wt")).unwrap();
        std::fs::create_dir_all(d.join("real")).unwrap();
        std::fs::write(d.join("real/HEAD"), "ref: refs/heads/wt-branch\n").unwrap();
        std::fs::write(d.join("wt/.git"), format!("gitdir: {}\n", d.join("real").display())).unwrap();
        assert_eq!(git_branch(&d.join("wt")).as_deref(), Some("wt-branch"));

        std::fs::create_dir_all(d.join("plain")).unwrap();
        assert_eq!(git_branch(&d.join("plain")), None, "only valid while no repository sits above the scratch dir");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pyenv_reads_the_nearest_python_version_file_then_the_home_default() {
        let d = scratch("py");
        std::fs::create_dir_all(d.join("proj/sub")).unwrap();
        std::fs::write(d.join("proj/.python-version"), "3.11.4\n3.10\n").unwrap();
        assert_eq!(pyenv_version(&d.join("proj/sub"), Some(&d)).as_deref(), Some("3.11.4"));
        // Nothing in the tree: no pyenv, no chip; with ~/.pyenv/version, that.
        std::fs::create_dir_all(d.join("other")).unwrap();
        assert_eq!(pyenv_version(&d.join("other"), Some(&d)), None);
        std::fs::create_dir_all(d.join(".pyenv")).unwrap();
        std::fs::write(d.join(".pyenv/version"), "3.12.1\n").unwrap();
        assert_eq!(pyenv_version(&d.join("other"), Some(&d)).as_deref(), Some("3.12.1"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn reports_of_dead_windows_are_removed_and_live_ones_kept() {
        let d = scratch("stale");
        let me = std::process::id();
        for name in [format!("axon-{me}.ctx"), "axon-4000000000.ctx".to_string(), "axon-x.ctx".to_string(), "other.ctx".to_string()] {
            std::fs::write(d.join(name), "x").unwrap();
        }
        remove_stale_reports(&d);
        let mut left: Vec<String> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, [format!("axon-{me}.ctx"), "axon-x.ctx".to_string(), "other.ctx".to_string()]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_repository_cannot_run_its_own_commands_through_the_changes_chip() {
        let d = scratch("evil");
        let git = |args: &[&str]| {
            Command::new("git").args(args).current_dir(&d).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
        };
        if !git(&["init", "-q"]) {
            return; // no git here: nothing to test
        }
        let marker = d.join("ran");
        let script = d.join("monitor.sh");
        std::fs::write(&script, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::fs::write(d.join("f"), "a\n").unwrap();
        let ok = git(&["add", "f"]) && git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "x"]);
        assert!(ok);
        // Config the repository carries: an fsmonitor command and an external diff.
        assert!(git(&["config", "core.fsmonitor", &script.to_string_lossy()]));
        assert!(git(&["config", "diff.external", &script.to_string_lossy()]));
        // A clean filter named by the repository's own .gitattributes.
        assert!(git(&["config", "filter.x.clean", &format!("{} #", script.display())]));
        std::fs::write(d.join(".gitattributes"), "f filter=x\n").unwrap();
        std::fs::write(d.join("f"), "b\n").unwrap();
        let out = git_diff_numstat(&d).expect("git diff ran");
        assert_eq!(parse_numstat(&out), (1, 1, 1));
        assert!(!marker.exists(), "the repository's own command was executed");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn gather_fills_what_it_can_for_a_process_that_exists() {
        let p = Probe { pid: std::process::id(), report: None, clock24: true, prev: Context::default(), git_age: Duration::ZERO };
        let c = gather(&p);
        assert_eq!(c.get(Chip::Time).len(), 5);
        assert!(!c.get(Chip::Dir).is_empty());
        assert!(!c.cwd.is_empty());
        // A process that does not exist has no directory: no directory, branch or changes chips.
        let gone = gather(&Probe { pid: u32::MAX, ..p });
        assert!(gone.get(Chip::Dir).is_empty() && gone.get(Chip::Branch).is_empty());
    }

    /// `cargo test --release -- --ignored print_the_context --nocapture`: what the chips would say here.
    #[test]
    #[ignore = "prints, for a look by hand"]
    fn print_the_context() {
        let p = Probe { pid: std::process::id(), report: None, clock24: false, prev: Context::default(), git_age: Duration::ZERO };
        let c = gather(&p);
        for chip in Chip::ALL {
            println!("{:8} {:?}", chip.name(), c.get(chip));
        }
    }

    /// `cargo test --release -- --ignored dump_the_bash_environment`: writes the `PROMPT_COMMAND` axon gives
    /// bash to /tmp/axon-prompt-command, for trying it in a real shell.
    #[test]
    #[ignore = "writes a file for a manual check"]
    fn dump_the_bash_environment() {
        let env = shell_env("/bin/bash", Some(Path::new("/tmp/axon-test.ctx")), Some(Path::new("/tmp/axon-test.chips")), None);
        let pc = env.iter().find(|(k, _)| k == "PROMPT_COMMAND").unwrap();
        std::fs::write("/tmp/axon-prompt-command", &pc.1).unwrap();
    }
}
