//! What the prompt chips say: the time, the user, the directory the shell is in, the git branch
//! and its changes, the Python and Kubernetes environments. Reading all of it touches the disk,
//! so `gather` is meant for a short-lived thread, never the UI thread; the pieces that decide what
//! the text is are pure functions with their own tests. axon itself never runs `git`.
//!
//! Where each piece comes from:
//! - directory: `/proc/<shell>/cwd`, right without any help from the shell;
//! - branch: `.git/HEAD`, read up the directory tree (worktrees and submodules included);
//! - changes: the shell's hook runs `git diff HEAD --numstat` at each prompt, in the user's own git
//!   environment (so `safe.directory` and the like apply), with the repository's own hooks and filters
//!   off, and reports `files added removed`;
//! - Python and Kubernetes: files (`.python-version`, `~/.pyenv/version`, the kubeconfig's
//!   `current-context`), and the shell's own variables (`CONDA_DEFAULT_ENV`, `VIRTUAL_ENV`,
//!   `PYENV_VERSION`, `KUBECONFIG`), which only the shell knows: `axon shell-init` makes it write them
//!   to the file named by `$AXON_CONTEXT` at every prompt.
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::config::Chip;

/// The longest a chip's text gets, in characters; anything beyond is cut with `…`.
const MAX_CHARS: usize = 40;
/// The text of every chip, indexed by `Chip as usize`. Empty means there is nothing to show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    pub text: [String; Chip::COUNT],
    /// The shell's directory as the kernel reports it, to tell when it changed.
    pub cwd: String,
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
    // The shell ran `git diff` itself (see `CHANGES`); it only reports from inside a repository.
    if branch.is_some() {
        c.text[Chip::Changes as usize] = changes_text(parse_changes(&report.changes));
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

/// What the bash and zsh hooks run to fill `__axon_c` with `files added removed`: `git diff HEAD --numstat`
/// in the shell's own directory, run by the shell (so with the user's own git configuration: a
/// `safe.directory` in it applies, which axon itself, never running git, could not honour). The repository
/// may be somebody else's (a clone just entered) and git runs whatever its own config names: `core.fsmonitor`
/// is a command `git diff` executes, `diff.external`, textconv and `clean` filters run programs on files. All of
/// those are switched off (attributes are read from the empty tree, not the work tree; `core.attributesFile`
/// is empty) and a repository with a `.git/info/attributes`, which cannot be switched off, gets no chip. Only
/// when the chips are on, and `timeout` keeps a huge repository from holding the prompt: no chip then.
macro_rules! changes_sh {
    () => {
        r##"__axon_c=
  if [ -e "$AXON_CHIPS" ]; then
    __axon_a=$(timeout 0.5 git -c core.fsmonitor=false rev-parse --git-path info/attributes 2>/dev/null </dev/null)
    if [ -n "$__axon_a" ] && [ ! -e "$__axon_a" ]; then
      __axon_c=$(GIT_OPTIONAL_LOCKS=0 timeout 0.5 git -c core.fsmonitor=false -c core.hooksPath=/dev/null -c core.attributesFile=/dev/null --attr-source=4b825dc642cb6eb9a060e54bf8d69288fbee4904 diff HEAD --numstat --no-ext-diff --no-textconv 2>/dev/null </dev/null | awk 'NF>=3{f++;a+=$1;d+=$2}END{if(f)printf "%d %d %d",f,a+0,d+0}')
    fi
  fi"##
    };
}

/// The bash hook as statements. For the shells axon starts it is the whole `PROMPT_COMMAND`
/// (`shell_env`); `axon shell-init bash` wraps it in a function. It keeps `$?`: see `BASH_KEEP_STATUS`.
pub const BASH_HOOK: &str = concat!(
    r##"__axon_s=$?
__axon_n=$((${__axon_n:-0}+1))
if [ -n "$AXON_CONTEXT" ]; then
  "##,
    changes_sh!(),
    r##"
  printf 'conda=%s\nvenv=%s\npyenv=%s\nkubeconfig=%s\nchanges=%s\n' "$CONDA_DEFAULT_ENV" "${VIRTUAL_ENV##*/}" "$PYENV_VERSION" "${KUBECONFIG%%:*}" "$__axon_c" > "$AXON_CONTEXT"
fi
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
fi"##
);

/// Makes the status of the command that just ran (`$?`) what it was, after the hook.
const BASH_KEEP_STATUS: &str = "(exit $__axon_s)";

const ZSH_HOOK: &str = concat!(
    r##"__axon_prompt() {
  local s=$? pfx=$'%{\e[8;38;2;'
  __axon_n=$(( ${__axon_n:-0} + 1 ))
  if [[ -n $AXON_CONTEXT ]]; then
  "##,
    changes_sh!(),
    r##"
    printf 'conda=%s\nvenv=%s\npyenv=%s\nkubeconfig=%s\nchanges=%s\n' "$CONDA_DEFAULT_ENV" "${VIRTUAL_ENV:t}" "$PYENV_VERSION" "${KUBECONFIG%%:*}" "$__axon_c" > $AXON_CONTEXT
  fi
  printf '\e]2;axon!hook E %s %s\a' $__axon_n $s
  local pre=$pfx$((__axon_n>>16&255))';'$((__axon_n>>8&255))';'$((__axon_n&255))$'m%} %{\e[0m%}\n' post=$'%{\e[8m%} %{\e[0m%}'
  if [[ -e $AXON_CHIPS ]]; then
    [[ $PS1 == "$pfx"* ]] || __axon_ps1=$PS1
    PS1=$pre$'%{\e]0;%n@%m: %~\a%}%{\e[1;38;2;255;180;84m%}%#%{\e[0m%}'$post
  elif [[ $PS1 == "$pfx"* ]]; then
    PS1=$__axon_ps1
  fi
  return $s
}
__axon_start() { printf '\e]2;axon!hook S\a' }
(( ${precmd_functions[(Ie)__axon_prompt]} )) || precmd_functions+=(__axon_prompt)
(( ${preexec_functions[(Ie)__axon_start]} )) || preexec_functions+=(__axon_start)
"##
);

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
    if test -n "$AXON_CONTEXT"
        set -l c
        if test -e "$AXON_CHIPS"
            set -l a (timeout 0.5 git -c core.fsmonitor=false rev-parse --git-path info/attributes 2>/dev/null </dev/null)
            if test -n "$a"; and not test -e "$a"
                set c (env GIT_OPTIONAL_LOCKS=0 timeout 0.5 git -c core.fsmonitor=false -c core.hooksPath=/dev/null -c core.attributesFile=/dev/null --attr-source=4b825dc642cb6eb9a060e54bf8d69288fbee4904 diff HEAD --numstat --no-ext-diff --no-textconv 2>/dev/null </dev/null | awk 'NF>=3{f++;a+=$1;d+=$2}END{if(f)printf "%d %d %d",f,a+0,d+0}')
            end
        end
        printf 'conda=%s\nvenv=%s\npyenv=%s\nkubeconfig=%s\nchanges=%s\n' "$CONDA_DEFAULT_ENV" (string replace -r '.*/' '' -- "$VIRTUAL_ENV") "$PYENV_VERSION" (string split -f1 : -- "$KUBECONFIG") "$c" > $AXON_CONTEXT
    end
    printf '\e]2;axon!hook E %s %s\a' $__axon_n $s
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

/// Files, added lines and removed lines from the hook's `files added removed`; zeros when it is not that.
pub fn parse_changes(text: &str) -> (usize, usize, usize) {
    let mut n = text.split_whitespace().map(|w| w.parse::<usize>());
    match (n.next(), n.next(), n.next(), n.next()) {
        (Some(Ok(f)), Some(Ok(a)), Some(Ok(d)), None) => (f, a, d),
        _ => (0, 0, 0),
    }
}

/// `3 • +10 -2`; nothing when nothing changed.
pub fn changes_text((files, add, del): (usize, usize, usize)) -> String {
    if files == 0 { String::new() } else { format!("{files} • +{add} -{del}") }
}

/// What `axon shell-init` makes the shell write at every prompt.
#[derive(Default, Debug, PartialEq)]
pub struct Report {
    pub conda: String,
    pub venv: String,
    pub pyenv: String,
    pub kubeconfig: String,
    /// `files added removed` as the hook computed it; empty outside a repository or when git said nothing.
    pub changes: String,
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
            "changes" => &mut r.changes,
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
    fn the_hooks_changes_are_three_numbers_and_nothing_else_counts() {
        assert_eq!(parse_changes("3 10 2"), (3, 10, 2));
        assert_eq!(parse_changes(" 3 10 2\n"), (3, 10, 2));
        for bad in ["", "x", "1 2", "1 2 3 4", "1 -2 3", "a b c"] {
            assert_eq!(parse_changes(bad), (0, 0, 0), "{bad:?}");
        }
        assert_eq!(changes_text((3, 10, 2)), "3 • +10 -2");
        assert_eq!(changes_text((0, 0, 0)), "");
    }

    #[test]
    fn the_shell_report_is_read_by_key() {
        let r = parse_report("conda=base\nvenv=\npyenv=3.12.1\nkubeconfig=/k/cfg\nchanges=1 2 3\nother=1\ngarbage\n");
        assert_eq!(r, Report { conda: "base".into(), venv: String::new(), pyenv: "3.12.1".into(), kubeconfig: "/k/cfg".into(), changes: "1 2 3".into() });
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

    // ----------------------------------------------------- the shells' hooks --
    // The hooks are run for real, as a shell would at a prompt. zsh and fish are skipped when not installed;
    // bash must be there.

    use std::process::{Command, Stdio};

    const SHELLS: [&str; 3] = ["bash", "zsh", "fish"];

    fn git(dir: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// A repository with `f` committed as `a\nb\n` and modified to `a\nc\n`: one file, +1 -1.
    fn repo(name: &str) -> PathBuf {
        let d = scratch(name);
        assert!(git(&d, &["init", "-q"]), "git is needed for these tests");
        std::fs::write(d.join("f"), "a\nb\n").unwrap();
        assert!(git(&d, &["add", "f"]) && git(&d, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "x"]));
        std::fs::write(d.join("f"), "a\nc\n").unwrap();
        d
    }

    fn script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }

    /// Runs `shell`'s hook once in `dir`, as at a prompt, and returns the report it wrote; `None` when the
    /// shell is not installed. `global` is the user's git config, `env` extra variables (they may replace `PATH`).
    fn run_hook(shell: &str, tag: &str, dir: &Path, chips: bool, global: Option<&Path>, env: &[(&str, String)]) -> Option<Report> {
        let w = scratch(&format!("run-{tag}-{shell}"));
        let (report, flag, file) = (w.join("report"), w.join("chips"), w.join("hook"));
        let call = if shell == "fish" { "__axon_report" } else { "__axon_prompt" };
        std::fs::write(&file, format!("{}\n{call}\n", shell_init_script(shell).unwrap())).unwrap();
        if chips {
            std::fs::write(&flag, "").unwrap();
        }
        let mut c = Command::new(shell);
        c.arg(match shell {
            "bash" => "--norc",
            "zsh" => "-f",
            _ => "--no-config",
        });
        c.arg(&file).current_dir(dir).env_clear();
        c.env("PATH", std::env::var("PATH").unwrap_or_default()).env("HOME", &w).env("LC_ALL", "C");
        c.env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", global.unwrap_or(Path::new("/dev/null")));
        c.env("AXON_CONTEXT", &report).env("AXON_CHIPS", &flag);
        for (k, v) in env {
            c.env(k, v);
        }
        c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let ran = c.status().ok()?;
        assert!(ran.success(), "{shell}'s hook failed");
        let text = std::fs::read_to_string(&report).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&w);
        Some(parse_report(&text))
    }

    /// Calls `check` with the report of every installed shell's hook.
    fn for_each_shell(tag: &str, dir: &Path, chips: bool, global: Option<&Path>, env: &[(&str, String)], check: impl Fn(&str, &Report)) {
        for sh in SHELLS {
            match run_hook(sh, tag, dir, chips, global, env) {
                Some(r) => check(sh, &r),
                None => assert_ne!(sh, "bash", "bash is needed for these tests"),
            }
        }
    }

    #[test]
    fn every_hook_reports_the_changes_of_a_normal_repository() {
        let d = repo("normal");
        for_each_shell("normal", &d, true, None, &[("PYENV_VERSION", "3.12.1".to_string())], |sh, r| {
            assert_eq!(r.changes, "1 1 1", "{sh}");
            assert_eq!(r.pyenv, "3.12.1", "{sh}: the rest of the report is still written");
        });
        // A subdirectory is the same repository.
        std::fs::create_dir_all(d.join("sub")).unwrap();
        for_each_shell("sub", &d.join("sub"), true, None, &[], |sh, r| assert_eq!(r.changes, "1 1 1", "{sh}"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_shell_axon_starts_reports_through_prompt_command() {
        let d = repo("raw");
        let w = scratch("raw-run");
        let (report, flag) = (w.join("report"), w.join("chips"));
        std::fs::write(&flag, "").unwrap();
        let env = shell_env("bash", Some(&report), Some(&flag), None);
        let pc = &env.iter().find(|(k, _)| k == "PROMPT_COMMAND").unwrap().1;
        let ok = Command::new("bash")
            .args(["--norc", "-c", "eval \"$PROMPT_COMMAND\""])
            .current_dir(&d)
            .env("PROMPT_COMMAND", pc)
            .env("AXON_CONTEXT", &report)
            .env("AXON_CHIPS", &flag)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok);
        assert_eq!(parse_report(&std::fs::read_to_string(&report).unwrap()).changes, "1 1 1");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn a_repository_cannot_run_its_own_commands_through_the_hook() {
        let d = repo("evil");
        let (marker, bin) = (d.join("ran"), d.join("evil.sh"));
        script(&bin, &format!("touch {}\ncat", marker.display()));
        let b = bin.to_string_lossy().into_owned();
        // What a repository can carry: an fsmonitor command, an external diff, a clean filter named by its own
        // .gitattributes and by a core.attributesFile its config points at.
        assert!(git(&d, &["config", "core.fsmonitor", &b]));
        assert!(git(&d, &["config", "diff.external", &b]));
        assert!(git(&d, &["config", "filter.x.clean", &b]));
        std::fs::write(d.join(".gitattributes"), "f filter=x\n").unwrap();
        std::fs::write(d.join("attrs"), "f filter=x\n").unwrap();
        assert!(git(&d, &["config", "core.attributesFile", &d.join("attrs").to_string_lossy()]));
        // The test has teeth: plain `git diff` does run it.
        let _ = Command::new("git").args(["diff", "HEAD", "--numstat"]).current_dir(&d).env("GIT_CONFIG_GLOBAL", "/dev/null").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        assert!(marker.exists(), "plain git did not run the repository's command: the test proves nothing");
        std::fs::remove_file(&marker).unwrap();
        for_each_shell("evil", &d, true, None, &[], |sh, r| {
            assert!(!marker.exists(), "{sh}: the repository's own command was executed");
            assert_eq!(r.changes, "1 1 1", "{sh}");
        });
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_repository_with_info_attributes_gets_no_chip() {
        let d = repo("info");
        let (marker, bin) = (d.join("ran"), d.join("evil.sh"));
        script(&bin, &format!("touch {}\ncat", marker.display()));
        assert!(git(&d, &["config", "filter.x.clean", &bin.to_string_lossy()]));
        std::fs::write(d.join(".git/info/attributes"), "f filter=x\n").unwrap();
        for_each_shell("info", &d, true, None, &[], |sh, r| {
            assert!(!marker.exists(), "{sh}: info/attributes ran a filter");
            assert_eq!(r.changes, "", "{sh}");
        });
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_safe_directory_in_the_users_git_config_applies() {
        // A repository somebody else owns (git is told to see it that way): refused unless the user's own
        // config trusts it, and then the chip is there. A refusal costs only the chip.
        let d = repo("foreign");
        let global = d.join("gitconfig");
        std::fs::write(&global, format!("[safe]\n\tdirectory = {}\n", d.display())).unwrap();
        let env = [("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1".to_string()), ("PYENV_VERSION", "3.12.1".to_string())];
        for_each_shell("trusted", &d, true, Some(&global), &env, |sh, r| assert_eq!(r.changes, "1 1 1", "{sh}"));
        for_each_shell("untrusted", &d, true, None, &env, |sh, r| {
            assert_eq!(r.changes, "", "{sh}");
            assert_eq!(r.pyenv, "3.12.1", "{sh}: the rest of the report is still written");
        });
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nothing_is_run_without_chips_or_outside_a_repository() {
        let d = repo("quiet");
        let (marker, fake) = (d.join("git-ran"), d.join("bin"));
        std::fs::create_dir_all(&fake).unwrap();
        script(&fake.join("git"), &format!("touch {}", marker.display()));
        let path = format!("{}:{}", fake.display(), std::env::var("PATH").unwrap_or_default());
        for_each_shell("off", &d, false, None, &[("PATH", path)], |sh, r| {
            assert_eq!(r.changes, "", "{sh}");
            assert!(!marker.exists(), "{sh}: git ran with the chips off");
        });
        let plain = scratch("plain");
        for_each_shell("plain", &plain, true, None, &[], |sh, r| assert_eq!(r.changes, "", "{sh}"));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&plain);
    }

    #[test]
    fn a_git_that_hangs_does_not_hold_the_prompt() {
        let d = repo("slow");
        let fake = d.join("bin");
        std::fs::create_dir_all(&fake).unwrap();
        script(&fake.join("git"), "exec sleep 5");
        let path = format!("{}:{}", fake.display(), std::env::var("PATH").unwrap_or_default());
        let t = std::time::Instant::now();
        for_each_shell("slow", &d, true, None, &[("PATH", path)], |sh, r| assert_eq!(r.changes, "", "{sh}"));
        assert!(t.elapsed() < Duration::from_secs(4 * SHELLS.len() as u64), "the hook waited for git");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn gather_fills_what_it_can_for_a_process_that_exists() {
        let p = Probe { pid: std::process::id(), report: None, clock24: true };
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
        let p = Probe { pid: std::process::id(), report: None, clock24: false };
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
