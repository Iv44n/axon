//! What the command palette (`Ctrl+K`) lists and what each entry means: the reference's `paletteAll()`.
//!
//! Pure: the app describes its windows and groups in a `State`, `build` turns that into the palette's
//! `List` (entries in the reference's order: actions, windows, groups), and what the list reports for a
//! chosen entry is an id that `Cmd::from_id` turns back into the command the app runs. The ranking, the
//! headings and the highlighting are the list's (`list.rs`); this file only decides which entries exist.
//!
//! **Entries** (the reference's, with what axon can do):
//! - *Actions*: new window, new group, new pane, split in columns / rows, the windows' panel (show or hide,
//!   left, right, top), search the history, clear the screen, settings, close the window; the overview only
//!   when the app has one (`State::overview`). Two more are listed, after the reference's, only where they apply:
//!   close the pane (the window has several) and take the window out of its group (it is in one).
//! - *Windows*: one per window, its directory and group as the description, `Alt+1..9` as the shortcut.
//! - *Groups*: one to go to the group's first window, one to collapse or expand it in the panel.
//!
//! The reference's *History* entries (its last eight commands, typed into its own editor) have no
//! counterpart: the shell owns the line, axon never sees the commands, so the palette offers *Search the
//! history* (the shell's `Ctrl+R`) instead. The reference's search box says "workflows" but defines none.
use crate::{
    config::TabsPos,
    i18n::{Str, tr},
    icons::Icon,
    list::{Entry, List, Mark},
    side::GroupItem,
};

/// The tag the palette is opened under (what `Ran` is matched on).
pub const TAG: u32 = 0x434d_444b;

/// What the palette can ask the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    NewWindow,
    NewGroup,
    NewPane,
    SplitCols,
    SplitRows,
    Overview,
    TogglePanel,
    Tabs(TabsPos),
    HistorySearch,
    ClearScreen,
    Settings,
    CloseWindow,
    ClosePane,
    Ungroup,
    /// Go to the window with this id (`short` of its `Win::id`). Ids, not places: while the palette is open a shell
    /// can end or a close can be undone, and a place would then be another window.
    Window(u32),
    /// Go to the first window of the group with this id (`Group::id`).
    Group(u32),
    /// Collapse or expand the group with this id.
    Fold(u32),
}

/// The commands with nothing to say but their name, in the order of their ids.
const FIXED: [Cmd; 16] = [
    Cmd::NewWindow,
    Cmd::NewGroup,
    Cmd::NewPane,
    Cmd::SplitCols,
    Cmd::SplitRows,
    Cmd::Overview,
    Cmd::TogglePanel,
    Cmd::Tabs(TabsPos::Left),
    Cmd::Tabs(TabsPos::Right),
    Cmd::Tabs(TabsPos::Top),
    Cmd::HistorySearch,
    Cmd::ClearScreen,
    Cmd::Settings,
    Cmd::CloseWindow,
    Cmd::ClosePane,
    Cmd::Ungroup,
];

/// The id of a command that takes a number: the kind in the two high bits, the number in the other thirty.
const KIND: u32 = 3 << 30;
const WINDOW: u32 = 1 << 30;
const GROUP: u32 = 2 << 30;
const FOLD: u32 = 3 << 30;
/// The largest number a window or a group can carry (a window's id counts every window ever opened).
const MAX_N: u32 = !KIND;

/// A window's id as `Cmd::Window` carries it.
pub fn short(id: u64) -> u32 {
    (id & MAX_N as u64) as u32
}

impl Cmd {
    /// The number the list carries for this command (`Entry::id`).
    pub fn id(self) -> u32 {
        match self {
            Cmd::Window(n) => WINDOW | n.min(MAX_N),
            Cmd::Group(n) => GROUP | n.min(MAX_N),
            Cmd::Fold(n) => FOLD | n.min(MAX_N),
            // The rest are numbered from 1 in the order of `FIXED`.
            c => FIXED.iter().position(|&f| f == c).map_or(0, |i| i as u32 + 1),
        }
    }

    /// The command an id stands for.
    pub fn from_id(id: u32) -> Option<Cmd> {
        let n = id & MAX_N;
        match id & KIND {
            0 => FIXED.get((id as usize).checked_sub(1)?).copied(),
            WINDOW => Some(Cmd::Window(n)),
            GROUP => Some(Cmd::Group(n)),
            FOLD => Some(Cmd::Fold(n)),
            _ => None,
        }
    }
}

/// A window as the palette lists it.
pub struct WinInfo {
    /// Its `Win::id`.
    pub id: u64,
    pub title: String,
    /// Its directory, in short form.
    pub place: String,
    /// The group it is in (an index into `State::groups`).
    pub group: Option<usize>,
}

/// What the palette is built from: the app's windows and groups at the moment it opens.
pub struct State<'a> {
    pub wins: &'a [WinInfo],
    pub groups: &'a [GroupItem],
    /// The window on screen and how many panes it has.
    pub cur: usize,
    pub panes: usize,
    /// Whether the app has an overview of its windows to open.
    pub overview: bool,
}

/// Text that finds an entry without being shown, in both languages: the reference's `kw` plus the English
/// for each, so the palette finds a command whatever language the window speaks.
mod kw {
    pub const NEW_WINDOW: &str = "tab pestaña";
    pub const NEW_GROUP: &str = "agrupar";
    pub const NEW_PANE: &str = "split dividir panel pane space espacio";
    pub const SPLIT: &str = "pane panel dividir";
    pub const OVERVIEW: &str = "ventanas tarjetas windows cards";
    pub const PANEL: &str = "lateral barra sidebar side";
    pub const PLACE: &str = "panel posición lateral position side tabs pestañas";
    pub const HISTORY: &str = "comandos anteriores previous commands";
    pub const CLEAR: &str = "clear borrar";
    pub const SETTINGS: &str = "fuente tamaño opacidad chips font size opacity preferences";
    pub const CLOSE: &str = "cerrar close";
    pub const CLOSE_PANE: &str = "cerrar close split dividir";
    pub const UNGROUP: &str = "grupo group disolver quitar remove";
    pub const GROUP: &str = "grupo group";
    pub const FOLD: &str = "grupo group fold plegar collapse expand contraer expandir";
}

/// The shortcut of the command that opens window `i`: `Alt+1` to `Alt+9`, none beyond.
fn window_keys(i: usize) -> String {
    if i < 9 { format!("Alt+{}", i + 1) } else { String::new() }
}

/// "3 windows", "1 window".
fn count(n: usize) -> String {
    format!("{n} {}", tr(if n == 1 { Str::CmdWindowOne } else { Str::CmdWindowMany }))
}

/// The entries the palette lists for `s`, filtered by an empty query (so every one is shown, under its heading).
pub fn build(s: &State) -> List {
    let mut list = List::default();
    let actions = tr(Str::OvGroupActions);
    let space = format!("Ctrl+{}", tr(Str::KeySpace));
    let mut act = |cmd: Cmd, label: &str, keys: &str, icon: Icon, keywords: &str| {
        list.push(Entry { label, keys, keywords, group: actions, mark: Mark::Icon(icon), id: cmd.id(), ..Entry::default() });
    };
    act(Cmd::NewWindow, tr(Str::OvActNewWindow), "Alt+N", Icon::Plus, kw::NEW_WINDOW);
    act(Cmd::NewGroup, tr(Str::OvActNewGroup), "Alt+G", Icon::Group, kw::NEW_GROUP);
    act(Cmd::NewPane, tr(Str::CmdNewPane), &space, Icon::Split, kw::NEW_PANE);
    act(Cmd::SplitCols, tr(Str::OvActSplit), "Alt+\\", Icon::Split, kw::SPLIT);
    act(Cmd::SplitRows, tr(Str::CmdSplitRows), "Alt+-", Icon::Split, kw::SPLIT);
    if s.overview {
        act(Cmd::Overview, tr(Str::OvActOverview), "Alt+O", Icon::Grid, kw::OVERVIEW);
    }
    act(Cmd::TogglePanel, tr(Str::OvActPanel), "Alt+B", Icon::Panel, kw::PANEL);
    act(Cmd::Tabs(TabsPos::Left), tr(Str::CmdTabsLeft), "", Icon::Panel, kw::PLACE);
    act(Cmd::Tabs(TabsPos::Right), tr(Str::CmdTabsRight), "", Icon::Panel, kw::PLACE);
    act(Cmd::Tabs(TabsPos::Top), tr(Str::CmdTabsTop), "", Icon::Panel, kw::PLACE);
    act(Cmd::HistorySearch, tr(Str::CmdHistory), "Ctrl+R", Icon::Lines, kw::HISTORY);
    act(Cmd::ClearScreen, tr(Str::CmdClear), "Ctrl+L", Icon::Lines, kw::CLEAR);
    act(Cmd::Settings, tr(Str::SettingsTitle), "Ctrl+,", Icon::Gear, kw::SETTINGS);
    act(Cmd::CloseWindow, tr(Str::OvActClose), "Alt+W", Icon::Cross, kw::CLOSE);
    if s.panes > 1 {
        act(Cmd::ClosePane, tr(Str::CmdClosePane), "Alt+Shift+W", Icon::Cross, kw::CLOSE_PANE);
    }
    if s.wins.get(s.cur).is_some_and(|w| w.group.is_some()) {
        act(Cmd::Ungroup, tr(Str::CmdUngroup), "Alt+Shift+G", Icon::Group, kw::UNGROUP);
    }

    let windows = tr(Str::Windows);
    for (i, w) in s.wins.iter().enumerate() {
        let group = w.group.and_then(|g| s.groups.get(g));
        let desc = match group {
            Some(g) => format!("{} \u{b7} {}", w.place, g.name),
            None => w.place.clone(),
        };
        let keys = window_keys(i);
        let keywords = group.map_or("", |g| g.name.as_str());
        list.push(Entry { label: &w.title, desc: &desc, keywords, keys: &keys, group: windows, mark: Mark::Icon(Icon::Prompt), id: Cmd::Window(short(w.id)).id() });
    }

    let heading = tr(Str::OvGroupGroups);
    for x in s.groups {
        list.push(Entry { label: &x.name, desc: &count(x.count), keywords: kw::GROUP, group: heading, mark: Mark::Dot(x.color), id: Cmd::Group(x.id).id(), ..Entry::default() });
    }
    for x in s.groups {
        let verb = tr(if x.collapsed { Str::CmdUnfold } else { Str::CmdFold });
        let label = format!("{verb} {}", x.name);
        list.push(Entry { label: &label, keywords: kw::FOLD, group: heading, mark: Mark::Dot(x.color), id: Cmd::Fold(x.id).id(), ..Entry::default() });
    }
    list.filter("");
    list
}

/// A window of the design board and of the tests: a group of three windows and one apart.
#[cfg(test)]
pub fn sample() -> (Vec<WinInfo>, Vec<GroupItem>) {
    // Ids that are their places, so that the tests read `Window(1)` for the second one.
    let win = |id, title: &str, place: &str, group| WinInfo { id, title: title.into(), place: place.into(), group };
    (
        vec![
            win(0, "arcton", "~/lab/apps/arcton", Some(0)),
            win(1, "servidor", "~/lab/apps/arcton", Some(0)),
            win(2, "pruebas", "~/lab/apps/arcton", Some(0)),
            win(3, "~", "~", None),
        ],
        vec![GroupItem { id: 0, name: "arcton".into(), color: 1, collapsed: false, count: 3, busy: false }],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state<'a>(wins: &'a [WinInfo], groups: &'a [GroupItem]) -> State<'a> {
        // The window on screen is the one in no group.
        State { wins, groups, cur: 3, panes: 1, overview: false }
    }

    /// The commands the list shows, in order.
    fn shown(l: &List) -> Vec<Cmd> {
        (0..l.len()).map(|n| Cmd::from_id(l.id(l.index(n).unwrap())).unwrap()).collect()
    }

    /// What `query` keeps, best first.
    fn found(wins: &[WinInfo], groups: &[GroupItem], query: &str) -> Vec<Cmd> {
        let mut l = build(&state(wins, groups));
        l.filter(query);
        shown(&l)
    }

    #[test]
    fn every_command_has_its_own_id_and_the_id_gives_it_back() {
        let mut ids = std::collections::HashSet::new();
        let all = FIXED.iter().copied().chain([Cmd::Window(0), Cmd::Window(9), Cmd::Window(MAX_N), Cmd::Group(0), Cmd::Group(3), Cmd::Fold(0), Cmd::Fold(7), Cmd::Fold(MAX_N)]);
        for c in all {
            assert!(ids.insert(c.id()), "{c:?} shares its id");
            assert_eq!(Cmd::from_id(c.id()), Some(c), "{c:?}");
        }
        assert_eq!(Cmd::from_id(0), None, "zero is nothing");
        assert_eq!(Cmd::from_id(FIXED.len() as u32 + 1), None);
        assert_eq!(Cmd::from_id(4 << 16), None);
    }

    #[test]
    fn the_palette_lists_the_actions_then_the_windows_then_the_groups() {
        let (wins, groups) = sample();
        let l = build(&state(&wins, &groups));
        let cmds = shown(&l);
        // The reference's fourteen actions, less the overview this app has not got, then each window, then the group's two.
        assert_eq!(cmds.len(), 13 + 4 + 2, "{cmds:?}");
        assert_eq!(&cmds[..3], &[Cmd::NewWindow, Cmd::NewGroup, Cmd::NewPane], "the reference's order");
        assert_eq!(cmds[12], Cmd::CloseWindow);
        assert_eq!(&cmds[13..], &[Cmd::Window(0), Cmd::Window(1), Cmd::Window(2), Cmd::Window(3), Cmd::Group(0), Cmd::Fold(0)]);
        assert!(!cmds.contains(&Cmd::Overview), "no overview, no entry");
        // The groups are the headings of a list with no query: three, in that order.
        let heads: Vec<&str> = (0..l.len()).map(|n| l.group(l.index(n).unwrap())).fold(Vec::new(), |mut v, g| {
            if v.last() != Some(&g) {
                v.push(g);
            }
            v
        });
        assert_eq!(heads, [tr(Str::OvGroupActions), tr(Str::Windows), tr(Str::OvGroupGroups)]);
    }

    #[test]
    fn the_first_fourteen_rows_are_the_references_actions_in_its_order() {
        let (wins, groups) = sample();
        // Whatever the window on screen is: the reference's list is never reordered, the extras come after it.
        for (cur, panes) in [(3, 1), (0, 1), (0, 3)] {
            let cmds = shown(&build(&State { wins: &wins, groups: &groups, cur, panes, overview: true }));
            assert_eq!(
                &cmds[..14],
                [
                    Cmd::NewWindow,
                    Cmd::NewGroup,
                    Cmd::NewPane,
                    Cmd::SplitCols,
                    Cmd::SplitRows,
                    Cmd::Overview,
                    Cmd::TogglePanel,
                    Cmd::Tabs(TabsPos::Left),
                    Cmd::Tabs(TabsPos::Right),
                    Cmd::Tabs(TabsPos::Top),
                    Cmd::HistorySearch,
                    Cmd::ClearScreen,
                    Cmd::Settings,
                    Cmd::CloseWindow,
                ],
                "cur {cur}, {panes} panes"
            );
        }
    }

    #[test]
    fn some_actions_are_listed_only_where_they_apply() {
        let (wins, groups) = sample();
        let with = |cur, panes, overview| {
            let l = build(&State { wins: &wins, groups: &groups, cur, panes, overview });
            shown(&l)
        };
        // Window 0 is in a group: it can leave it; window 3 is not.
        assert!(with(0, 1, false).contains(&Cmd::Ungroup));
        assert!(!with(3, 1, false).contains(&Cmd::Ungroup));
        // Closing a pane needs more than one.
        assert!(!with(0, 1, false).contains(&Cmd::ClosePane));
        assert!(with(0, 2, false).contains(&Cmd::ClosePane));
        // The overview, only once the app has one, in the reference's place (after the splits).
        let cmds = with(0, 1, true);
        let at = cmds.iter().position(|&c| c == Cmd::Overview).unwrap();
        assert_eq!(cmds[at - 1], Cmd::SplitRows);
        assert_eq!(cmds[at + 1], Cmd::TogglePanel);
        // Both extras come after the reference's last action, close the pane first.
        let cmds = with(0, 2, true);
        let close = cmds.iter().position(|&c| c == Cmd::CloseWindow).unwrap();
        assert_eq!(&cmds[close + 1..close + 3], &[Cmd::ClosePane, Cmd::Ungroup]);
        // No windows at all (the app always has one, but the list must not index past what it was given).
        assert!(shown(&build(&State { wins: &[], groups: &[], cur: 0, panes: 1, overview: false })).len() >= 13);
    }

    #[test]
    fn a_window_shows_its_directory_and_group_and_the_first_nine_have_their_alt_digit() {
        let (wins, groups) = sample();
        let l = build(&state(&wins, &groups));
        let entry = |c: Cmd| (0..l.total()).find(|&e| l.id(e) == c.id()).unwrap();
        let w = entry(Cmd::Window(1));
        assert_eq!((l.label(w), l.desc(w), l.keys(w)), ("servidor", "~/lab/apps/arcton \u{b7} arcton", "Alt+2"));
        let loose = entry(Cmd::Window(3));
        assert_eq!((l.desc(loose), l.keys(loose)), ("~", "Alt+4"), "a window in no group says only where it is");
        assert_eq!(l.mark(w), Mark::Icon(Icon::Prompt));
        let g = entry(Cmd::Group(0));
        assert_eq!((l.label(g), l.desc(g), l.mark(g)), ("arcton", "3 windows", Mark::Dot(1)));
        // Ten windows: the tenth has no digit.
        let many: Vec<WinInfo> = (0..10).map(|i| WinInfo { id: i, title: format!("w{i}"), place: "~".into(), group: None }).collect();
        let l = build(&State { wins: &many, groups: &[], cur: 0, panes: 1, overview: false });
        let tenth = (0..l.total()).find(|&e| l.id(e) == Cmd::Window(9).id()).unwrap();
        assert_eq!(l.keys(tenth), "");
        // A group of one says "1 window".
        let one = [GroupItem { id: 0, name: "solo".into(), color: 0, collapsed: false, count: 1, busy: false }];
        let l = build(&state(&wins, &one));
        let g = (0..l.total()).find(|&e| l.id(e) == Cmd::Group(0).id()).unwrap();
        assert_eq!(l.desc(g), "1 window");
    }

    #[test]
    fn a_window_or_group_entry_names_its_id_and_not_its_place() {
        // Window 7 is the first in the list and group 5 the only one: when the list changes under the palette (a shell
        // ends, a close is undone) the entry still means the same window and group.
        let win = |id, title: &str, group| WinInfo { id, title: title.into(), place: "~".into(), group };
        let wins = [win(7, "a", Some(0)), win(2, "b", None)];
        let groups = [GroupItem { id: 5, name: "g".into(), color: 0, collapsed: false, count: 1, busy: false }];
        let cmds = shown(&build(&state(&wins, &groups)));
        assert_eq!(&cmds[cmds.len() - 4..], &[Cmd::Window(7), Cmd::Window(2), Cmd::Group(5), Cmd::Fold(5)]);
        // A window's id is long: what does not fit in an entry's number is dropped, as the app's lookup does.
        assert_eq!(short(7), 7);
        assert_eq!(short((1 << 40) | 9), 9);
    }

    #[test]
    fn the_actions_show_the_shortcuts_that_run_them() {
        let (wins, groups) = sample();
        let l = build(&State { wins: &wins, groups: &groups, cur: 0, panes: 2, overview: true });
        let keys = |c: Cmd| (0..l.total()).find(|&e| l.id(e) == c.id()).map(|e| l.keys(e).to_string()).unwrap();
        assert_eq!(keys(Cmd::NewWindow), "Alt+N");
        assert_eq!(keys(Cmd::NewGroup), "Alt+G");
        assert_eq!(keys(Cmd::Ungroup), "Alt+Shift+G");
        assert_eq!(keys(Cmd::SplitCols), "Alt+\\");
        assert_eq!(keys(Cmd::SplitRows), "Alt+-");
        assert_eq!(keys(Cmd::Overview), "Alt+O");
        assert_eq!(keys(Cmd::TogglePanel), "Alt+B");
        assert_eq!(keys(Cmd::Settings), "Ctrl+,");
        assert_eq!(keys(Cmd::CloseWindow), "Alt+W");
        assert_eq!(keys(Cmd::ClosePane), "Alt+Shift+W");
        assert!(keys(Cmd::NewPane).starts_with("Ctrl+"));
        for p in [TabsPos::Left, TabsPos::Right, TabsPos::Top] {
            assert_eq!(keys(Cmd::Tabs(p)), "", "the panel's place has no shortcut");
        }
    }

    #[test]
    fn typing_finds_windows_by_their_group_and_their_directory() {
        let (wins, groups) = sample();
        // «servidor»: its own title first.
        assert_eq!(found(&wins, &groups, "servidor")[0], Cmd::Window(1));
        // A directory finds the three windows that are there, the one whose title comes first ranked first.
        let r = found(&wins, &groups, "apps/arcton");
        assert_eq!(r, [Cmd::Window(0), Cmd::Window(2), Cmd::Window(1)], "by where the query sits in «title directory»");
        // The group's name finds its windows (by the group in the description), the group, and the fold command.
        let r = found(&wins, &groups, "arcton");
        for c in [Cmd::Window(0), Cmd::Group(0), Cmd::Fold(0)] {
            assert!(r.contains(&c), "{c:?} in {r:?}");
        }
        assert!(!r.contains(&Cmd::Window(3)), "the loose window is not in it");
        // Accents and case do not matter, in either direction.
        assert_eq!(found(&wins, &groups, "PESTANA")[0], Cmd::NewWindow);
        assert_eq!(found(&wins, &groups, "pestaña")[0], Cmd::NewWindow);
    }

    #[test]
    fn a_query_in_either_language_finds_the_commands_by_their_hidden_words() {
        let (wins, groups) = sample();
        let first = |q: &str| found(&wins, &groups, q).first().copied();
        assert_eq!(first("clear"), Some(Cmd::ClearScreen));
        assert_eq!(first("borrar"), Some(Cmd::ClearScreen));
        assert_eq!(first("sidebar"), Some(Cmd::TogglePanel));
        assert_eq!(first("lateral"), Some(Cmd::TogglePanel));
        assert_eq!(first("preferences"), Some(Cmd::Settings));
        assert_eq!(first("opacidad"), Some(Cmd::Settings));
        // The letters in order are enough, but a whole query is ranked above them.
        let r = found(&wins, &groups, "nwt");
        assert!(r.contains(&Cmd::NewWindow), "N-ew W-indow T-ab in order: {r:?}");
        assert!(found(&wins, &groups, "zzzz").is_empty());
        // Four ways to the same thing rank by where the query sits: «tab» is in the new window's hidden words.
        assert_eq!(first("tab"), Some(Cmd::NewWindow));
    }

    #[test]
    fn collapsing_says_expand_once_the_group_is_collapsed() {
        let (wins, mut groups) = sample();
        let label = |groups: &[GroupItem]| {
            let l = build(&state(&wins, groups));
            let e = (0..l.total()).find(|&e| l.id(e) == Cmd::Fold(0).id()).unwrap();
            l.label(e).to_string()
        };
        let open = label(&groups);
        groups[0].collapsed = true;
        let shut = label(&groups);
        assert_ne!(open, shut);
        assert!(open.ends_with("arcton") && shut.ends_with("arcton"), "{open} / {shut}");
    }

    #[test]
    fn the_selection_runs_the_command_the_row_stands_for() {
        // What the app does with `Nav::Chose`: the id of the entry under the selection is the command.
        let (wins, groups) = sample();
        let mut l = build(&state(&wins, &groups));
        l.filter("servidor");
        let e = l.chosen().unwrap();
        assert_eq!(Cmd::from_id(l.id(e)), Some(Cmd::Window(1)));
    }
}
