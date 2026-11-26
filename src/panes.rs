//! Split panes of a window: pure layout, after the reference's `Tab` (`terminal (2).html`, CSS `.panes`).
//!
//! A window with one pane has no frame at all: the pane is the whole workspace, as before panes
//! existed. With more, they sit in a grid with a 6 px gap and 6 px of padding around: `Cols` is one row of
//! `n` columns, `Rows` one column of `n` rows, `Grid` has `ceil(sqrt(n))` columns and `ceil(n / cols)` rows
//! and the last pane spans the cells the last row leaves empty. Every rectangle is arithmetic on the
//! index and the sizes of the columns and rows (`rect`, `Tracks`): nothing else is stored, so a layout can never
//! disagree with the panes it lays out. The columns and rows share their axis evenly until the user resizes them
//! (a drag on the gap between two panes, or the keys: `drag`, `grow`, ADR 0045).
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

/// The most columns, and the most rows, a split keeps sizes for: one with more shares that axis evenly.
pub const MAX_TRACKS: usize = 16;

/// The columns or the rows of a split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Cols = 0,
    Rows = 1,
}

/// The sizes of a split's columns and rows (ADR 0045), kept by the group whose windows the split shows. A track's share
/// of its axis is its weight over the sum of the axis' weights; the weights are the tracks' lengths in pixels when they
/// were last set, so a split nobody resized since is laid out to the pixel, and a window resized afterwards keeps the
/// proportions. They hold for the number of tracks they were set for (`n`): an axis that has another count now (a window
/// joined or left the split) is shared evenly, never by sizes that were for other panes. All zero (`EVEN`, the default)
/// is the even split, pixel for pixel what the layout was before sizes existed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Tracks {
    /// How many columns and how many rows the weights are for (0: none set).
    pub n: [u8; 2],
    /// The weights of the columns, then of the rows.
    pub w: [[u16; MAX_TRACKS]; 2],
}

impl Tracks {
    /// Every axis shared evenly.
    pub const EVEN: Tracks = Tracks { n: [0; 2], w: [[0; MAX_TRACKS]; 2] };

    /// Whether axis `a` has sizes of its own for `count` tracks.
    pub fn sized(&self, a: Axis, count: usize) -> bool {
        (2..=MAX_TRACKS).contains(&count) && self.n[a as usize] as usize == count
    }

    /// The weights of axis `a` for `count` tracks: its own, or all the same.
    pub fn weights(&self, a: Axis, count: usize) -> &[u16] {
        const ONE: [u16; MAX_TRACKS] = [1; MAX_TRACKS];
        if self.sized(a, count) { &self.w[a as usize][..count] } else { &ONE[..count.min(MAX_TRACKS)] }
    }

    /// Where track `k` of `count` starts in `avail` pixels shared by weight, without the gaps: the weights before it
    /// over all of them, so track `count` starts at `avail` and the tracks fill the space exactly. Evenly shared, it is
    /// `k * avail / count`, the remainder spread.
    fn start(&self, a: Axis, k: usize, count: usize, avail: i32) -> i32 {
        if count > MAX_TRACKS || !self.sized(a, count) {
            return (k as i64 * avail as i64 / count.max(1) as i64) as i32;
        }
        let w = self.weights(a, count);
        let total: i64 = w.iter().map(|&x| x.max(1) as i64).sum();
        let before: i64 = w[..k].iter().map(|&x| x.max(1) as i64).sum();
        (avail as i64 * before / total) as i32
    }

    /// Gives axis `a` the lengths `lens`, in pixels (one per track).
    fn set(&mut self, a: Axis, lens: &[i32]) {
        if lens.len() > MAX_TRACKS {
            return;
        }
        let a = a as usize;
        self.n[a] = lens.len() as u8;
        self.w[a] = [0; MAX_TRACKS];
        for (w, &l) in self.w[a].iter_mut().zip(lens) {
            *w = l.clamp(1, u16::MAX as i32) as u16;
        }
    }

    /// Shares axis `a` evenly again.
    pub fn even(&mut self, a: Axis) {
        self.n[a as usize] = 0;
        self.w[a as usize] = [0; MAX_TRACKS];
    }

    /// Whether some axis is not shared evenly.
    pub fn uneven(&self) -> bool {
        self.n != [0; 2]
    }

    /// One more track after track `k` of axis `a`, which had `count`: it takes half of track `k`, the others keep their
    /// size (a split halves the window it splits). An axis shared evenly stays so.
    pub fn split(&mut self, a: Axis, count: usize, k: usize) {
        if !self.sized(a, count) || count + 1 > MAX_TRACKS || k >= count {
            return;
        }
        let w = &mut self.w[a as usize];
        let half = w[k] / 2;
        w.copy_within(k + 1..count, k + 2);
        w[k + 1] = half.max(1);
        w[k] = (w[k] - half).max(1);
        self.n[a as usize] = (count + 1) as u8;
    }
}

/// Where axis `a` of a split of `cr` (columns, rows) starts in `area` (inside the padding), the pixels its tracks share
/// (the gaps left out), and how many tracks it has.
fn axis(a: Axis, (c, r): (usize, usize), area: Rect) -> (i32, i32, usize) {
    let inner = area.inset(PAD);
    match a {
        Axis::Cols => (inner.x, inner.w - (c as i32 - 1) * GAP, c),
        Axis::Rows => (inner.y, inner.h - (r as i32 - 1) * GAP, r),
    }
}

/// Where tracks `k0..k1` of axis `a` begin and end together, in window pixels (the gaps between them included).
fn bounds(t: &Tracks, a: Axis, cr: (usize, usize), area: Rect, k0: usize, k1: usize) -> (i32, i32) {
    let (o, avail, count) = axis(a, cr, area);
    (o + k0 as i32 * GAP + t.start(a, k0, count, avail), o + (k1 as i32 - 1) * GAP + t.start(a, k1, count, avail))
}

/// The lengths of the `count` tracks of axis `a` (they add up to the pixels the axis shares), in `out`.
fn lengths(t: &Tracks, a: Axis, count: usize, avail: i32, out: &mut [i32; MAX_TRACKS]) {
    for (k, l) in out.iter_mut().enumerate().take(count) {
        *l = t.start(a, k + 1, count, avail) - t.start(a, k, count, avail);
    }
}

/// The rectangle of pane `i` of `n` in the workspace `area`, its columns and rows sized by `t`. One pane is the
/// whole `area`.
pub fn rect(kind: Kind, n: usize, i: usize, area: Rect, t: &Tracks) -> Rect {
    if n <= 1 || kind == Kind::Single {
        return area;
    }
    let cr = dims(kind, n);
    let (col, row) = (i % cr.0, i / cr.0);
    let (x0, x1) = bounds(t, Axis::Cols, cr, area, col, col + span(kind, n, i));
    let (y0, y1) = bounds(t, Axis::Rows, cr, area, row, row + 1);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// The pane under window pixel `(x, y)`, if any: the gaps and the padding belong to none.
pub fn at(kind: Kind, n: usize, area: Rect, t: &Tracks, x: f32, y: f32) -> Option<usize> {
    (0..n).find(|&i| rect(kind, n, i, area, t).contains(x, y))
}

/// A boundary of a split: the gap after track `k` of an axis, between it and track `k + 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub axis: Axis,
    pub k: usize,
}

/// The boundary whose gap is under window pixel `(x, y)`: a gap between two panes, not the padding round them, not
/// a pane (where the last pane of a grid spans a column boundary, that part is the pane's). Where a column gap crosses
/// a row gap, the column's.
pub fn edge_at(kind: Kind, n: usize, area: Rect, t: &Tracks, x: f32, y: f32) -> Option<Edge> {
    if n <= 1 || kind == Kind::Single || !area.inset(PAD).contains(x, y) || at(kind, n, area, t, x, y).is_some() {
        return None;
    }
    let cr = dims(kind, n);
    [Axis::Cols, Axis::Rows].into_iter().find_map(|a| {
        let (o, avail, count) = axis(a, cr, area);
        let p = if a == Axis::Cols { x } else { y };
        (0..count.saturating_sub(1)).find_map(|k| {
            let end = o + k as i32 * GAP + t.start(a, k + 1, count, avail);
            (p >= end as f32 && p < (end + GAP) as f32).then_some(Edge { axis: a, k })
        })
    })
}

/// Where the gap of boundary `e` starts along its axis, in window pixels (the first pixel after track `e.k`).
/// None when the split has no such boundary (any more: a window left it).
pub fn edge_pos(kind: Kind, n: usize, area: Rect, t: &Tracks, e: Edge) -> Option<i32> {
    let cr = dims(kind, n);
    let count = if e.axis == Axis::Cols { cr.0 } else { cr.1 };
    (n > 1 && kind != Kind::Single && e.k + 1 < count).then(|| bounds(t, e.axis, cr, area, e.k, e.k + 1).1)
}

/// The line boundary `e` is drawn as while the pointer is on it or drags it: 2 px down the middle of its gap, as long
/// as the panes it lies between (a column boundary that the last pane of a grid spans stops above that pane). None
/// when the split has no such boundary.
pub fn edge_line(kind: Kind, n: usize, area: Rect, t: &Tracks, e: Edge) -> Option<Rect> {
    let inner = area.inset(PAD);
    let cr = dims(kind, n);
    let mid = edge_pos(kind, n, area, t, e)? + GAP / 2 - 1;
    Some(match e.axis {
        Axis::Rows => Rect::new(inner.x, mid, inner.w, 2),
        Axis::Cols => {
            let (c, r) = cr;
            let first = (n - 1) % c;
            let covered = kind == Kind::Grid && span(kind, n, n - 1) > 1 && e.k >= first && r > 1;
            let bottom = if covered { bounds(t, Axis::Rows, cr, area, r - 2, r - 1).1 } else { inner.y2() };
            Rect::new(mid, inner.y, 2, bottom - inner.y)
        }
    })
}

/// Moves boundary `e` so that its gap starts at window pixel `to` along its axis, as far as the tracks either side
/// keep `min` pixels each (or what they have, when that is less already). The others keep their size. Returns whether
/// a size changed.
pub fn drag(t: &mut Tracks, kind: Kind, n: usize, area: Rect, e: Edge, to: i32, min: i32) -> bool {
    let cr = dims(kind, n);
    let (o, avail, count) = axis(e.axis, cr, area);
    if n <= 1 || kind == Kind::Single || e.k + 1 >= count || count > MAX_TRACKS {
        return false;
    }
    let mut lens = [0; MAX_TRACKS];
    lengths(t, e.axis, count, avail, &mut lens);
    let (a, b) = (lens[e.k], lens[e.k + 1]);
    let (lo, hi) = (min.min(a), a + b - min.min(b));
    let start = o + e.k as i32 * GAP + t.start(e.axis, e.k, count, avail);
    let a2 = (to - start).clamp(lo, hi.max(lo));
    if a2 == a {
        return false;
    }
    (lens[e.k], lens[e.k + 1]) = (a2, a + b - a2);
    t.set(e.axis, &lens[..count]);
    true
}

/// The step of `grow`: a twentieth of what the axis shares.
pub const STEP_DIV: i32 = 20;

/// Makes pane `i` longer along axis `a` by a step (`dir` 1) or shorter (`dir` -1): its tracks take the difference from
/// the others, or give it to them, in proportion to what each has above `min` pixels. Nothing goes below `min` that
/// was not below it already. Returns whether a size changed (not when the axis has one track, or nothing can give).
#[allow(clippy::too_many_arguments)]
pub fn grow(t: &mut Tracks, kind: Kind, n: usize, i: usize, area: Rect, a: Axis, dir: i32, min: i32) -> bool {
    if n <= 1 || kind == Kind::Single {
        return false;
    }
    let cr = dims(kind, n);
    let (_, avail, count) = axis(a, cr, area);
    if !(2..=MAX_TRACKS).contains(&count) || i >= n {
        return false;
    }
    let (k0, k1) = match a {
        Axis::Cols => (i % cr.0, i % cr.0 + span(kind, n, i)),
        Axis::Rows => (i / cr.0, i / cr.0 + 1),
    };
    if k1 - k0 >= count {
        return false; // the pane spans the whole axis: nothing beside it to take from
    }
    let mut lens = [0; MAX_TRACKS];
    lengths(t, a, count, avail, &mut lens);
    let floor = |l: i32| l.min(min);
    let mine: i32 = lens[k0..k1].iter().sum();
    let rest = avail - mine;
    let (mine_floor, rest_floor): (i32, i32) = lens[..count].iter().enumerate().fold((0, 0), |(m, r), (k, &l)| if (k0..k1).contains(&k) { (m + floor(l), r) } else { (m, r + floor(l)) });
    let step = (avail / STEP_DIV).max(1) * dir.signum();
    let target = (mine + step).clamp(mine_floor, (avail - rest_floor).max(mine_floor));
    if target == mine || mine <= 0 {
        return false;
    }
    // The other tracks share what is left above their floors in proportion to what they had above them (evenly when
    // none had any); the pane's own tracks (more than one when it spans) keep their proportions.
    let (spare, spare2) = (rest - rest_floor, avail - target - rest_floor);
    let others = (count - (k1 - k0)) as i64;
    let mut new = [0; MAX_TRACKS];
    for k in 0..count {
        new[k] = if (k0..k1).contains(&k) {
            (lens[k] as i64 * target as i64 / mine as i64) as i32
        } else if spare > 0 {
            floor(lens[k]) + ((lens[k] - floor(lens[k])) as i64 * spare2 as i64 / spare as i64) as i32
        } else {
            floor(lens[k]) + (spare2 as i64 / others) as i32
        };
    }
    // What the divisions rounded away goes to the pane itself, so the tracks fill the axis exactly.
    let short = avail - new[..count].iter().sum::<i32>();
    new[k1 - 1] += short;
    t.set(a, &new[..count]);
    true
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

/// The layout `n` panes have when what holds them (a group, ADR 0034) only remembers the last choice: one is
/// `Single`, and two or more never are (two columns, then the automatic grid), whatever was remembered.
pub fn fit_kind(kind: Kind, n: usize) -> Kind {
    match (kind, n) {
        (_, 0 | 1) => Kind::Single,
        (Kind::Single, 2) => Kind::Cols,
        (Kind::Single, _) => Kind::Grid,
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
pub fn fits(kind: Kind, n: usize, area: Rect, t: &Tracks, min_w: i32, min_h: i32) -> bool {
    (0..n).all(|i| {
        let r = rect(kind, n, i, area, t);
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

    const EVEN: Tracks = Tracks::EVEN;

    const AREA: Rect = Rect::new(220, 42, 680, 550);

    fn all(kind: Kind, n: usize) -> Vec<Rect> {
        (0..n).map(|i| rect(kind, n, i, AREA, &EVEN)).collect()
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.x2() && b.x < a.x2() && a.y < b.y2() && b.y < a.y2()
    }

    #[test]
    fn a_remembered_layout_always_fits_the_count() {
        assert_eq!(fit_kind(Kind::Grid, 1), Kind::Single);
        assert_eq!(fit_kind(Kind::Cols, 0), Kind::Single);
        assert_eq!(fit_kind(Kind::Single, 2), Kind::Cols);
        assert_eq!(fit_kind(Kind::Single, 3), Kind::Grid);
        assert_eq!(fit_kind(Kind::Single, 9), Kind::Grid);
        // A real choice is kept.
        assert_eq!(fit_kind(Kind::Rows, 4), Kind::Rows);
        assert_eq!(fit_kind(Kind::Cols, 3), Kind::Cols);
        // Whatever it returns for two or more panes can be laid out without overlap.
        for n in 2..=8 {
            let rs: Vec<Rect> = (0..n).map(|i| rect(fit_kind(Kind::Single, n), n, i, AREA, &EVEN)).collect();
            assert!(rs.iter().enumerate().all(|(i, a)| rs.iter().skip(i + 1).all(|b| !overlap(*a, *b))), "n = {n}");
        }
    }

    #[test]
    fn one_pane_is_the_whole_workspace_with_no_frame() {
        assert_eq!(rect(Kind::Single, 1, 0, AREA, &EVEN), AREA);
        // Even a stale layout kind cannot frame a single pane.
        assert_eq!(rect(Kind::Grid, 1, 0, AREA, &EVEN), AREA);
        assert_eq!(rect(Kind::Cols, 1, 0, AREA, &EVEN), AREA);
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
            assert_eq!(at(Kind::Grid, 5, AREA, &EVEN, r.x as f32 + 1.0, r.y as f32 + 1.0), Some(i));
            assert_eq!(at(Kind::Grid, 5, AREA, &EVEN, r.x2() as f32 - 0.5, r.y2() as f32 - 0.5), Some(i));
        }
        // The gap between two columns, the padding, and outside.
        let gap_x = rs[0].x2() as f32 + 2.0;
        assert_eq!(at(Kind::Grid, 5, AREA, &EVEN, gap_x, rs[0].y as f32 + 5.0), None);
        assert_eq!(at(Kind::Grid, 5, AREA, &EVEN, AREA.x as f32 + 2.0, AREA.y as f32 + 2.0), None);
        assert_eq!(at(Kind::Grid, 5, AREA, &EVEN, 0.0, 0.0), None);
        // One pane: the whole workspace, padding included.
        assert_eq!(at(Kind::Single, 1, AREA, &EVEN, AREA.x as f32 + 1.0, AREA.y as f32 + 1.0), Some(0));
    }

    #[test]
    fn a_split_is_refused_when_a_pane_would_be_too_small() {
        let (min_w, min_h) = (200, 100);
        // 680 px: three columns of ~218 fit, four of ~162 do not.
        assert!(fits(Kind::Cols, 3, AREA, &EVEN, min_w, min_h));
        assert!(!fits(Kind::Cols, 4, AREA, &EVEN, min_w, min_h));
        // 550 px: five rows of ~100 do not fit with the gaps, four do.
        assert!(fits(Kind::Rows, 4, AREA, &EVEN, min_w, min_h));
        assert!(!fits(Kind::Rows, 6, AREA, &EVEN, min_w, min_h));
        // The grid: nine panes (3 x 3) fit, ten (4 x 3) do not.
        assert!(fits(Kind::Grid, 9, AREA, &EVEN, min_w, min_h));
        assert!(!fits(Kind::Grid, 10, AREA, &EVEN, min_w, min_h));
    }

    #[test]
    fn a_pane_of_exactly_the_minimum_is_allowed() {
        // The smallest pane of each layout is the limit: it fits at its own size and not a pixel more either way.
        for (kind, n) in [(Kind::Cols, 3), (Kind::Rows, 4), (Kind::Grid, 5), (Kind::Grid, 9)] {
            let min_w = (0..n).map(|i| rect(kind, n, i, AREA, &EVEN).w).min().unwrap();
            let min_h = (0..n).map(|i| rect(kind, n, i, AREA, &EVEN).h).min().unwrap();
            assert!(fits(kind, n, AREA, &EVEN, min_w, min_h), "{kind:?} {n}: exactly the smallest");
            assert!(!fits(kind, n, AREA, &EVEN, min_w + 1, min_h), "{kind:?} {n}: one pixel wider");
            assert!(!fits(kind, n, AREA, &EVEN, min_w, min_h + 1), "{kind:?} {n}: one pixel taller");
        }
    }

    #[test]
    fn the_cross_sits_at_the_top_right_inside_the_pane() {
        let r = rect(Kind::Cols, 2, 1, AREA, &EVEN);
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

    /// The panes of `kind` x `n` sized by `t`.
    fn sized(kind: Kind, n: usize, t: &Tracks) -> Vec<Rect> {
        (0..n).map(|i| rect(kind, n, i, AREA, t)).collect()
    }

    /// A split's panes stay inside the padding, apart, and with the gaps fill it exactly.
    fn sound(kind: Kind, n: usize, t: &Tracks) {
        let rs = sized(kind, n, t);
        let inner = AREA.inset(PAD);
        for (i, r) in rs.iter().enumerate() {
            assert!(r.w > 0 && r.h > 0, "{kind:?} {n} #{i}: {r:?}");
            assert!(r.x >= inner.x && r.y >= inner.y && r.x2() <= inner.x2() && r.y2() <= inner.y2(), "{kind:?} {n} #{i}: {r:?}");
            for (j, s) in rs.iter().enumerate().skip(i + 1) {
                assert!(!overlap(*r, *s), "{kind:?} {n}: #{i} {r:?} overlaps #{j} {s:?}");
            }
        }
        let area: i64 = rs.iter().map(|r| r.w as i64 * r.h as i64).sum();
        let (c, r) = dims(kind, n);
        let (g, w, h) = (GAP as i64, inner.w as i64, inner.h as i64);
        let gaps = (c as i64 - 1) * g * h + (r as i64 - 1) * g * w - (c as i64 - 1) * (r as i64 - 1) * g * g;
        // The last pane of a grid covers the gaps between the cells it spans too.
        let spanned = (span(kind, n, n - 1) as i64 - 1) * g * rs[n - 1].h as i64;
        assert_eq!(area, w * h - gaps + spanned, "{kind:?} {n}: the panes and the gaps fill the padded area");
    }

    #[test]
    fn even_tracks_are_the_layout_there_was_before_sizes() {
        // `k * avail / count`, the formula before ADR 0045, for every layout and count.
        for kind in [Kind::Cols, Kind::Rows, Kind::Grid] {
            for n in 2..=17 {
                let (c, r) = dims(kind, n);
                let inner = AREA.inset(PAD);
                let (aw, ah) = (inner.w - (c as i32 - 1) * GAP, inner.h - (r as i32 - 1) * GAP);
                for i in 0..n {
                    let (col, row, s) = ((i % c) as i32, (i / c) as i32, span(kind, n, i) as i32);
                    let x0 = inner.x + col * GAP + col * aw / c as i32;
                    let x1 = inner.x + (col + s - 1) * GAP + (col + s) * aw / c as i32;
                    let y0 = inner.y + row * GAP + row * ah / r as i32;
                    let y1 = inner.y + row * GAP + (row + 1) * ah / r as i32;
                    assert_eq!(rect(kind, n, i, AREA, &EVEN), Rect::new(x0, y0, x1 - x0, y1 - y0), "{kind:?} {n} #{i}");
                }
                sound(kind, n, &EVEN);
            }
        }
    }

    #[test]
    fn dragging_a_boundary_moves_it_to_the_pixel_and_leaves_the_other_tracks_alone() {
        let mut t = EVEN;
        let before = sized(Kind::Cols, 3, &t);
        let e = Edge { axis: Axis::Cols, k: 0 };
        let to = before[0].x2() + 80;
        assert!(drag(&mut t, Kind::Cols, 3, AREA, e, to, 50));
        let after = sized(Kind::Cols, 3, &t);
        assert_eq!(after[0].x2(), to, "the gap starts where it was dragged");
        assert_eq!(after[1].x2(), before[1].x2(), "the boundary after the next pane did not move");
        assert_eq!(after[2], before[2]);
        assert_eq!(edge_pos(Kind::Cols, 3, AREA, &t, e), Some(to));
        assert_eq!(edge_pos(Kind::Cols, 3, AREA, &t, Edge { axis: Axis::Cols, k: 2 }), None, "no boundary after the last");
        assert_eq!(edge_pos(Kind::Cols, 3, AREA, &t, Edge { axis: Axis::Rows, k: 0 }), None, "one row: no row boundary");
        sound(Kind::Cols, 3, &t);
        // Not past the minimum of either side, and a drag that changes nothing says so.
        assert!(drag(&mut t, Kind::Cols, 3, AREA, e, AREA.x2() + 500, 50));
        assert_eq!(sized(Kind::Cols, 3, &t)[1].w, 50);
        assert!(!drag(&mut t, Kind::Cols, 3, AREA, e, AREA.x2() + 900, 50));
        assert!(drag(&mut t, Kind::Cols, 3, AREA, e, -1000, 50));
        assert_eq!(sized(Kind::Cols, 3, &t)[0].w, 50);
        // Rows, the same.
        let mut t = EVEN;
        let e = Edge { axis: Axis::Rows, k: 1 };
        let to = sized(Kind::Rows, 3, &t)[1].y2() - 40;
        assert!(drag(&mut t, Kind::Rows, 3, AREA, e, to, 60));
        assert_eq!(sized(Kind::Rows, 3, &t)[1].y2(), to);
        sound(Kind::Rows, 3, &t);
        // A boundary that does not exist moves nothing.
        assert!(!drag(&mut t, Kind::Rows, 3, AREA, Edge { axis: Axis::Rows, k: 2 }, 300, 60));
        assert!(!drag(&mut t, Kind::Rows, 3, AREA, Edge { axis: Axis::Cols, k: 0 }, 300, 60));
    }

    #[test]
    fn a_track_already_under_the_minimum_does_not_shrink() {
        // Three columns of ~218 px with a minimum of 300: neither side of the boundary may lose a pixel.
        let mut t = EVEN;
        let e = Edge { axis: Axis::Cols, k: 0 };
        let rs = sized(Kind::Cols, 3, &t);
        assert!(!drag(&mut t, Kind::Cols, 3, AREA, e, rs[0].x2() - 30, 300), "the first would shrink");
        assert!(!drag(&mut t, Kind::Cols, 3, AREA, e, rs[0].x2() + 30, 300), "the second would shrink");
        assert_eq!(t, EVEN);
    }

    #[test]
    fn sizes_are_proportions_when_the_window_changes_size() {
        let mut t = EVEN;
        let rs = sized(Kind::Cols, 2, &t);
        assert!(drag(&mut t, Kind::Cols, 2, AREA, Edge { axis: Axis::Cols, k: 0 }, rs[0].x + (rs[0].w + rs[1].w) / 4, 20));
        let wide = Rect::new(0, 0, AREA.w * 2 + 2 * PAD, AREA.h);
        let (a, b) = (rect(Kind::Cols, 2, 0, AREA, &t).w, rect(Kind::Cols, 2, 1, AREA, &t).w);
        let (a2, b2) = (rect(Kind::Cols, 2, 0, wide, &t).w, rect(Kind::Cols, 2, 1, wide, &t).w);
        assert!((a2 as f32 / (a2 + b2) as f32 - a as f32 / (a + b) as f32).abs() < 0.01, "{a}:{b} became {a2}:{b2}");
        assert!(a * 2 < b, "a quarter and three quarters");
    }

    #[test]
    fn another_count_of_tracks_is_shared_evenly() {
        let mut t = EVEN;
        assert!(drag(&mut t, Kind::Cols, 2, AREA, Edge { axis: Axis::Cols, k: 0 }, AREA.x + 150, 20));
        assert!(t.sized(Axis::Cols, 2) && !t.sized(Axis::Cols, 3));
        // A third column (a window joined): even, as a split nobody resized.
        assert_eq!(sized(Kind::Cols, 3, &t), sized(Kind::Cols, 3, &EVEN));
        // The grid of three has the same two columns: it keeps them, and its rows are even.
        let (g, even) = (sized(Kind::Grid, 3, &t), sized(Kind::Grid, 3, &EVEN));
        assert_eq!(g[0].x2(), AREA.x + 150);
        assert_eq!((g[0].y2(), g[2].h), (even[0].y2(), even[2].h));
        // Evening an axis forgets its sizes.
        assert!(t.uneven());
        t.even(Axis::Cols);
        assert_eq!(t, EVEN);
        assert!(!t.uneven());
    }

    #[test]
    fn the_last_pane_of_a_grid_follows_the_columns_it_spans() {
        let mut t = EVEN;
        // Five in three columns: move both column boundaries and the row boundary.
        let base = sized(Kind::Grid, 5, &t);
        assert!(drag(&mut t, Kind::Grid, 5, AREA, Edge { axis: Axis::Cols, k: 0 }, base[0].x2() + 60, 40));
        assert!(drag(&mut t, Kind::Grid, 5, AREA, Edge { axis: Axis::Cols, k: 1 }, base[1].x2() - 20, 40));
        assert!(drag(&mut t, Kind::Grid, 5, AREA, Edge { axis: Axis::Rows, k: 0 }, base[0].y2() + 70, 40));
        let rs = sized(Kind::Grid, 5, &t);
        sound(Kind::Grid, 5, &t);
        assert_eq!(rs[4].x, rs[1].x, "it starts at the second column");
        assert_eq!(rs[4].x2(), rs[2].x2(), "and ends with the third");
        assert_eq!(rs[3].x2(), rs[0].x2(), "the pane under the first column is as wide as it");
        assert_eq!(rs[0].y2(), base[0].y2() + 70);
    }

    #[test]
    fn the_gap_under_the_pointer_is_its_boundary_and_a_spanned_one_is_the_panes() {
        let t = EVEN;
        let rs = sized(Kind::Grid, 5, &t);
        let (col0, col1, row0) = ((rs[0].x2() + 2) as f32, (rs[1].x2() + 2) as f32, (rs[0].y2() + 2) as f32);
        let c0 = Some(Edge { axis: Axis::Cols, k: 0 });
        // Between the first two columns, in either row.
        assert_eq!(edge_at(Kind::Grid, 5, AREA, &t, col0, rs[0].y as f32 + 9.0), c0);
        assert_eq!(edge_at(Kind::Grid, 5, AREA, &t, col0, rs[3].y as f32 + 9.0), c0);
        // Between the second and third: in the first row; in the second it is inside the last pane.
        assert_eq!(edge_at(Kind::Grid, 5, AREA, &t, col1, rs[1].y as f32 + 9.0), Some(Edge { axis: Axis::Cols, k: 1 }));
        assert_eq!(edge_at(Kind::Grid, 5, AREA, &t, col1, rs[4].y as f32 + 9.0), None);
        // The row gap; at a crossing, the column's.
        assert_eq!(edge_at(Kind::Grid, 5, AREA, &t, rs[2].x as f32 + 9.0, row0), Some(Edge { axis: Axis::Rows, k: 0 }));
        assert_eq!(edge_at(Kind::Grid, 5, AREA, &t, col0, row0), c0);
        // A pane, the padding, one pane.
        assert_eq!(edge_at(Kind::Grid, 5, AREA, &t, rs[0].x as f32 + 9.0, rs[0].y as f32 + 9.0), None);
        assert_eq!(edge_at(Kind::Grid, 5, AREA, &t, AREA.x as f32 + 2.0, rs[0].y as f32 + 9.0), None);
        assert_eq!(edge_at(Kind::Single, 1, AREA, &t, col0, row0), None);
        // Its line: down the middle of the gap, as long as the panes either side.
        let l = edge_line(Kind::Grid, 5, AREA, &t, Edge { axis: Axis::Cols, k: 0 }).unwrap();
        assert_eq!((l.x, l.w, l.y, l.y2()), (rs[0].x2() + GAP / 2 - 1, 2, rs[0].y, rs[3].y2()));
        let l = edge_line(Kind::Grid, 5, AREA, &t, Edge { axis: Axis::Cols, k: 1 }).unwrap();
        assert_eq!((l.y, l.y2()), (rs[1].y, rs[1].y2()), "it stops above the pane that spans it");
        let l = edge_line(Kind::Grid, 5, AREA, &t, Edge { axis: Axis::Rows, k: 0 }).unwrap();
        assert_eq!((l.y, l.h, l.x, l.x2()), (rs[0].y2() + GAP / 2 - 1, 2, rs[0].x, rs[2].x2()));
    }

    #[test]
    fn growing_takes_from_the_others_in_proportion_and_never_below_the_minimum() {
        let mut t = EVEN;
        let min = 60;
        let before = sized(Kind::Cols, 3, &t);
        assert!(grow(&mut t, Kind::Cols, 3, 1, AREA, Axis::Cols, 1, min));
        let after = sized(Kind::Cols, 3, &t);
        let step = (before.iter().map(|r| r.w).sum::<i32>() / STEP_DIV).max(1);
        assert!((after[1].w - (before[1].w + step)).abs() <= 1, "a step wider (and the pixel the others' shares rounded away)");
        assert!((after[0].w - after[2].w).abs() <= 1, "the two others gave the same");
        sound(Kind::Cols, 3, &t);
        // Over and over: it stops when the others are at the minimum.
        for _ in 0..40 {
            grow(&mut t, Kind::Cols, 3, 1, AREA, Axis::Cols, 1, min);
        }
        let rs = sized(Kind::Cols, 3, &t);
        assert_eq!((rs[0].w, rs[2].w), (min, min));
        assert!(!grow(&mut t, Kind::Cols, 3, 1, AREA, Axis::Cols, 1, min));
        sound(Kind::Cols, 3, &t);
        // Shrinking gives back to the others, and stops at the minimum too.
        for _ in 0..40 {
            grow(&mut t, Kind::Cols, 3, 1, AREA, Axis::Cols, -1, min);
        }
        assert_eq!(sized(Kind::Cols, 3, &t)[1].w, min);
        sound(Kind::Cols, 3, &t);
        // An axis with one track, or a pane that spans all of it, has nothing to take from.
        assert!(!grow(&mut t, Kind::Cols, 3, 1, AREA, Axis::Rows, 1, min));
        let mut g = EVEN;
        assert!(!grow(&mut g, Kind::Grid, 3, 2, AREA, Axis::Cols, 1, min), "the last of three spans both columns");
        assert!(grow(&mut g, Kind::Grid, 3, 2, AREA, Axis::Rows, 1, min));
        sound(Kind::Grid, 3, &g);
    }

    #[test]
    fn a_split_of_a_resized_axis_halves_the_window_it_splits() {
        let mut t = EVEN;
        assert!(drag(&mut t, Kind::Cols, 2, AREA, Edge { axis: Axis::Cols, k: 0 }, AREA.x + 400, 20));
        let w = t.w[0];
        t.split(Axis::Cols, 2, 0);
        assert!(t.sized(Axis::Cols, 3));
        assert_eq!(t.w[0][0] + t.w[0][1], w[0], "the first column is halved");
        assert_eq!(t.w[0][2], w[1], "the second keeps its size");
        sound(Kind::Cols, 3, &t);
        // An even axis stays even.
        let mut e = EVEN;
        e.split(Axis::Cols, 2, 0);
        assert_eq!(e, EVEN);
    }

    #[test]
    fn many_drags_and_steps_keep_every_layout_sound() {
        // A walk of drags and steps on every layout and count: never an overlap, a hole or a pane outside the padding.
        let mut seed = 7u32;
        let mut next = |m: i32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) as i32 % m.max(1)
        };
        for kind in [Kind::Cols, Kind::Rows, Kind::Grid] {
            for n in 2..=10 {
                let mut t = EVEN;
                let (c, r) = dims(kind, n);
                for _ in 0..30 {
                    let a = if next(2) == 0 { Axis::Cols } else { Axis::Rows };
                    if next(2) == 0 {
                        let count = if a == Axis::Cols { c } else { r };
                        if count < 2 {
                            continue;
                        }
                        let e = Edge { axis: a, k: next(count as i32 - 1) as usize };
                        let (o, len) = if a == Axis::Cols { (AREA.x, AREA.w) } else { (AREA.y, AREA.h) };
                        drag(&mut t, kind, n, AREA, e, o + next(len), 24);
                    } else {
                        grow(&mut t, kind, n, next(n as i32) as usize, AREA, a, if next(2) == 0 { 1 } else { -1 }, 24);
                    }
                    sound(kind, n, &t);
                }
            }
        }
    }
}
