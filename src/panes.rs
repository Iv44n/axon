//! Split panes of a window: pure layout, after the reference's `Tab` (`terminal (2).html`, CSS `.panes`).
//!
//! A window with one pane has no frame at all: the pane is the whole workspace, as before panes
//! existed. With more, they sit in a grid with a 6 px gap and 6 px of padding around: `Cols` is one row of
//! `n` columns, `Rows` one column of `n` rows, `Grid` has `ceil(sqrt(n))` columns and `ceil(n / cols)` rows
//! and the last pane spans the cells the last row leaves empty. Every rectangle is arithmetic on the
//! index (`rect`): nothing is stored, so a layout can never disagree with the panes it lays out.
//!
//! Coordinates are window pixels. No dependencies.

/// A rectangle in window pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// The first column and row outside it.
    pub const fn x2(self) -> i32 {
        self.x + self.w
    }

    pub const fn y2(self) -> i32 {
        self.y + self.h
    }

    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x as f32 && px < self.x2() as f32 && py >= self.y as f32 && py < self.y2() as f32
    }

    /// Smaller by `d` on every side.
    pub const fn inset(self, d: i32) -> Rect {
        Rect { x: self.x + d, y: self.y + d, w: self.w - 2 * d, h: self.h - 2 * d }
    }
}

/// The space between panes and around them, in pixels (the reference's `gap` and `padding`).
pub const GAP: i32 = 6;
pub const PAD: i32 = 6;
/// The panes' corner radius and the cross at the top right of one: its size, its distance from the top and from
/// the right edge.
pub const RADIUS: f32 = 10.0;
pub const CLOSE: i32 = 20;
const CLOSE_TOP: i32 = 7;
const CLOSE_RIGHT: i32 = 9;
/// What a split may leave a pane: no fewer columns and rows than this (the reference has no limit and scrolls;
/// axon refuses the split instead, ADR 0024).
pub const MIN_COLS: usize = 20;
pub const MIN_ROWS: usize = 5;

/// How the panes of a window are arranged (the reference's `data-layout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    /// One pane, no frame.
    #[default]
    Single,
    /// One row of columns.
    Cols,
    /// One column of rows.
    Rows,
    /// The automatic grid.
    Grid,
}

/// Where a split goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// `Alt+\`: another column.
    Cols,
    /// `Alt+-`: another row.
    Rows,
    /// `Ctrl+Space`: to the right at first, then the grid.
    Auto,
}

/// Columns and rows of the track grid of `n` panes.
pub fn dims(kind: Kind, n: usize) -> (usize, usize) {
    let n = n.max(1);
    match kind {
        Kind::Single => (1, 1),
        Kind::Cols => (n, 1),
        Kind::Rows => (1, n),
        Kind::Grid => {
            // ceil(sqrt(n)) without floats.
            let cols = (1..=n).find(|c| c * c >= n).unwrap_or(1);
            (cols, n.div_ceil(cols))
        }
    }
}

/// How many columns pane `i` spans: only the last pane of a grid takes the cells the last row leaves empty.
pub fn span(kind: Kind, n: usize, i: usize) -> usize {
    if kind == Kind::Grid && n > 0 && i == n - 1 {
        let (c, r) = dims(kind, n);
        c * r - n + 1
    } else {
        1
    }
}

/// Where track `k` of `count` starts inside `avail` pixels shared equally (the remainder is spread, so the
/// tracks fill the space exactly), without the gaps.
fn track(k: i32, count: i32, avail: i32) -> i32 {
    k * avail / count
}

/// The rectangle of pane `i` of `n` in the workspace `area`. One pane is the whole `area`.
pub fn rect(kind: Kind, n: usize, i: usize, area: Rect) -> Rect {
    if n <= 1 || kind == Kind::Single {
        return area;
    }
    let (c, r) = dims(kind, n);
    let (col, row) = (i % c, i / c);
    let spans = span(kind, n, i);
    let inner = area.inset(PAD);
    let (aw, ah) = (inner.w - (c as i32 - 1) * GAP, inner.h - (r as i32 - 1) * GAP);
    let (c0, c1) = (col as i32, (col + spans) as i32);
    let (x0, x1) = (inner.x + c0 * GAP + track(c0, c as i32, aw), inner.x + (c1 - 1) * GAP + track(c1, c as i32, aw));
    let (r0, r1) = (row as i32, row as i32 + 1);
    let (y0, y1) = (inner.y + r0 * GAP + track(r0, r as i32, ah), inner.y + (r1 - 1) * GAP + track(r1, r as i32, ah));
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// The pane under window pixel `(x, y)`, if any: the gaps and the padding belong to none.
pub fn at(kind: Kind, n: usize, area: Rect, x: f32, y: f32) -> Option<usize> {
    (0..n).find(|&i| rect(kind, n, i, area).contains(x, y))
}

/// The layout after a split in direction `dir` of a window that had `n` panes (the reference's `split`):
/// the first split picks the direction; later, the same direction keeps the layout and a different one
/// makes the grid.
pub fn after_split(kind: Kind, n: usize, dir: Dir) -> Kind {
    if n <= 1 {
        return if dir == Dir::Rows { Kind::Rows } else { Kind::Cols };
    }
    match (kind, dir) {
        (Kind::Cols, Dir::Cols) | (Kind::Rows, Dir::Rows) | (Kind::Grid, Dir::Auto) => kind,
        _ => Kind::Grid,
    }
}

/// The layout once a pane has gone and `n` are left: one pane is `Single`; a grid of two is just two columns.
pub fn after_remove(kind: Kind, n: usize) -> Kind {
    match (kind, n) {
        (_, 0 | 1) => Kind::Single,
        (Kind::Grid, 2) => Kind::Cols,
        (k, _) => k,
    }
}

/// The pane `d` places (-1, +1) from `i` among `n`, wrapping round.
pub fn cycle(i: usize, n: usize, d: i32) -> usize {
    if n == 0 {
        return 0;
    }
    (i as i64 + d as i64).rem_euclid(n as i64) as usize
}

/// The pane that takes the focus when pane `i` closes: the one before it, or the first (the reference's
/// `panes[max(0, i - 1)]`, an index into the panes that are left).
pub fn focus_after_remove(i: usize) -> usize {
    i.saturating_sub(1)
}

/// Whether every pane of `n` in the layout `kind` is at least `min_w` x `min_h` pixels in `area`.
pub fn fits(kind: Kind, n: usize, area: Rect, min_w: i32, min_h: i32) -> bool {
    (0..n).all(|i| {
        let r = rect(kind, n, i, area);
        r.w >= min_w && r.h >= min_h
    })
}

/// The cross at the top right of a pane's box `r`.
pub fn close_box(r: Rect) -> Rect {
    Rect::new(r.x2() - CLOSE_RIGHT - CLOSE, r.y + CLOSE_TOP, CLOSE, CLOSE)
}

/// A pane's text grid: where column 0 of row 0 starts, and how many columns and rows fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Grid {
    pub x: i32,
    pub y: i32,
    pub cols: usize,
    pub rows: usize,
}

/// The grid of a pane in the box `r`, `(left, top, right, bottom)` pixels of margin, for cells `cw` x `ch`: at
/// least 2 x 2 whatever the room (a terminal cannot be smaller).
pub fn grid(r: Rect, (ml, mt, mr, mb): (i32, i32, i32, i32), cw: u32, ch: u32) -> Grid {
    let cols = ((r.w - ml - mr).max(0) as u32 / cw.max(1)).max(2) as usize;
    let rows = ((r.h - mt - mb).max(0) as u32 / ch.max(1)).max(2) as usize;
    Grid { x: r.x + ml, y: r.y + mt, cols, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(220, 42, 680, 550);

    fn all(kind: Kind, n: usize) -> Vec<Rect> {
        (0..n).map(|i| rect(kind, n, i, AREA)).collect()
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.x2() && b.x < a.x2() && a.y < b.y2() && b.y < a.y2()
    }

    #[test]
    fn one_pane_is_the_whole_workspace_with_no_frame() {
        assert_eq!(rect(Kind::Single, 1, 0, AREA), AREA);
        // Even a stale layout kind cannot frame a single pane.
        assert_eq!(rect(Kind::Grid, 1, 0, AREA), AREA);
        assert_eq!(rect(Kind::Cols, 1, 0, AREA), AREA);
    }

    #[test]
    fn the_automatic_grid_has_ceil_sqrt_columns_and_the_rows_that_follow() {
        let want = [(1, 1, 1), (2, 2, 1), (3, 2, 2), (4, 2, 2), (5, 3, 2), (6, 3, 2), (7, 3, 3), (9, 3, 3), (10, 4, 3), (12, 4, 3), (13, 4, 4), (17, 5, 4)];
        for (n, c, r) in want {
            let k = if n == 1 { Kind::Single } else { Kind::Grid };
            assert_eq!(dims(k, n), (c, r), "n = {n}");
        }
        assert_eq!(dims(Kind::Cols, 5), (5, 1));
        assert_eq!(dims(Kind::Rows, 5), (1, 5));
    }

    #[test]
    fn only_the_last_pane_of_a_grid_spans_the_empty_cells() {
        // 5 in 3 columns: one empty cell, so the last spans 2; 7 in 3: two empty, spans 3; full rows span 1.
        assert_eq!(span(Kind::Grid, 5, 4), 2);
        assert_eq!(span(Kind::Grid, 5, 3), 1);
        assert_eq!(span(Kind::Grid, 7, 6), 3);
        assert_eq!(span(Kind::Grid, 3, 2), 2);
        assert_eq!(span(Kind::Grid, 4, 3), 1);
        assert_eq!(span(Kind::Grid, 6, 5), 1);
        // 10 in 4 columns: two empty cells in the last row, so the last pane spans three.
        assert_eq!(span(Kind::Grid, 10, 9), 3);
        assert_eq!(span(Kind::Grid, 12, 11), 1);
        for n in 1..=12 {
            for kind in [Kind::Cols, Kind::Rows] {
                assert!((0..n).all(|i| span(kind, n, i) == 1), "{kind:?} {n}");
            }
        }
    }

    #[test]
    fn rectangles_never_overlap_and_stay_inside_the_padding_for_every_count_and_layout() {
        for kind in [Kind::Cols, Kind::Rows, Kind::Grid] {
            for n in 2..=12 {
                let rs = all(kind, n);
                let inner = AREA.inset(PAD);
                for (i, r) in rs.iter().enumerate() {
                    assert!(r.w > 0 && r.h > 0, "{kind:?} {n} #{i}: {r:?}");
                    assert!(r.x >= inner.x && r.y >= inner.y && r.x2() <= inner.x2() && r.y2() <= inner.y2(), "{kind:?} {n} #{i}: {r:?}");
                    for (j, s) in rs.iter().enumerate().skip(i + 1) {
                        assert!(!overlap(*r, *s), "{kind:?} {n}: #{i} {r:?} overlaps #{j} {s:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn panes_fill_the_padded_area_exactly_and_are_a_gap_apart() {
        let inner = AREA.inset(PAD);
        // Columns: the first starts at the padding, the last ends at the padding, neighbours are a gap apart.
        for n in 2..=12 {
            let rs = all(Kind::Cols, n);
            assert_eq!(rs[0].x, inner.x);
            assert_eq!(rs[n - 1].x2(), inner.x2());
            for w in rs.windows(2) {
                assert_eq!(w[1].x - w[0].x2(), GAP);
                assert_eq!((w[0].y, w[0].h), (inner.y, inner.h));
            }
            // The widths differ by at most one pixel (the remainder is spread).
            let (lo, hi) = rs.iter().fold((i32::MAX, 0), |(lo, hi), r| (lo.min(r.w), hi.max(r.w)));
            assert!(hi - lo <= 1, "n = {n}: {lo}..{hi}");
            let rs = all(Kind::Rows, n);
            assert_eq!(rs[0].y, inner.y);
            assert_eq!(rs[n - 1].y2(), inner.y2());
            assert!(rs.windows(2).all(|w| w[1].y - w[0].y2() == GAP && w[0].x == inner.x && w[0].w == inner.w));
        }
    }

    #[test]
    fn a_grid_of_five_is_two_rows_the_last_pane_filling_the_bottom_right() {
        let rs = all(Kind::Grid, 5);
        let inner = AREA.inset(PAD);
        // Row 0: three panes, row 1: one pane and the last, which spans two columns.
        assert_eq!(rs[0].y, rs[1].y);
        assert_eq!(rs[1].y, rs[2].y);
        assert_eq!(rs[3].y, rs[4].y);
        assert!(rs[3].y > rs[0].y2());
        assert_eq!(rs[3].x, rs[0].x);
        assert_eq!(rs[4].x2(), inner.x2());
        assert_eq!(rs[4].x - rs[3].x2(), GAP);
        // The spanning pane is as wide as the two columns under it and the gap between them.
        assert_eq!(rs[4].x, rs[1].x);
        assert_eq!(rs[4].w, rs[1].w + GAP + rs[2].w);
        // And the grid of three: two on top, the last as wide as both.
        let rs = all(Kind::Grid, 3);
        assert_eq!((rs[2].x, rs[2].x2()), (inner.x, inner.x2()));
    }

    #[test]
    fn a_split_picks_the_direction_first_and_the_grid_when_directions_mix() {
        // From one pane: the direction asked for, and Ctrl+Space (auto) goes to the right.
        assert_eq!(after_split(Kind::Single, 1, Dir::Cols), Kind::Cols);
        assert_eq!(after_split(Kind::Single, 1, Dir::Rows), Kind::Rows);
        assert_eq!(after_split(Kind::Single, 1, Dir::Auto), Kind::Cols);
        // The same direction keeps it; a different one gives the grid.
        assert_eq!(after_split(Kind::Cols, 2, Dir::Cols), Kind::Cols);
        assert_eq!(after_split(Kind::Rows, 3, Dir::Rows), Kind::Rows);
        assert_eq!(after_split(Kind::Cols, 2, Dir::Rows), Kind::Grid);
        assert_eq!(after_split(Kind::Rows, 2, Dir::Cols), Kind::Grid);
        assert_eq!(after_split(Kind::Cols, 2, Dir::Auto), Kind::Grid);
        assert_eq!(after_split(Kind::Rows, 2, Dir::Auto), Kind::Grid);
        assert_eq!(after_split(Kind::Grid, 4, Dir::Auto), Kind::Grid);
        assert_eq!(after_split(Kind::Grid, 4, Dir::Cols), Kind::Grid);
        assert_eq!(after_split(Kind::Grid, 4, Dir::Rows), Kind::Grid);
        // Ctrl+Space over and over: one to the right, then the grid for good.
        let (mut kind, mut n) = (Kind::Single, 1);
        let mut seen = vec![];
        for _ in 0..6 {
            kind = after_split(kind, n, Dir::Auto);
            n += 1;
            seen.push(kind);
        }
        assert_eq!(seen, [Kind::Cols, Kind::Grid, Kind::Grid, Kind::Grid, Kind::Grid, Kind::Grid]);
    }

    #[test]
    fn closing_leaves_single_for_one_pane_and_columns_for_a_grid_of_two() {
        assert_eq!(after_remove(Kind::Cols, 1), Kind::Single);
        assert_eq!(after_remove(Kind::Grid, 1), Kind::Single);
        assert_eq!(after_remove(Kind::Grid, 2), Kind::Cols);
        assert_eq!(after_remove(Kind::Grid, 3), Kind::Grid);
        assert_eq!(after_remove(Kind::Rows, 2), Kind::Rows);
        assert_eq!(after_remove(Kind::Cols, 4), Kind::Cols);
    }

    #[test]
    fn cycling_wraps_both_ways_and_focus_goes_to_the_pane_before_the_closed_one() {
        assert_eq!(cycle(0, 3, 1), 1);
        assert_eq!(cycle(2, 3, 1), 0);
        assert_eq!(cycle(0, 3, -1), 2);
        assert_eq!(cycle(1, 3, -1), 0);
        assert_eq!(cycle(0, 1, 1), 0);
        assert_eq!(cycle(0, 0, 1), 0);
        assert_eq!(focus_after_remove(0), 0);
        assert_eq!(focus_after_remove(1), 0);
        assert_eq!(focus_after_remove(4), 3);
    }

    #[test]
    fn the_pane_under_a_point_is_found_and_the_gaps_belong_to_none() {
        let rs = all(Kind::Grid, 5);
        for (i, r) in rs.iter().enumerate() {
            assert_eq!(at(Kind::Grid, 5, AREA, r.x as f32 + 1.0, r.y as f32 + 1.0), Some(i));
            assert_eq!(at(Kind::Grid, 5, AREA, r.x2() as f32 - 0.5, r.y2() as f32 - 0.5), Some(i));
        }
        // The gap between two columns, the padding, and outside.
        let gap_x = rs[0].x2() as f32 + 2.0;
        assert_eq!(at(Kind::Grid, 5, AREA, gap_x, rs[0].y as f32 + 5.0), None);
        assert_eq!(at(Kind::Grid, 5, AREA, AREA.x as f32 + 2.0, AREA.y as f32 + 2.0), None);
        assert_eq!(at(Kind::Grid, 5, AREA, 0.0, 0.0), None);
        // One pane: the whole workspace, padding included.
        assert_eq!(at(Kind::Single, 1, AREA, AREA.x as f32 + 1.0, AREA.y as f32 + 1.0), Some(0));
    }

    #[test]
    fn a_split_is_refused_when_a_pane_would_be_too_small() {
        let (min_w, min_h) = (200, 100);
        // 680 px: three columns of ~218 fit, four of ~162 do not.
        assert!(fits(Kind::Cols, 3, AREA, min_w, min_h));
        assert!(!fits(Kind::Cols, 4, AREA, min_w, min_h));
        // 550 px: five rows of ~100 do not fit with the gaps, four do.
        assert!(fits(Kind::Rows, 4, AREA, min_w, min_h));
        assert!(!fits(Kind::Rows, 6, AREA, min_w, min_h));
        // The grid: nine panes (3 x 3) fit, ten (4 x 3) do not.
        assert!(fits(Kind::Grid, 9, AREA, min_w, min_h));
        assert!(!fits(Kind::Grid, 10, AREA, min_w, min_h));
    }

    #[test]
    fn the_cross_sits_at_the_top_right_inside_the_pane() {
        let r = rect(Kind::Cols, 2, 1, AREA);
        let b = close_box(r);
        assert_eq!((b.w, b.h), (CLOSE, CLOSE));
        assert_eq!((b.y - r.y, r.x2() - b.x2()), (7, 9));
        assert!(b.x > r.x && b.x2() < r.x2());
    }

    #[test]
    fn the_grid_of_a_pane_follows_its_margins_and_is_never_smaller_than_two_by_two() {
        let r = Rect::new(100, 50, 400, 300);
        let g = grid(r, (11, 7, 11, 7), 9, 19);
        assert_eq!((g.x, g.y), (111, 57));
        assert_eq!((g.cols, g.rows), ((400 - 22) / 9, (300 - 14) / 19));
        assert_eq!(grid(Rect::new(0, 0, 10, 10), (11, 7, 11, 7), 9, 19), Grid { x: 11, y: 7, cols: 2, rows: 2 });
    }
}
