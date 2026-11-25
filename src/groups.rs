//! Groups of windows, the pure part ([0029](../knowledge/decisions/workspace/0029-group-dialog-menus-rename-toasts.md)):
//! what a group is, how the form of the group dialog is checked, how names and colours are suggested, how
//! windows leave a group, how a close is undone, and what the dialog's templates open. Nothing here knows a
//! window, a shell or a screen: the app (`wlapp/groups_ui.rs`) hands in plain numbers and strings, so every
//! rule is tested without a terminal.
//!
//! A window's group is an index into the app's list of groups (`Win::group`), as the panel needs it; a group
//! also has a stable `id` that a menu or a dialog can hold on to while the indices shift under it.
use crate::panes;
use std::path::Path;

/// The most characters a group's name can have (`maxlength` of the reference's field), and a window's.
pub const NAME_MAX: usize = 24;
pub const RENAME_MAX: usize = 28;
/// The colours a group can have (`side::group_color`).
pub const COLORS: usize = 8;

/// A group of windows: a name, one of the eight colours, whether the panel shows its windows, and how they are laid
/// side by side in the grid area (ADR 0034: the windows of a group are the panes of one split view). `kind` is the
/// last choice (a split in columns or in rows); what it means for the windows there are now is `panes::fit_kind`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: u32,
    pub name: String,
    pub color: u8,
    pub collapsed: bool,
    pub kind: panes::Kind,
}

/// The first colour no group has, else the one the count points at (the reference's `nextColor`).
pub fn next_color(used: &[u8]) -> u8 {
    (0..COLORS as u8).find(|c| !used.contains(c)).unwrap_or((used.len() % COLORS) as u8)
}

/// Two names are the same group's when they differ only in case or in the spaces around them.
fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// A name for a new group: `base` (the folder of the first window) or `fallback` when that says nothing
/// (the home folder), then `name 2`, `name 3`... until no group has it.
pub fn suggest_name(base: &str, fallback: &str, taken: &[&str]) -> String {
    let root = if matches!(base, "" | "~" | "/") { fallback } else { base };
    let root: String = root.chars().take(NAME_MAX - 3).collect();
    let mut name = root.clone();
    let mut n = 2;
    while taken.iter().any(|t| same(t, &name)) {
        name = format!("{root} {n}");
        n += 1;
    }
    name
}

/// What is wrong with the group dialog's form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormError {
    NameEmpty,
    NameTaken,
    NothingChosen,
}

/// Checks the form: the name has letters in it and no other group has it (`taken`, without the group being
/// edited), and a new group has at least one window, chosen or to be created. Returns the name, trimmed.
pub fn check(name: &str, taken: &[&str], chosen: usize, spawn: usize, edit: bool) -> Result<String, FormError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FormError::NameEmpty);
    }
    if taken.iter().any(|t| same(t, name)) {
        return Err(FormError::NameTaken);
    }
    if !edit && chosen + spawn == 0 {
        return Err(FormError::NothingChosen);
    }
    Ok(name.to_string())
}

/// What a renamed window is called: what was typed, trimmed; nothing (the automatic name) when it is empty or
/// is the automatic name already.
pub fn rename_result(typed: &str, auto: &str) -> String {
    let v = typed.trim();
    if v.is_empty() || v == auto { String::new() } else { v.to_string() }
}

/// `template` with each `{}` replaced by the next of `args`: the interface's sentences with a name in them.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut args = args.iter();
    let mut rest = template;
    while let Some(at) = rest.find("{}") {
        out.push_str(&rest[..at]);
        out.push_str(args.next().copied().unwrap_or(""));
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

// -------------------------------------------------------------- membership --

/// Drops the groups no window is in and renumbers the others in `win_group` (the group of each window, in
/// the order of the windows). Returns the groups that went.
pub fn prune(groups: &mut Vec<Group>, win_group: &mut [Option<usize>]) -> Vec<Group> {
    let mut gone = Vec::new();
    let mut g = 0;
    while g < groups.len() {
        if win_group.contains(&Some(g)) {
            g += 1;
            continue;
        }
        gone.push(groups.remove(g));
        for w in win_group.iter_mut() {
            *w = w.map(|x| if x > g { x - 1 } else { x });
        }
    }
    gone
}

/// Group `g` is dissolved: its windows stay, in none. (`prune` then drops the empty group.)
pub fn dissolve(win_group: &mut [Option<usize>], g: usize) {
    for w in win_group.iter_mut().filter(|w| **w == Some(g)) {
        *w = None;
    }
}

/// Where the windows of group `g` are in the list: `(first, count)`. `of` is the group of each window in the order of the
/// list, which keeps a group's windows together (`App::rebuild_side`); `None` when no window is in it.
pub fn span(of: impl IntoIterator<Item = Option<usize>>, g: usize) -> Option<(usize, usize)> {
    let (mut first, mut n) = (None, 0);
    for (i, x) in of.into_iter().enumerate() {
        match (x == Some(g), first) {
            (true, None) => (first, n) = (Some(i), 1),
            (true, Some(_)) => n += 1,
            (false, Some(_)) => break,
            (false, None) => {}
        }
    }
    first.map(|f| (f, n))
}

/// The index of the group with `id`.
pub fn index_of(groups: &[Group], id: u32) -> Option<usize> {
    groups.iter().position(|g| g.id == id)
}

// ------------------------------------------------------------------- close --

/// Which window is on screen after the windows at `closed` (their indices before, ascending) left a list of
/// `n`, `cur` being the one on screen: the same one when it stays, else the one that took the first closed
/// one's place (the reference's `tabs[min(firstIdx, len - 1)]`).
pub fn after_close(cur: usize, closed: &[usize], n: usize) -> usize {
    let left = n.saturating_sub(closed.len());
    if left == 0 {
        return 0;
    }
    if closed.contains(&cur) {
        closed.first().copied().unwrap_or(0).min(left - 1)
    } else {
        cur - closed.iter().filter(|&&c| c < cur).count()
    }
}

/// Like `after_close`, for windows that are shown together: when the window on screen is one of a group's and closes,
/// the group's window before it is shown (the way a closed pane gives the focus to the one before it), so the
/// split view stays on screen; only when none is left does the list decide. `of` is the group of each window
/// before the close.
pub fn after_close_in(of: &[Option<usize>], cur: usize, closed: &[usize]) -> usize {
    if closed.contains(&cur)
        && let Some(g) = of[cur]
        && let Some(prev) = (0..cur).rev().find(|&i| of[i] == Some(g) && !closed.contains(&i))
    {
        return prev - closed.iter().filter(|&&c| c < prev).count();
    }
    after_close(cur, closed, of.len())
}

/// Puts back what a close took out, each at the place it had (`items` are `(index before, item)`; those past
/// the end go last): the reference's `splice(min(index, len), 0, tab)` in ascending order.
pub fn reinsert<T>(list: &mut Vec<T>, mut items: Vec<(usize, T)>) {
    items.sort_by_key(|(i, _)| *i);
    for (i, t) in items {
        list.insert(i.min(list.len()), t);
    }
}

/// The index of `snap`'s group in `groups`, which is put back (at the end) when a close had dropped it.
pub fn resolve(groups: &mut Vec<Group>, snap: &Group) -> usize {
    index_of(groups, snap.id).unwrap_or_else(|| {
        groups.push(snap.clone());
        groups.len() - 1
    })
}

// --------------------------------------------------------------- templates --

/// What the dialog can create besides the windows it groups.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Template {
    #[default]
    None,
    One,
    Dev,
}

impl Template {
    pub const ALL: [Template; 3] = [Template::None, Template::One, Template::Dev];

    /// How many windows it opens.
    pub fn count(self) -> usize {
        match self {
            Template::None => 0,
            Template::One => 1,
            Template::Dev => 3,
        }
    }
}

/// What kind of project a folder holds, for the development template's commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Plain,
    Cargo,
    Npm,
    Go,
    Python,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Project {
    pub kind: Kind,
    /// The folder is in a git repository.
    pub git: bool,
}

/// The project a folder is, from the files it has (`has`) and whether it is in a git repository.
pub fn detect(has: &dyn Fn(&str) -> bool, git: bool) -> Project {
    let kind = if has("Cargo.toml") {
        Kind::Cargo
    } else if has("package.json") {
        Kind::Npm
    } else if has("go.mod") {
        Kind::Go
    } else if has("pyproject.toml") || has("setup.py") || has("requirements.txt") {
        Kind::Python
    } else {
        Kind::Plain
    };
    Project { kind, git }
}

/// `detect` on the real folder `dir`: its files, and a `.git` in it or above it.
pub fn project_at(dir: &Path) -> Project {
    detect(&|f| dir.join(f).exists(), dir.ancestors().any(|d| d.join(".git").exists()))
}

/// The commands the development template types: the server (`dev` of the reference), the tests, and git's state.
pub fn commands(p: Project) -> [Option<&'static str>; 3] {
    let (run, test) = match p.kind {
        Kind::Cargo => (Some("cargo run"), Some("cargo test")),
        Kind::Npm => (Some("npm run dev"), Some("npm test")),
        Kind::Go => (Some("go run ."), Some("go test ./...")),
        Kind::Python => (None, Some("pytest")),
        Kind::Plain => (None, None),
    };
    [run, test, p.git.then_some("git status")]
}

/// A window the template opens: its name (empty: the automatic one) and the command typed into its shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewWin {
    pub title: String,
    pub cmd: Option<String>,
}

/// The windows `t` opens in a folder of project `p`; `names` are the development template's three names.
pub fn template_wins(t: Template, p: Project, names: [&str; 3]) -> Vec<NewWin> {
    match t {
        Template::None => Vec::new(),
        Template::One => vec![NewWin { title: String::new(), cmd: None }],
        Template::Dev => commands(p).iter().zip(names).map(|(c, n)| NewWin { title: n.to_string(), cmd: c.map(str::to_string) }).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: u32, name: &str) -> Group {
        Group { id, name: name.into(), color: 0, collapsed: false, kind: panes::Kind::Single }
    }

    #[test]
    fn closing_the_window_on_screen_keeps_its_group_in_view() {
        let of = [Some(0), Some(0), Some(0), None, None];
        // The middle one of the group closes: the one before it takes the focus, in the shortened list.
        assert_eq!(after_close_in(&of, 1, &[1]), 0);
        assert_eq!(after_close_in(&of, 2, &[2]), 1);
        // The first of the group has none before it: the list decides (the one that took its place).
        assert_eq!(after_close_in(&of, 0, &[0]), 0);
        // Several closing at once: the nearest survivor before it.
        assert_eq!(after_close_in(&of, 2, &[1, 2]), 0);
        // A window out of the group, or one that is not on screen, is as `after_close` says.
        assert_eq!(after_close_in(&of, 3, &[3]), 3);
        assert_eq!(after_close_in(&of, 4, &[4]), 3);
        assert_eq!(after_close_in(&of, 2, &[0]), 1);
    }

    #[test]
    fn a_groups_windows_are_found_as_a_run_of_the_list() {
        // Groups in order, a group's windows together, the windows in none last (`App::rebuild_side`).
        let of = [Some(0), Some(0), Some(0), Some(1), Some(2), Some(2), None, None];
        let at = |g| span(of.iter().copied(), g);
        assert_eq!((at(0), at(1), at(2)), (Some((0, 3)), Some((3, 1)), Some((4, 2))));
        assert_eq!(at(3), None, "no window is in it");
        assert_eq!(span(std::iter::empty(), 0), None);
    }

    #[test]
    fn a_new_group_gets_the_first_colour_nobody_has_and_then_wraps() {
        assert_eq!(next_color(&[]), 0);
        assert_eq!(next_color(&[0, 1]), 2);
        assert_eq!(next_color(&[0, 2]), 1, "a hole is filled first");
        assert_eq!(next_color(&[0, 1, 2, 3, 4, 5, 6, 7]), 0, "all in use: 8 groups mod 8");
        assert_eq!(next_color(&[0, 1, 2, 3, 4, 5, 6, 7, 7]), 1);
    }

    #[test]
    fn the_suggested_name_is_the_folder_unless_it_says_nothing_and_never_one_in_use() {
        assert_eq!(suggest_name("axon", "group", &[]), "axon");
        assert_eq!(suggest_name("~", "group", &[]), "group");
        assert_eq!(suggest_name("", "group", &[]), "group");
        assert_eq!(suggest_name("axon", "group", &["axon"]), "axon 2");
        assert_eq!(suggest_name("axon", "group", &["axon", "Axon 2", "axon 3"]), "axon 4", "case does not make a name different");
        assert_eq!(suggest_name("~", "group", &["group", "group 2"]), "group 3");
        let long = suggest_name(&"x".repeat(60), "group", &[]);
        assert!(long.chars().count() <= NAME_MAX, "{long}");
        let taken = long.clone();
        assert!(suggest_name(&"x".repeat(60), "group", &[&taken]).chars().count() <= NAME_MAX);
        // What is suggested always passes the check.
        assert!(check(&suggest_name("axon", "group", &["axon"]), &["axon"], 1, 0, false).is_ok());
    }

    #[test]
    fn the_form_needs_a_name_nobody_has_and_a_window() {
        assert_eq!(check("api", &[], 1, 0, false), Ok("api".into()));
        assert_eq!(check("  api  ", &[], 0, 1, false), Ok("api".into()), "trimmed; a window to create is a window");
        assert_eq!(check("", &["a"], 1, 0, false), Err(FormError::NameEmpty));
        assert_eq!(check("   ", &[], 1, 0, false), Err(FormError::NameEmpty), "spaces are no name");
        assert_eq!(check("API", &["api"], 1, 0, false), Err(FormError::NameTaken), "case is ignored");
        assert_eq!(check(" api", &["api "], 1, 0, false), Err(FormError::NameTaken));
        assert_eq!(check("api", &[], 0, 0, false), Err(FormError::NothingChosen));
        assert_eq!(check("api", &[], 0, 0, true), Ok("api".into()), "an edit has no windows to choose");
        assert_eq!(check("", &[], 0, 0, false), Err(FormError::NameEmpty), "the name comes first");
    }

    #[test]
    fn a_rename_that_says_the_automatic_name_or_nothing_goes_back_to_the_automatic_one() {
        assert_eq!(rename_result("scratch", "axon"), "scratch");
        assert_eq!(rename_result("  scratch ", "axon"), "scratch");
        assert_eq!(rename_result("", "axon"), "");
        assert_eq!(rename_result("   ", "axon"), "");
        assert_eq!(rename_result("axon", "axon"), "");
    }

    #[test]
    fn sentences_get_their_names_in_order() {
        assert_eq!(fill("Group “{}” created · {}", &["api", "3 windows"]), "Group “api” created · 3 windows");
        assert_eq!(fill("no holes", &["x"]), "no holes");
        assert_eq!(fill("{} and {}", &["a"]), "a and ", "a missing argument is empty, not a panic");
        assert_eq!(fill("«{}»", &["{}"]), "«{}»", "an argument that looks like a hole is left alone");
    }

    #[test]
    fn a_group_nobody_is_in_goes_and_the_others_are_renumbered() {
        let mut groups = vec![g(1, "a"), g(2, "b"), g(3, "c")];
        let mut wins = vec![Some(0), None, Some(2), Some(2)];
        let gone = prune(&mut groups, &mut wins);
        assert_eq!(gone, [g(2, "b")]);
        assert_eq!(groups.iter().map(|g| g.id).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(wins, [Some(0), None, Some(1), Some(1)]);
        // Nothing to drop: nothing changes.
        assert!(prune(&mut groups, &mut wins).is_empty());
        // Every window gone from every group: all groups go.
        let mut none = vec![None, None];
        prune(&mut groups, &mut none);
        assert!(groups.is_empty());
    }

    #[test]
    fn deleting_a_group_keeps_its_windows_loose_and_the_other_groups_intact() {
        let mut groups = vec![g(1, "a"), g(2, "b")];
        let mut wins = vec![Some(0), Some(1), Some(0), None];
        dissolve(&mut wins, 0);
        assert_eq!(wins, [None, Some(1), None, None], "the windows are all still there");
        prune(&mut groups, &mut wins);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "b");
        assert_eq!(wins, [None, Some(0), None, None], "b is group 0 now");
        assert_eq!(index_of(&groups, 2), Some(0));
        assert_eq!(index_of(&groups, 1), None);
    }

    #[test]
    fn the_window_on_screen_after_a_close_is_the_same_or_the_one_that_took_its_place() {
        // Five windows; the current one is not closed: it only moves up by the closed ones before it.
        assert_eq!(after_close(3, &[0, 1], 5), 1);
        assert_eq!(after_close(1, &[2, 4], 5), 1);
        // The current one is closed: the one now at the first closed place, at most the last.
        assert_eq!(after_close(2, &[2], 5), 2);
        assert_eq!(after_close(4, &[4], 5), 3);
        assert_eq!(after_close(1, &[1, 2, 3], 5), 1);
        assert_eq!(after_close(3, &[1, 3, 4], 5), 1);
        assert_eq!(after_close(1, &[1, 2, 3, 4], 5), 0);
        // Not clamped: the first of the closed ones is the place, not the last (two of six leave four).
        assert_eq!(after_close(1, &[1, 2], 6), 1);
        assert_eq!(after_close(2, &[1, 2], 6), 1);
        // Nothing left: the caller made a placeholder first; 0 is as good as any.
        assert_eq!(after_close(0, &[0], 1), 0);
    }

    #[test]
    fn closed_windows_come_back_where_they_were_whatever_order_they_are_given() {
        let mut list = vec!['a', 'c', 'e'];
        // b (index 1), d (3) were taken out of a b c d e.
        reinsert(&mut list, vec![(3, 'd'), (1, 'b')]);
        assert_eq!(list, ['a', 'b', 'c', 'd', 'e']);
        // The list got shorter meanwhile: what is past the end goes last.
        let mut short = vec!['a'];
        reinsert(&mut short, vec![(4, 'z'), (2, 'y')]);
        assert_eq!(short, ['a', 'y', 'z']);
    }

    #[test]
    fn a_group_the_close_dropped_comes_back_and_one_that_stayed_is_found() {
        let mut groups = vec![g(1, "a"), g(3, "c")];
        assert_eq!(resolve(&mut groups, &g(3, "c")), 1, "still there");
        assert_eq!(resolve(&mut groups, &g(2, "b")), 2, "back at the end");
        assert_eq!(groups.len(), 3);
        assert_eq!(resolve(&mut groups, &g(2, "b")), 2, "once");
    }

    /// A close and its undo as the app does them, on a list of (name, group) pairs.
    #[test]
    fn closing_a_whole_group_and_undoing_it_restores_the_windows_and_the_group() {
        let mut groups = vec![g(1, "a"), g(2, "b")];
        let mut wins: Vec<(&str, Option<usize>)> = vec![("w0", Some(0)), ("w1", Some(1)), ("w2", Some(1)), ("w3", None)];
        // Close group b: w1 and w2, remembering each one's group.
        let closed = [1usize, 2];
        let snaps: Vec<_> = closed.iter().map(|&i| (i, wins[i], wins[i].1.map(|g| groups[g].clone()))).collect();
        for &i in closed.iter().rev() {
            wins.remove(i);
        }
        let mut of: Vec<_> = wins.iter().map(|w| w.1).collect();
        prune(&mut groups, &mut of);
        for (w, g) in wins.iter_mut().zip(&of) {
            w.1 = *g;
        }
        assert_eq!((groups.len(), wins.len()), (1, 2));
        assert_eq!(after_close(1, &closed, 4), 1);
        // Undo.
        let back: Vec<_> = snaps
            .into_iter()
            .map(|(i, w, snap)| {
                let g = snap.map(|s| resolve(&mut groups, &s));
                (i, (w.0, g))
            })
            .collect();
        reinsert(&mut wins, back);
        assert_eq!(wins, [("w0", Some(0)), ("w1", Some(1)), ("w2", Some(1)), ("w3", None)]);
        assert_eq!(groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn a_folder_is_the_project_its_files_say_and_git_is_its_own_question() {
        let files = |list: &'static [&'static str]| move |f: &str| list.contains(&f);
        assert_eq!(detect(&files(&["Cargo.toml", "package.json"]), true), Project { kind: Kind::Cargo, git: true }, "cargo before npm");
        assert_eq!(detect(&files(&["package.json"]), false).kind, Kind::Npm);
        assert_eq!(detect(&files(&["go.mod"]), false).kind, Kind::Go);
        assert_eq!(detect(&files(&["pyproject.toml"]), false).kind, Kind::Python);
        assert_eq!(detect(&files(&[]), false), Project::default());
    }

    #[test]
    fn a_real_folder_is_looked_at_for_its_files_and_the_git_directory_above_it() {
        let root = std::env::temp_dir().join(format!("axon-groups-{}", std::process::id()));
        let sub = root.join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("a/Cargo.toml"), "").unwrap();
        assert_eq!(project_at(&sub).kind, Kind::Plain);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        assert_eq!(project_at(&root.join("a")), Project { kind: Kind::Cargo, git: true });
        assert!(project_at(&sub).git, "the repository is above");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_templates_open_as_many_windows_as_they_say_and_the_development_one_types_what_the_project_runs() {
        let names = ["server", "tests", "git"];
        assert!(template_wins(Template::None, Project::default(), names).is_empty());
        assert_eq!(template_wins(Template::One, Project::default(), names), [NewWin { title: String::new(), cmd: None }]);
        for t in Template::ALL {
            assert_eq!(template_wins(t, Project { kind: Kind::Cargo, git: true }, names).len(), t.count());
        }
        let dev = template_wins(Template::Dev, Project { kind: Kind::Cargo, git: true }, names);
        assert_eq!(dev.iter().map(|w| (w.title.as_str(), w.cmd.as_deref())).collect::<Vec<_>>(), [("server", Some("cargo run")), ("tests", Some("cargo test")), ("git", Some("git status"))]);
        // No project and no repository: the windows are named and empty (typing a command that cannot run would only print an error).
        let bare = template_wins(Template::Dev, Project::default(), names);
        assert_eq!(bare.len(), 3);
        assert!(bare.iter().all(|w| w.cmd.is_none()));
        // Python has tests but no server.
        let py = commands(Project { kind: Kind::Python, git: false });
        assert_eq!(py, [None, Some("pytest"), None]);
    }
}
