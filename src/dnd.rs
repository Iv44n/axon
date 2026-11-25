//! Drag and drop in the list of windows (the panel or the tabs): what can be grabbed, where it can land and
//! what the windows and groups look like afterwards. Pure: no window, no pixels; `side.rs` finds the
//! target under the pointer and paints the marks, `wlapp.rs` applies the result.
//!
//! The rules are the reference's (`terminal (2).html`, "arrastrar pestañas y grupos"): a window dropped on a
//! window goes next to it and takes its group; on a group's header it goes into the group, first; on the empty
//! part of the list it leaves its group and goes last. A group dropped on a group or on a window of one goes
//! before or after that group. The one rule that is ours is ADR 0023's: the windows stay sorted (groups in
//! order, each group's windows together, the windows in none last), so a group dropped near a loose window ends
//! up after the last group, and a loose window cannot sit between two groups.

/// How far the pointer must move after a press before it is a drag and not a click.
pub const THRESHOLD: f32 = 5.0;

/// Whether the pointer, pressed at `from` and now at `at`, has moved far enough to be dragging.
pub fn started(from: (f32, f32), at: (f32, f32)) -> bool {
    (at.0 - from.0).hypot(at.1 - from.1) >= THRESHOLD
}

/// What is being dragged: a window, or a group (by its header or chip), by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    Win(usize),
    Group(usize),
}

/// Where a drop would put what is dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A window next to window `win`, before it or after it, in the group `win` is in (or in none).
    Near { win: usize, after: bool },
    /// A window into group `g`, first among its windows.
    Into(usize),
    /// A group before or after group `group`.
    Beside { group: usize, after: bool },
    /// A window out of its group, after every other.
    End,
}

/// The windows and groups after a drop, as permutations of what there was.
#[derive(Debug, PartialEq, Eq)]
pub struct Arrangement {
    /// The windows in their new order, each as the index it had.
    pub order: Vec<usize>,
    /// The group of each window of `order`, in the new numbering of the groups.
    pub group: Vec<Option<usize>>,
    /// The groups in their new order, each as the index it had. A group left with no window is not in it.
    pub groups: Vec<usize>,
    /// The group (new numbering) a window went into: it is unfolded so the window shows.
    pub unfold: Option<usize>,
}

/// What `grab` dropped on `to` does to windows whose groups are `win_group` (one entry per window, in the order
/// of the list) out of `n_groups` groups. `None` when nothing would change (dropped on itself, or where it was).
pub fn arrange(win_group: &[Option<usize>], n_groups: usize, grab: Grab, to: Target) -> Option<Arrangement> {
    let mut list: Vec<(usize, Option<usize>)> = win_group.iter().copied().enumerate().collect();
    let mut gorder: Vec<usize> = (0..n_groups).collect();
    let mut unfold = None;
    match (grab, to) {
        (Grab::Win(w), Target::Near { win, after }) => {
            if win == w {
                return None;
            }
            let at = list.iter().position(|&(i, _)| i == w)?;
            let mut me = list.remove(at);
            let t = list.iter().position(|&(i, _)| i == win)?;
            me.1 = list[t].1;
            list.insert(t + after as usize, me);
        }
        (Grab::Win(w), Target::Into(g)) => {
            if g >= n_groups {
                return None;
            }
            let at = list.iter().position(|&(i, _)| i == w)?;
            let mut me = list.remove(at);
            me.1 = Some(g);
            unfold = Some(g);
            let first = list.iter().position(|&(_, x)| x == Some(g)).unwrap_or(list.len());
            list.insert(first, me);
        }
        (Grab::Win(w), Target::End) => {
            let at = list.iter().position(|&(i, _)| i == w)?;
            let mut me = list.remove(at);
            me.1 = None;
            list.push(me);
        }
        (Grab::Group(g), Target::Beside { group: h, after }) => {
            if g == h || g >= n_groups || h >= n_groups {
                return None;
            }
            gorder.retain(|&x| x != g);
            let at = gorder.iter().position(|&x| x == h)?;
            gorder.insert(at + after as usize, g);
        }
        _ => return None,
    }
    // The windows in the order of their groups, those in none last; the sort is stable, so each group's
    // windows keep the order they were put in.
    let rank = |g: Option<usize>| g.map_or(usize::MAX, |g| gorder.iter().position(|&x| x == g).unwrap_or(usize::MAX - 1));
    list.sort_by_key(|&(_, g)| rank(g));
    gorder.retain(|&g| list.iter().any(|&(_, x)| x == Some(g)));
    let renumber = |g: usize| gorder.iter().position(|&x| x == g);
    let order: Vec<usize> = list.iter().map(|&(i, _)| i).collect();
    let group: Vec<Option<usize>> = list.iter().map(|&(_, g)| g.and_then(renumber)).collect();
    let unfold = unfold.and_then(renumber);
    let unmoved = order.iter().copied().eq(0..win_group.len()) && group == win_group && gorder.iter().copied().eq(0..n_groups);
    (!unmoved).then_some(Arrangement { order, group, groups: gorder, unfold })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Option<usize> = None;

    fn near(win: usize, after: bool) -> Target {
        Target::Near { win, after }
    }

    #[test]
    fn a_press_becomes_a_drag_after_a_few_pixels() {
        assert!(!started((10.0, 10.0), (12.0, 12.0)));
        assert!(!started((10.0, 10.0), (10.0, 14.9)));
        assert!(started((10.0, 10.0), (10.0, 15.0)));
        assert!(started((10.0, 10.0), (14.0, 14.0)), "the distance counts, not each axis");
    }

    #[test]
    fn loose_windows_are_reordered_before_or_after_another() {
        let wg = [NONE, NONE, NONE, NONE];
        let a = arrange(&wg, 0, Grab::Win(0), near(2, true)).unwrap();
        assert_eq!(a.order, [1, 2, 0, 3]);
        assert_eq!(a.group, [NONE; 4]);
        let a = arrange(&wg, 0, Grab::Win(3), near(1, false)).unwrap();
        assert_eq!(a.order, [0, 3, 1, 2]);
        // Dropped on itself, or where it already is: nothing changes.
        assert_eq!(arrange(&wg, 0, Grab::Win(1), near(1, false)), None);
        assert_eq!(arrange(&wg, 0, Grab::Win(1), near(2, false)), None, "before its next one is where it is");
        assert_eq!(arrange(&wg, 0, Grab::Win(1), near(0, true)), None, "after its previous one is where it is");
    }

    #[test]
    fn a_window_dropped_on_a_window_takes_its_group() {
        // Windows 0 and 1 in group 0, 2 in group 1, 3 and 4 in none.
        let wg = [Some(0), Some(0), Some(1), NONE, NONE];
        // A loose window next to one in group 0, after it: it joins the group, in that place.
        let a = arrange(&wg, 2, Grab::Win(3), near(0, true)).unwrap();
        assert_eq!(a.order, [0, 3, 1, 2, 4]);
        assert_eq!(a.group, [Some(0), Some(0), Some(0), Some(1), NONE]);
        assert_eq!(a.groups, [0, 1]);
        assert_eq!(a.unfold, None, "dropping next to a window leaves the group's fold alone");
        // A window of group 0 next to one of group 1: it changes group.
        let a = arrange(&wg, 2, Grab::Win(1), near(2, false)).unwrap();
        assert_eq!(a.order, [0, 1, 2, 3, 4]);
        assert_eq!(a.group, [Some(0), Some(1), Some(1), NONE, NONE]);
        // A grouped window next to a loose one leaves its group.
        let a = arrange(&wg, 2, Grab::Win(0), near(4, false)).unwrap();
        assert_eq!(a.order, [1, 2, 3, 0, 4]);
        assert_eq!(a.group, [Some(0), Some(1), NONE, NONE, NONE]);
    }

    #[test]
    fn a_window_dropped_on_a_header_goes_into_the_group_first_and_unfolds_it() {
        let wg = [Some(0), Some(0), Some(1), NONE];
        let a = arrange(&wg, 2, Grab::Win(3), Target::Into(0)).unwrap();
        assert_eq!(a.order, [3, 0, 1, 2]);
        assert_eq!(a.group, [Some(0), Some(0), Some(0), Some(1)]);
        assert_eq!(a.unfold, Some(0));
        // From one group into another: first in the new one.
        let a = arrange(&wg, 2, Grab::Win(1), Target::Into(1)).unwrap();
        assert_eq!(a.order, [0, 1, 2, 3]);
        assert_eq!(a.group, [Some(0), Some(1), Some(1), NONE]);
        // Into the group it is already first in: nothing to do.
        assert_eq!(arrange(&wg, 2, Grab::Win(0), Target::Into(0)), None);
        // Into a group that is not there.
        assert_eq!(arrange(&wg, 2, Grab::Win(3), Target::Into(2)), None);
    }

    #[test]
    fn a_window_dropped_on_the_empty_part_leaves_its_group_and_goes_last() {
        let wg = [Some(0), Some(0), Some(1), NONE];
        let a = arrange(&wg, 2, Grab::Win(0), Target::End).unwrap();
        assert_eq!(a.order, [1, 2, 3, 0]);
        assert_eq!(a.group, [Some(0), Some(1), NONE, NONE]);
        // The last loose window is already last.
        assert_eq!(arrange(&wg, 2, Grab::Win(3), Target::End), None);
    }

    #[test]
    fn a_group_left_with_no_window_is_dropped_and_the_others_renumbered() {
        // Group 0 has only window 0; group 1 has 1 and 2; window 3 is loose.
        let wg = [Some(0), Some(1), Some(1), NONE];
        let a = arrange(&wg, 2, Grab::Win(0), Target::End).unwrap();
        assert_eq!(a.order, [1, 2, 3, 0]);
        assert_eq!(a.group, [Some(0), Some(0), NONE, NONE]);
        assert_eq!(a.groups, [1], "the old group 1 is now the group 0");
        // Moving the only window of group 1's neighbour into it: group 0 goes, group 1 becomes 0, unfold follows.
        let a = arrange(&wg, 2, Grab::Win(0), Target::Into(1)).unwrap();
        assert_eq!(a.groups, [1]);
        assert_eq!(a.order, [0, 1, 2, 3]);
        assert_eq!(a.group, [Some(0), Some(0), Some(0), NONE]);
        assert_eq!(a.unfold, Some(0));
    }

    #[test]
    fn a_group_is_moved_before_or_after_another_with_its_windows() {
        // Three groups of one, two and one windows, and a loose window.
        let wg = [Some(0), Some(1), Some(1), Some(2), NONE];
        let a = arrange(&wg, 3, Grab::Group(0), Target::Beside { group: 2, after: true }).unwrap();
        assert_eq!(a.groups, [1, 2, 0]);
        assert_eq!(a.order, [1, 2, 3, 0, 4]);
        assert_eq!(a.group, [Some(0), Some(0), Some(1), Some(2), NONE]);
        let a = arrange(&wg, 3, Grab::Group(2), Target::Beside { group: 0, after: false }).unwrap();
        assert_eq!(a.groups, [2, 0, 1]);
        assert_eq!(a.order, [3, 0, 1, 2, 4]);
        assert_eq!(a.group, [Some(0), Some(1), Some(2), Some(2), NONE]);
        // Onto itself, onto the neighbour it is already before: nothing.
        assert_eq!(arrange(&wg, 3, Grab::Group(1), Target::Beside { group: 1, after: true }), None);
        assert_eq!(arrange(&wg, 3, Grab::Group(1), Target::Beside { group: 2, after: false }), None);
        assert_eq!(arrange(&wg, 3, Grab::Group(1), Target::Beside { group: 0, after: true }), None);
    }

    #[test]
    fn what_cannot_be_dropped_where_it_is_aimed_does_nothing() {
        let wg = [Some(0), NONE];
        assert_eq!(arrange(&wg, 1, Grab::Group(0), Target::End), None);
        assert_eq!(arrange(&wg, 1, Grab::Group(0), Target::Into(0)), None);
        assert_eq!(arrange(&wg, 1, Grab::Win(0), Target::Beside { group: 0, after: true }), None);
        assert_eq!(arrange(&wg, 1, Grab::Win(5), Target::End), None, "a window that is not there");
        assert_eq!(arrange(&wg, 1, Grab::Win(0), near(7, true)), None, "a target that is not there");
        assert_eq!(arrange(&wg, 1, Grab::Group(3), Target::Beside { group: 0, after: true }), None);
    }

    #[test]
    fn whatever_is_dropped_the_windows_stay_sorted_and_none_is_lost() {
        let wg = [Some(0), Some(0), Some(1), Some(2), NONE, NONE];
        let targets = [Target::End, Target::Into(0), Target::Into(1), Target::Into(2), near(0, false), near(0, true), near(2, true), near(4, false), near(5, true)];
        for w in 0..wg.len() {
            for &t in &targets {
                let Some(a) = arrange(&wg, 3, Grab::Win(w), t) else { continue };
                let mut seen = a.order.clone();
                seen.sort_unstable();
                assert_eq!(seen, [0, 1, 2, 3, 4, 5], "{w} {t:?}");
                // Sorted: the group numbers never go down, those in none are last.
                let key = |g: &Option<usize>| g.unwrap_or(usize::MAX);
                assert!(a.group.windows(2).all(|p| key(&p[0]) <= key(&p[1])), "{w} {t:?}: {:?}", a.group);
                // No gaps in the numbering: every group is used.
                for g in 0..a.groups.len() {
                    assert!(a.group.contains(&Some(g)), "{w} {t:?}: group {g} is empty");
                }
            }
        }
        for g in 0..3 {
            for h in 0..3 {
                for after in [false, true] {
                    let Some(a) = arrange(&wg, 3, Grab::Group(g), Target::Beside { group: h, after }) else { continue };
                    let mut groups = a.groups.clone();
                    groups.sort_unstable();
                    assert_eq!(groups, [0, 1, 2]);
                    assert!(a.group.windows(2).all(|p| p[0].unwrap_or(usize::MAX) <= p[1].unwrap_or(usize::MAX)));
                }
            }
        }
    }
}
