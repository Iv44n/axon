//! A filtered, scrolling list: the model of the command palette (and of every other list in the overlays:
//! the overview's results, the group dialog's windows). Entries with a label, a description, hidden
//! keywords, a shortcut, a group, a dot or an icon and the app's command id; a query that keeps and ranks
//! them the way the reference's palette does; a selection moved by the keyboard, the pointer and the wheel.
//!
//! Pure and tested without a window or a font. The entries live in one text pool with `(start, len)` spans
//! and a flat array of small items; what the query shows is an array of indices into it, best first. There is
//! no cache: `filter` rebuilds that array from the entries and the query, and row positions are summed from
//! the metrics when asked (a palette has a few dozen rows).
//!
//! **Ranking** (the reference's `renderPalette`): the text searched is `label + " " + description + " " +
//! keywords`, lower-cased with accents folded; a query found as a whole in it scores `100` less its position
//! (at most 60), one whose letters are found in order scores `20`, anything else is out; best first, ties in
//! the order the entries were given, at most 40 shown (70 when there is no query). **Highlighting** is ours:
//! the positions of the characters that matched, in the text above (`hits`), so a row can colour them.

use crate::{
    icons::{self, Icon},
    input::sym,
    overlay::{Cap, Ink, KBD_GAP, PX, PX_HEAD, PX_SMALL, Rect, Text, accent, dim, faint, fg, kbd, kbd_row},
    paint::{Paint, WHITE},
    side::group_color,
    xkb::Key,
};

/// The most rows shown for a query, and with none (the reference's `slice(0, 40)` and `slice(0, 70)`).
pub const MAX_QUERY: usize = 40;
pub const MAX_ALL: usize = 70;

/// A range of the text pool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Span {
    start: u32,
    len: u32,
}

/// What a row shows at its left: nothing, a colour dot (a group's colour, 0 to 7) or an icon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mark {
    #[default]
    None,
    Dot(u8),
    Icon(Icon),
}

/// What the app gives for one entry; the strings are copied into the list.
#[derive(Clone, Copy, Debug, Default)]
pub struct Entry<'a> {
    pub label: &'a str,
    /// A second line under the label (`pal-main small`).
    pub desc: &'a str,
    /// Words that find the entry without being shown.
    pub keywords: &'a str,
    /// A shortcut as `Alt+N`, drawn as key caps at the right.
    pub keys: &'a str,
    /// The heading it is listed under when there is no query.
    pub group: &'a str,
    pub mark: Mark,
    /// What the app runs when it is chosen.
    pub id: u32,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    label: Span,
    desc: Span,
    keywords: Span,
    keys: Span,
    group: u8,
    mark: Mark,
    id: u32,
}

/// Heights in pixels: the padding around the rows, a group heading, a row, a row with a description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub pad: i32,
    pub head: i32,
    pub item: i32,
    pub item2: i32,
}

/// The palette's: `.pal-list` pads 6, `.pal-group` 28, `.pal-item` 40 (24 px icon and 8 above and below), 51 with a description.
pub const PALETTE: Metrics = Metrics { pad: 6, head: 28, item: 40, item2: 51 };

/// A row of the list as laid out: a group heading or the `n`th shown entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Head(u8),
    Item(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub row: Row,
    /// From the top of the list's content (before scrolling).
    pub top: i32,
    pub h: i32,
}

/// What a key did to the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    /// Not a key of the list: the owner's.
    Ignored,
    Same,
    /// The selection moved (and the list scrolled to show it).
    Moved,
    /// Enter: the entry chosen, as the index it was given in (the order of `push`).
    Chose(usize),
}

pub struct List {
    pool: String,
    items: Vec<Item>,
    groups: Vec<Span>,
    /// The entries the query shows, best first: indices into `items`.
    shown: Vec<u32>,
    /// For each shown entry, its matched characters: a range of `hits`.
    hit_at: Vec<(u32, u32)>,
    hits: Vec<u16>,
    /// The query as typed (trimmed) and folded; scratch for the matching.
    query: String,
    q: Vec<char>,
    t: Vec<char>,
    scored: Vec<(i32, u32)>,
    sel: usize,
    scroll: i32,
    /// The height of the viewport the list is shown in.
    pub view: i32,
    pub metrics: Metrics,
    pub max_query: usize,
    pub max_all: usize,
}

/// A character as the search sees it: lower case, accents folded (the reference's NFD with the combining
/// marks removed), one character for one so that positions line up.
pub fn fold(c: char) -> char {
    let l = c.to_lowercase().next().unwrap_or(c);
    match l {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò'..='ö' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        c => c,
    }
}

/// How the query `q` ranks in the text `t` (both folded), and where it matched: the reference's score.
/// `hits` receives the matched positions in `t`, when asked for.
pub fn score(q: &[char], t: &[char], hits: Option<&mut Vec<u16>>) -> Option<i32> {
    if q.is_empty() {
        return None;
    }
    if let Some(i) = t.windows(q.len()).position(|w| w == q) {
        if let Some(h) = hits {
            h.extend((i..i + q.len()).map(|p| p as u16));
        }
        return Some(100 - i.min(60) as i32);
    }
    let mut at = 0;
    let mut found = Vec::new();
    for (i, c) in t.iter().enumerate() {
        if at < q.len() && *c == q[at] {
            found.push(i as u16);
            at += 1;
        }
    }
    if at < q.len() {
        return None;
    }
    if let Some(h) = hits {
        h.extend(found);
    }
    Some(20)
}

impl Default for List {
    fn default() -> List {
        List::new(PALETTE)
    }
}

impl List {
    pub fn new(metrics: Metrics) -> List {
        List {
            pool: String::new(),
            items: Vec::new(),
            groups: Vec::new(),
            shown: Vec::new(),
            hit_at: Vec::new(),
            hits: Vec::new(),
            query: String::new(),
            q: Vec::new(),
            t: Vec::new(),
            scored: Vec::new(),
            sel: 0,
            scroll: 0,
            view: 0,
            metrics,
            max_query: MAX_QUERY,
            max_all: MAX_ALL,
        }
    }

    fn span(&mut self, s: &str) -> Span {
        let start = self.pool.len() as u32;
        self.pool.push_str(s);
        Span { start, len: s.len() as u32 }
    }

    fn get(&self, s: Span) -> &str {
        &self.pool[s.start as usize..(s.start + s.len) as usize]
    }

    /// Adds an entry; call `filter` after the last one. Returns its index (what `Nav::Chose` reports).
    pub fn push(&mut self, e: Entry) -> usize {
        let group = match (0..self.groups.len()).find(|&g| self.get(self.groups[g]) == e.group) {
            Some(g) => g,
            None => {
                let s = self.span(e.group);
                self.groups.push(s);
                self.groups.len() - 1
            }
        };
        let item = Item { label: self.span(e.label), desc: self.span(e.desc), keywords: self.span(e.keywords), keys: self.span(e.keys), group: group as u8, mark: e.mark, id: e.id };
        self.items.push(item);
        self.items.len() - 1
    }

    /// How many entries there are (`len` is how many the query shows).
    #[cfg(test)]
    pub fn total(&self) -> usize {
        self.items.len()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.shown.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }

    /// The query as given to `filter`, trimmed.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The entry the `n`th shown row is: its index in the order of `push`.
    pub fn index(&self, n: usize) -> Option<usize> {
        self.shown.get(n).map(|&i| i as usize)
    }

    pub fn id(&self, entry: usize) -> u32 {
        self.items[entry].id
    }

    pub fn label(&self, entry: usize) -> &str {
        self.get(self.items[entry].label)
    }

    pub fn desc(&self, entry: usize) -> &str {
        self.get(self.items[entry].desc)
    }

    pub fn keys(&self, entry: usize) -> &str {
        self.get(self.items[entry].keys)
    }

    pub fn group(&self, entry: usize) -> &str {
        self.get(self.groups[self.items[entry].group as usize])
    }

    pub fn mark(&self, entry: usize) -> Mark {
        self.items[entry].mark
    }

    /// The matched characters of the `n`th shown row, as positions in `label + " " + description + " " +
    /// keywords` (so a position under the label's length is in the label); sorted, empty without a query.
    pub fn hits(&self, n: usize) -> &[u16] {
        self.hit_at.get(n).map_or(&[], |&(a, b)| &self.hits[a as usize..b as usize])
    }

    /// The `n`th shown row's matched characters of its label.
    pub fn label_hits(&self, n: usize) -> impl Iterator<Item = usize> + Clone + '_ {
        let len = self.index(n).map_or(0, |e| self.label(e).chars().count());
        self.hits(n).iter().map(|&p| p as usize).take_while(move |&p| p < len)
    }

    /// The `n`th shown row's matched characters of its description.
    pub fn desc_hits(&self, n: usize) -> impl Iterator<Item = usize> + Clone + '_ {
        let (from, len) = self.index(n).map_or((0, 0), |e| (self.label(e).chars().count() + 1, self.desc(e).chars().count()));
        self.hits(n).iter().map(|&p| p as usize).filter(move |&p| p >= from && p < from + len).map(move |p| p - from)
    }

    /// Shows the entries `query` keeps, best first (all of them, in order, when it is empty); the selection
    /// goes to the first and the list to the top.
    pub fn filter(&mut self, query: &str) {
        self.shown.clear();
        self.hit_at.clear();
        self.hits.clear();
        self.query.clear();
        self.query.push_str(query.trim());
        self.q.clear();
        self.q.extend(self.query.chars().map(fold));
        (self.sel, self.scroll) = (0, 0);
        if self.q.is_empty() {
            let n = self.items.len().min(self.max_all);
            self.shown.extend(0..n as u32);
            self.hit_at.resize(n, (0, 0));
            return;
        }
        let mut scored = std::mem::take(&mut self.scored);
        scored.clear();
        for i in 0..self.items.len() {
            self.searchable(i);
            if let Some(s) = score(&self.q, &self.t, None) {
                scored.push((s, i as u32));
            }
        }
        // Best first; a stable sort keeps the entries' own order among equals.
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored.truncate(self.max_query);
        for &(_, i) in &scored {
            self.searchable(i as usize);
            let from = self.hits.len() as u32;
            let mut hits = std::mem::take(&mut self.hits);
            score(&self.q, &self.t, Some(&mut hits));
            self.hits = hits;
            self.hit_at.push((from, self.hits.len() as u32));
            self.shown.push(i);
        }
        self.scored = scored;
    }

    /// Fills `t` with the searched text of entry `i`.
    fn searchable(&mut self, i: usize) {
        let it = self.items[i];
        let mut t = std::mem::take(&mut self.t);
        t.clear();
        let (l, d, k) = (self.get(it.label), self.get(it.desc), self.get(it.keywords));
        t.extend(l.chars().map(fold).chain([' ']).chain(d.chars().map(fold)).chain([' ']).chain(k.chars().map(fold)));
        self.t = t;
    }

    // ------------------------------------------------------------------ layout --

    fn row_h(&self, it: &Item) -> i32 {
        if it.desc.len > 0 { self.metrics.item2 } else { self.metrics.item }
    }

    /// The rows top to bottom: a heading before the first entry of each group when there is no query, then
    /// the entries. `top` counts from the content's top, after the padding.
    pub fn rows(&self) -> Rows<'_> {
        Rows { l: self, k: 0, head_done: false, y: self.metrics.pad }
    }

    /// The height of everything: the rows and the padding above and below.
    pub fn content_h(&self) -> i32 {
        self.rows().last().map_or(2 * self.metrics.pad, |r| r.top + r.h + self.metrics.pad)
    }

    fn max_scroll(&self) -> i32 {
        (self.content_h() - self.view).max(0)
    }

    pub fn scroll(&self) -> i32 {
        self.scroll
    }

    /// Whether the content is taller than the viewport (a scrollbar shows).
    #[cfg(test)]
    pub fn overflows(&self) -> bool {
        self.content_h() > self.view
    }

    /// The thumb of the scrollbar, `(top, height)` from the viewport's top, when there is one.
    #[cfg(test)]
    pub fn thumb(&self) -> Option<(i32, i32)> {
        self.thumb_in(self.view)
    }

    /// `thumb` for a viewport `view` high (painting uses the rectangle it is given, not the last `view` set).
    pub fn thumb_in(&self, view: i32) -> Option<(i32, i32)> {
        let content = self.content_h();
        if content <= view || view <= 0 {
            return None;
        }
        let h = (view as i64 * view as i64 / content as i64).max(20).min(view as i64) as i32;
        let travel = view - h;
        let scroll = self.scroll.min(content - view);
        let top = (scroll as i64 * travel as i64 / (content - view).max(1) as i64) as i32;
        Some((top, h))
    }

    // --------------------------------------------------------------- selection --

    /// The selected row's index among the shown ones.
    pub fn selected(&self) -> usize {
        self.sel
    }

    /// The entry selected, as its index in the order of `push`.
    pub fn chosen(&self) -> Option<usize> {
        self.index(self.sel)
    }

    fn place_of(&self, n: usize) -> Option<Placed> {
        self.rows().find(|r| r.row == Row::Item(n))
    }

    /// Scrolls the least that shows the selected row whole.
    fn reveal(&mut self) {
        // Before the first layout the viewport is not known: nothing to keep in view yet.
        let Some(r) = self.place_of(self.sel).filter(|_| self.view > 0) else { return };
        if self.sel == 0 {
            self.scroll = 0;
        } else if r.top < self.scroll {
            self.scroll = r.top;
        } else if r.top + r.h + self.metrics.pad > self.scroll + self.view && self.sel + 1 == self.shown.len() {
            self.scroll = r.top + r.h + self.metrics.pad - self.view;
        } else if r.top + r.h > self.scroll + self.view {
            self.scroll = r.top + r.h - self.view;
        }
        self.scroll = self.scroll.clamp(0, self.max_scroll());
    }

    pub fn select(&mut self, n: usize) -> bool {
        let n = n.min(self.shown.len().saturating_sub(1));
        let moved = n != self.sel;
        self.sel = n;
        self.reveal();
        moved
    }

    /// How many rows a Page key moves: what fits, less one.
    fn page(&self) -> usize {
        ((self.view / self.metrics.item.max(1)) as usize).saturating_sub(1).max(1)
    }

    /// Up and Down wrap round, as the reference's; Page keys, Home and End stop at the ends; Enter chooses.
    pub fn key(&mut self, k: &Key) -> Nav {
        let n = self.shown.len();
        let to = match k.sym {
            _ if k.mods.alt => return Nav::Ignored,
            sym::UP | sym::KP_UP if n > 0 => (self.sel + n - 1) % n,
            sym::DOWN | sym::KP_DOWN if n > 0 => (self.sel + 1) % n,
            sym::PAGE_UP | sym::KP_PAGE_UP if n > 0 => self.sel.saturating_sub(self.page()),
            sym::PAGE_DOWN | sym::KP_PAGE_DOWN if n > 0 => (self.sel + self.page()).min(n - 1),
            sym::HOME | sym::KP_HOME if n > 0 => 0,
            sym::END | sym::KP_END if n > 0 => n - 1,
            sym::RETURN | sym::KP_ENTER => return self.chosen().map_or(Nav::Same, Nav::Chose),
            sym::UP | sym::DOWN | sym::PAGE_UP | sym::PAGE_DOWN | sym::HOME | sym::END | sym::KP_UP | sym::KP_DOWN | sym::KP_PAGE_UP | sym::KP_PAGE_DOWN | sym::KP_HOME | sym::KP_END => return Nav::Same,
            _ => return Nav::Ignored,
        };
        if self.select(to) { Nav::Moved } else { Nav::Same }
    }

    /// The wheel: `dy` pixels, down positive. True if the list moved.
    pub fn scroll_by(&mut self, dy: f32) -> bool {
        let to = (self.scroll + dy.round() as i32).clamp(0, self.max_scroll());
        std::mem::replace(&mut self.scroll, to) != to
    }

    /// The shown row at `y` pixels below the viewport's top, if it is an entry.
    pub fn row_at(&self, y: i32) -> Option<usize> {
        let y = y + self.scroll;
        self.rows().find_map(|r| match r.row {
            Row::Item(n) if y >= r.top && y < r.top + r.h => Some(n),
            _ => None,
        })
    }

    /// The pointer is at `y` below the viewport's top: the row under it becomes the selection (the list does
    /// not scroll). True if the selection changed.
    pub fn hover(&mut self, y: i32) -> bool {
        match self.row_at(y) {
            Some(n) if n != self.sel => {
                self.sel = n;
                true
            }
            _ => false,
        }
    }
}

/// The rows of a list, see `List::rows`.
pub struct Rows<'a> {
    l: &'a List,
    k: usize,
    head_done: bool,
    y: i32,
}

impl Iterator for Rows<'_> {
    type Item = Placed;

    fn next(&mut self) -> Option<Placed> {
        let l = self.l;
        let it = l.items.get(*l.shown.get(self.k)? as usize)?;
        let new_group = self.k == 0 || l.items[l.shown[self.k - 1] as usize].group != it.group;
        let (row, h) = if self.query_is_empty() && new_group && !self.head_done {
            self.head_done = true;
            (Row::Head(it.group), l.metrics.head)
        } else {
            self.head_done = false;
            self.k += 1;
            (Row::Item(self.k - 1), l.row_h(it))
        };
        let p = Placed { row, top: self.y, h };
        self.y += h;
        Some(p)
    }
}

impl Rows<'_> {
    fn query_is_empty(&self) -> bool {
        self.l.q.is_empty()
    }
}

// ----------------------------------------------------------------- painting --

/// A paint over rows `y0..y1` of the window only: what is drawn through it cannot spill out of a viewport
/// (a row cut by the top or bottom of a scrolling list).
pub fn clipped<'a>(p: &'a mut Paint, y0: i32, y1: i32) -> Paint<'a> {
    let (w, h) = (p.w, p.h as i32);
    let (f0, f1) = ((y0 + p.oy).clamp(0, h), (y1 + p.oy).clamp(0, h));
    let (a, b) = ((f0 as usize * w).min(p.fb.len()), (f1.max(f0) as usize * w).min(p.fb.len()));
    Paint { fb: &mut p.fb[a..b], w, h: (f1 - f0).max(0) as usize, ox: p.ox, oy: p.oy - f0, fade: p.fade }
}

/// The palette's footer: a hairline, then `hints` as key caps and a label, 36 px high.
pub const FOOT_H: i32 = 35;

/// What a footer hint shows: the caps and the label after them.
pub type Hint<'a> = (&'a [Cap<'a>], &'a str);

pub fn paint_footer(p: &mut Paint, t: &Text, (x, y, w, _): Rect, hints: &[Hint]) {
    p.shape(x, y, x + w, y + 1, WHITE, 0.07, &|_, _| 1.0);
    let cy = y + 1 + (FOOT_H - 1) / 2;
    let mut at = x + 16;
    for (caps, label) in hints {
        for &c in caps.iter() {
            at += kbd(p, t, at, cy, c, 1.0) + KBD_GAP;
        }
        t.draw(p, at, cy, label, Ink::new(PX_SMALL, faint()));
        at += t.width(label, PX_SMALL) + 16;
    }
}

/// Draws `s` at `x` with the characters whose index is in `hits` in the accent colour. Returns its width. The runs of
/// lit and plain characters are slices of `s`: nothing is copied.
fn highlighted(p: &mut Paint, t: &Text, x: i32, cy: i32, s: &str, ink: Ink, hits: impl Iterator<Item = usize> + Clone) -> i32 {
    if hits.clone().next().is_none() {
        t.draw(p, x, cy, s, ink);
        return t.width(s, ink.px);
    }
    let mut at = x;
    let (mut from, mut lit) = (0, false);
    let flush = |p: &mut Paint, run: &str, lit: bool, at: &mut i32| {
        if !run.is_empty() {
            t.draw(p, *at, cy, run, if lit { Ink { c: accent(), ..ink } } else { ink });
            *at += t.width(run, ink.px);
        }
    };
    for (i, (b, _)) in s.char_indices().enumerate() {
        let on = hits.clone().any(|h| h == i);
        if on != lit {
            flush(p, &s[from..b], lit, &mut at);
            (from, lit) = (b, on);
        }
    }
    flush(p, &s[from..], lit, &mut at);
    at - x
}

/// Paints the list in the viewport `r` in the palette's style: the heading rows, the entries (a 24 px icon
/// box with an icon or a dot, the label with the characters that matched lit in the accent colour, the
/// description under it, the group's name when a query mixes the groups, the shortcut as key caps), the
/// selection, the scrollbar, and `Nothing matches` when the query keeps nothing. `empty` is that message.
pub fn paint(p: &mut Paint, t: &Text, l: &List, r: Rect, empty: &str) {
    let (x, y, w, h) = r;
    if l.is_empty() {
        let msg = format!("{empty} «{}».", l.query());
        let msg = t.fit(&msg, PX, w - 32);
        let mw = t.width(&msg, PX);
        t.draw(p, x + (w - mw) / 2, y + 6 + 28 + 9, &msg, Ink::new(PX, dim()));
        return;
    }
    let overflow = l.content_h() > h;
    let bar = if overflow { 10 } else { 0 };
    // The viewport is the rectangle given; the scroll is kept inside it even if the window just changed size.
    let scroll = l.scroll().min((l.content_h() - h).max(0));
    let mut p = clipped(p, y, y + h);
    let p = &mut p;
    let pad = l.metrics.pad;
    for r in l.rows() {
        let top = y - scroll + r.top;
        if top + r.h <= y || top >= y + h {
            continue;
        }
        let rx = x + pad;
        let rw = w - 2 * pad - bar;
        match r.row {
            Row::Head(g) => {
                let name = l.group_name(g);
                t.draw(p, rx + 10, top + 4 + (r.h - 4) / 2, name, Ink::new(PX_HEAD, faint()));
            }
            Row::Item(n) => paint_item(p, t, l, n, (rx, top, rw, r.h)),
        }
    }
    if let Some((ty, th)) = l.thumb_in(h) {
        p.rrect(x + w - 8, y + ty, 5, th, 2.5, WHITE, 0.16);
    }
}

fn paint_item(p: &mut Paint, t: &Text, l: &List, n: usize, (x, y, w, h): Rect) {
    let Some(e) = l.index(n) else { return };
    let cy = y + h / 2;
    if n == l.selected() {
        p.rrect(x, y, w, h, 9.0, WHITE, 0.09);
    }
    // The icon box.
    let (bx, by) = (x + 10, cy - 12);
    p.rrect(bx, by, 24, 24, 7.0, WHITE, 0.05);
    match l.mark(e) {
        Mark::None => {}
        Mark::Icon(i) => icons::paint(p, i, bx as f32 + 12.0, cy as f32, 13.0, 1.5, dim(), 1.0),
        Mark::Dot(c) => p.disc(bx as f32 + 12.0, cy as f32, 4.5, group_color(c), 1.0),
    }
    let tx = bx + 24 + 11;
    // The right end: the shortcut's caps, with the group's name before them when a query mixes the groups.
    let mut right = x + w - 10;
    let keys = l.keys(e);
    if !keys.is_empty() {
        right = kbd_row(p, t, right, cy, keys, 1.0);
    }
    if !l.query().is_empty() {
        let g = l.group(e);
        let gw = t.width(g, PX_HEAD);
        right -= if keys.is_empty() { 0 } else { 6 };
        t.draw(p, right - gw, cy, g, Ink::new(PX_HEAD, faint()));
        right -= gw + 11;
    } else {
        right -= 11;
    }
    let room = (right - tx).max(0);
    let desc = l.desc(e);
    // Hits past a cut are not drawn: the cut text is a prefix of the label, and only its characters are asked about.
    let label = t.fit(l.label(e), PX, room);
    if desc.is_empty() {
        highlighted(p, t, tx, cy, &label, Ink::new(PX, fg()).medium(), l.label_hits(n));
    } else {
        highlighted(p, t, tx, y + 8 + 9, &label, Ink::new(PX, fg()).medium(), l.label_hits(n));
        highlighted(p, t, tx, y + 8 + 18 + 1 + 8, &t.fit(desc, PX_SMALL, room), Ink::new(PX_SMALL, dim()), l.desc_hits(n));
    }
}

impl List {
    /// The heading text of group `g`.
    pub fn group_name(&self, g: u8) -> &str {
        self.get(self.groups[g as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xkb::Mods;

    fn key(sym: u32) -> Key {
        Key { sym, ..Key::default() }
    }

    fn chars(s: &str) -> Vec<char> {
        s.chars().map(fold).collect()
    }

    /// The reference's palette as its screenshots show it (in Spanish): actions, a window, a group.
    fn palette() -> List {
        let mut l = List::default();
        let act = |label: &'static str, keys: &'static str, kw: &'static str, id| Entry { label, keys, keywords: kw, group: "Acciones", mark: Mark::Icon(Icon::Plus), id, ..Entry::default() };
        l.push(act("Nueva ventana", "Alt+N", "pestaña tab", 1));
        l.push(act("Nuevo grupo…", "Alt+G", "agrupar", 2));
        l.push(act("Dividir en columnas", "Alt+\\", "panel dividir", 3));
        l.push(act("Vista general de ventanas", "Alt+O", "ventanas tarjetas", 4));
        l.push(act("Ventanas y grupos a la izquierda", "", "panel posición lateral", 5));
        l.push(act("Ventanas y grupos a la derecha", "", "panel posición lateral", 6));
        l.push(act("Ventanas y grupos arriba (pestañas)", "", "panel posición pestañas", 7));
        l.push(Entry { label: "servidor", desc: "~/lab/servidor · arcton", keywords: "arcton", group: "Ventanas", mark: Mark::Icon(Icon::Prompt), id: 8, keys: "Alt+2" });
        l.push(Entry { label: "arcton", desc: "3 ventanas", keywords: "grupo", group: "Grupos", mark: Mark::Dot(1), id: 9, ..Entry::default() });
        l.filter("");
        l.view = 380;
        l
    }

    fn labels(l: &List) -> Vec<&str> {
        (0..l.len()).map(|n| l.label(l.index(n).unwrap())).collect()
    }

    #[test]
    fn accents_and_case_are_folded_one_character_for_one() {
        assert_eq!(chars("Pestañas ÉCOLE Çà"), chars("pestanas ecole ca"));
        assert_eq!(fold('Ñ'), 'n');
        assert_eq!(fold('ß'), 'ß', "what NFD does not decompose stays");
        assert_eq!(fold('…'), '…');
    }

    #[test]
    fn a_query_found_whole_scores_100_less_its_position_and_a_subsequence_scores_20() {
        let t = chars("Nuevo grupo… agrupar");
        assert_eq!(score(&chars("nuevo"), &t, None), Some(100));
        assert_eq!(score(&chars("grupo"), &t, None), Some(94));
        // Far into the text the penalty stops at 60.
        let long = chars(&format!("{}x", "a".repeat(100)));
        assert_eq!(score(&chars("x"), &long, None), Some(40));
        // Letters in order, not together.
        assert_eq!(score(&chars("ngp"), &t, None), Some(20));
        assert_eq!(score(&chars("pgn"), &t, None), None);
        assert_eq!(score(&chars("zzz"), &t, None), None);
        assert_eq!(score(&[], &t, None), None);
        // It may span the separators of the searched text: label, description, keywords.
        assert_eq!(score(&chars("… ag"), &t, None), Some(100 - 11));
    }

    #[test]
    fn the_positions_that_matched_are_reported_for_both_kinds_of_match() {
        let t = chars("Nuevo grupo… agrupar");
        let mut hits = Vec::new();
        score(&chars("grupo"), &t, Some(&mut hits));
        assert_eq!(hits, [6, 7, 8, 9, 10]);
        hits.clear();
        score(&chars("ngp"), &t, Some(&mut hits));
        assert_eq!(hits, [0, 6, 9], "n, then g, then p, each the first one after the last");
    }

    #[test]
    fn with_no_query_everything_is_listed_in_order_under_its_group_headings() {
        let l = palette();
        assert_eq!((l.total(), l.len()), (9, 9));
        let rows: Vec<Row> = l.rows().map(|r| r.row).collect();
        assert_eq!(rows[0], Row::Head(0));
        assert_eq!(rows[1], Row::Item(0));
        assert_eq!(rows.iter().filter(|r| matches!(r, Row::Head(_))).count(), 3);
        assert_eq!(rows.iter().filter(|r| matches!(r, Row::Item(_))).count(), 9);
        assert_eq!(l.group_name(0), "Acciones");
        assert_eq!(l.group(7), "Ventanas");
        assert_eq!(l.hits(0), &[] as &[u16], "nothing matched without a query");
    }

    #[test]
    fn the_reference_ranks_grupo_this_way_and_ties_keep_their_order() {
        // The reference's screenshot for «grupo»: Nuevo grupo… (found at 6), the three «Ventanas y grupos…»
        // (at 11), then the group arcton (found only in its keywords).
        let mut l = palette();
        l.filter("grupo");
        assert_eq!(labels(&l), ["Nuevo grupo…", "Ventanas y grupos a la izquierda", "Ventanas y grupos a la derecha", "Ventanas y grupos arriba (pestañas)", "arcton"]);
        assert_eq!(l.query(), "grupo");
        // With a query there are no headings.
        assert!(l.rows().all(|r| matches!(r.row, Row::Item(_))));
        assert_eq!(l.chosen(), Some(1));
        assert_eq!(l.id(1), 2);
    }

    #[test]
    fn a_query_is_trimmed_folded_and_looks_in_descriptions_and_keywords() {
        let mut l = palette();
        l.filter("  PESTANAS ");
        assert_eq!(labels(&l), ["Ventanas y grupos arriba (pestañas)"]);
        l.filter("tarjetas");
        assert_eq!(labels(&l), ["Vista general de ventanas"], "a keyword finds it");
        l.filter("lab/serv");
        assert_eq!(labels(&l), ["servidor"], "so does the description");
        l.filter("alt+n");
        assert!(l.is_empty(), "the shortcut is not searched");
        l.filter("   ");
        assert_eq!(l.len(), 9, "only spaces is no query");
    }

    #[test]
    fn the_matched_characters_of_the_label_and_of_the_description_are_reported_apart() {
        let mut l = palette();
        l.filter("grupo");
        assert_eq!(l.label_hits(0).collect::<Vec<_>>(), [6, 7, 8, 9, 10]);
        l.filter("arcton");
        assert_eq!(labels(&l), ["arcton", "servidor"]);
        assert_eq!(l.label_hits(0).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5]);
        assert_eq!(l.label_hits(1).count(), 0);
        // «~/lab/servidor · arcton»: «arcton» starts at its 17th character.
        assert_eq!(l.desc_hits(1).collect::<Vec<_>>(), [17, 18, 19, 20, 21, 22]);
        assert_eq!(l.desc_hits(0).count(), 0, "arcton the group has a description that does not match");
    }

    #[test]
    fn at_most_forty_entries_are_shown_for_a_query_and_seventy_for_none() {
        let mut l = List::default();
        for i in 0..100 {
            l.push(Entry { label: "alpha", group: "g", id: i, ..Entry::default() });
        }
        l.filter("");
        assert_eq!(l.len(), MAX_ALL);
        l.filter("alp");
        assert_eq!(l.len(), MAX_QUERY);
        assert_eq!(l.total(), 100);
        assert_eq!(l.id(l.index(0).unwrap()), 0, "ties in the order given");
        assert_eq!(l.id(l.index(39).unwrap()), 39);
    }

    #[test]
    fn up_and_down_wrap_page_home_and_end_stop_and_enter_chooses() {
        let mut l = palette();
        l.filter("a");
        let n = l.len();
        assert!(n > 5);
        assert_eq!(l.key(&key(sym::UP)), Nav::Moved, "wraps to the last");
        assert_eq!(l.selected(), n - 1);
        assert_eq!(l.key(&key(sym::DOWN)), Nav::Moved);
        assert_eq!(l.selected(), 0);
        assert_eq!(l.key(&key(sym::END)), Nav::Moved);
        assert_eq!(l.selected(), n - 1);
        assert_eq!(l.key(&key(sym::END)), Nav::Same);
        assert_eq!(l.key(&key(sym::HOME)), Nav::Moved);
        assert_eq!(l.key(&key(sym::PAGE_DOWN)), Nav::Moved);
        assert_eq!(l.selected(), l.page().min(n - 1));
        l.key(&key(sym::END));
        assert_eq!(l.key(&key(sym::PAGE_DOWN)), Nav::Same, "stops at the end");
        assert_eq!(l.key(&key(sym::PAGE_UP)), Nav::Moved);
        assert_eq!(l.selected(), n - 1 - l.page());
        l.key(&key(sym::HOME));
        assert_eq!(l.key(&key(sym::PAGE_UP)), Nav::Same, "and at the start");
        let want = l.chosen().unwrap();
        assert_eq!(l.key(&key(sym::RETURN)), Nav::Chose(want));
        assert_eq!(l.key(&key(sym::KP_ENTER)), Nav::Chose(want));
    }

    #[test]
    fn keys_that_are_not_the_lists_are_left_alone_and_an_empty_list_chooses_nothing() {
        let mut l = palette();
        assert_eq!(l.key(&key(0x61)), Nav::Ignored);
        assert_eq!(l.key(&key(sym::TAB)), Nav::Ignored);
        assert_eq!(l.key(&key(sym::ESCAPE)), Nav::Ignored);
        assert_eq!(l.key(&Key { sym: sym::DOWN, mods: Mods { alt: true, ..Mods::default() }, ..Key::default() }), Nav::Ignored);
        l.filter("zzzz");
        assert!(l.is_empty());
        assert_eq!(l.key(&key(sym::DOWN)), Nav::Same);
        assert_eq!(l.key(&key(sym::RETURN)), Nav::Same);
        assert_eq!(l.chosen(), None);
    }

    #[test]
    fn rows_are_sized_by_their_metrics_and_a_description_makes_a_row_taller() {
        let l = palette();
        let rows: Vec<Placed> = l.rows().collect();
        assert_eq!(rows[0], Placed { row: Row::Head(0), top: 6, h: 28 });
        assert_eq!(rows[1], Placed { row: Row::Item(0), top: 34, h: 40 });
        let with_desc = rows.iter().find(|r| r.row == Row::Item(7)).unwrap();
        assert_eq!(with_desc.h, 51);
        let sum: i32 = rows.iter().map(|r| r.h).sum();
        assert_eq!(l.content_h(), sum + 12);
        assert_eq!(rows.last().map(|r| r.top + r.h + 6), Some(l.content_h()));
        // Each row starts where the one before ended.
        for w in rows.windows(2) {
            assert_eq!(w[1].top, w[0].top + w[0].h);
        }
    }

    #[test]
    fn the_selection_scrolls_the_list_just_enough_to_stay_in_view() {
        let mut l = palette();
        l.view = 200;
        assert_eq!(l.scroll(), 0);
        assert!(l.overflows());
        // Down to an entry below the fold: its bottom edge lands on the viewport's.
        l.select(5);
        let r = l.place_of(5).unwrap();
        assert_eq!(l.scroll(), r.top + r.h - 200);
        // Back up to one above: its top edge lands on the viewport's.
        l.select(1);
        assert_eq!(l.scroll(), l.place_of(1).unwrap().top);
        // The first entry shows its heading and the padding: all the way up.
        l.select(0);
        assert_eq!(l.scroll(), 0);
        // The last scrolls to the very end, padding included.
        l.select(8);
        assert_eq!(l.scroll(), l.content_h() - 200);
        // A selection that is already visible does not scroll.
        l.select(7);
        assert_eq!(l.scroll(), l.content_h() - 200);
    }

    #[test]
    fn choosing_before_the_first_layout_does_not_scroll_anywhere() {
        let mut l = palette();
        l.view = 0;
        l.select(5);
        assert_eq!((l.selected(), l.scroll()), (5, 0), "the viewport is not known yet");
    }

    #[test]
    fn the_wheel_scrolls_by_pixels_and_stops_at_both_ends() {
        let mut l = palette();
        l.view = 200;
        assert!(!l.scroll_by(-30.0), "already at the top");
        assert!(l.scroll_by(48.0));
        assert_eq!(l.scroll(), 48);
        assert!(l.scroll_by(1000.0));
        assert_eq!(l.scroll(), l.content_h() - 200);
        assert!(!l.scroll_by(10.0));
        // A list that fits does not scroll.
        l.view = 2000;
        l.scroll = 0;
        assert!(!l.scroll_by(100.0) && !l.overflows());
    }

    #[test]
    fn the_scrollbar_thumb_is_in_proportion_and_travels_the_track() {
        let mut l = palette();
        l.view = 200;
        let (top, h) = l.thumb().unwrap();
        assert_eq!(top, 0);
        assert!(h > 20 && h < 200 && h == 200 * 200 / l.content_h());
        l.scroll_by(1e6);
        let (top, h2) = l.thumb().unwrap();
        assert_eq!((top + h2, h2), (200, h), "the thumb ends at the track's end");
        l.view = 2000;
        assert_eq!(l.thumb(), None, "nothing to scroll: no thumb");
        // A long list does not shrink the thumb below a grip.
        let mut long = List::default();
        for i in 0..70 {
            long.push(Entry { label: "x", id: i, ..Entry::default() });
        }
        long.filter("");
        long.view = 100;
        assert_eq!(long.thumb().unwrap().1, 20);
    }

    #[test]
    fn the_pointer_finds_rows_and_headings_are_not_rows() {
        let mut l = palette();
        l.view = 400;
        // y = 10 is on the first heading (6 to 34), 34 to 74 the first entry.
        assert_eq!(l.row_at(10), None);
        assert_eq!(l.row_at(34), Some(0));
        assert_eq!(l.row_at(73), Some(0));
        assert_eq!(l.row_at(74), Some(1));
        assert_eq!(l.row_at(5000), None);
        // Hovering selects, once, and not a heading.
        assert!(l.hover(80));
        assert_eq!(l.selected(), 1);
        assert!(!l.hover(85), "the same row");
        assert!(!l.hover(10), "a heading changes nothing");
        assert_eq!(l.selected(), 1);
        // Scrolled, the same y is another row.
        l.view = 100;
        l.scroll_by(40.0);
        assert_eq!(l.row_at(34), Some(1));
        assert_eq!(l.row_at(0), Some(0));
    }

    #[test]
    fn filtering_again_goes_back_to_the_top_with_the_first_selected() {
        let mut l = palette();
        l.view = 100;
        l.select(7);
        assert!(l.scroll() > 0);
        l.filter("a");
        assert_eq!((l.selected(), l.scroll()), (0, 0));
    }

    #[test]
    fn a_paint_cannot_spill_out_of_its_band() {
        let mut fb = vec![0u32; 50 * 100];
        let mut p = Paint { fb: &mut fb, w: 50, h: 100, ox: 5, oy: 3, fade: 1.0 };
        {
            let mut c = clipped(&mut p, 20, 40);
            c.rrect(-5, 0, 40, 100, 0.0, WHITE, 1.0);
            c.put(0, 19, WHITE, 1.0);
            c.put(0, 40, WHITE, 1.0);
        }
        let rows: Vec<usize> = (0..100).filter(|&y| fb[y * 50..][..50].iter().any(|&px| px != 0)).collect();
        // Window rows 20..40 are framebuffer rows 23..43.
        assert_eq!((rows.first(), rows.last()), (Some(&23), Some(&42)));
        assert_eq!(rows.len(), 20);
        // A band partly above the buffer is cut, not wrapped.
        let mut fb = vec![0u32; 50 * 100];
        let mut p = Paint { fb: &mut fb, w: 50, h: 100, ox: 5, oy: 3, fade: 1.0 };
        let mut c = clipped(&mut p, -50, 5);
        c.put(0, -4, WHITE, 1.0);
        c.put(0, 4, WHITE, 1.0);
    }

    #[test]
    fn the_list_paints_inside_its_viewport_with_its_selection_and_highlights() {
        let t = Text::load();
        let mut l = palette();
        l.view = 200;
        let paint_it = |l: &List| {
            let mut fb = vec![0u32; 600 * 400];
            let mut p = Paint { fb: &mut fb, w: 600, h: 400, ox: 0, oy: 0, fade: 1.0 };
            paint(&mut p, &t, l, (10, 100, 580, 200), "Nothing matches");
            fb
        };
        l.select(3);
        let fb = paint_it(&l);
        let rows: Vec<usize> = (0..400).filter(|&y| fb[y * 600..][..600].iter().any(|&px| px != 0)).collect();
        assert!(rows.first().is_some_and(|&y| y >= 100) && rows.last().is_some_and(|&y| y < 300), "{:?}..{:?}", rows.first(), rows.last());
        // A query lights the matched letters: the picture differs from the plain one in colour, not only in rows.
        l.filter("grupo");
        let lit = paint_it(&l);
        let amber = |px: u32| (px >> 16 & 255) > 200 && (px >> 8 & 255) > 140 && (px & 255) < 140;
        assert!(lit.iter().any(|&px| amber(px)), "the matched characters are amber");
        assert!(!fb.iter().any(|&px| amber(px)), "and nothing is amber without a query");
        // No match: the message.
        l.filter("zzzz");
        assert!(paint_it(&l).iter().any(|&px| px != 0));
    }

    #[test]
    fn the_footer_draws_caps_and_labels_after_a_hairline() {
        let t = Text::load();
        let mut fb = vec![0u32; 600 * 60];
        let mut p = Paint { fb: &mut fb, w: 600, h: 60, ox: 0, oy: 0, fade: 1.0 };
        let hints: [Hint; 2] = [(&[Cap::Icon(Icon::Up), Cap::Icon(Icon::Down)], "move"), (&[Cap::Text("Esc")], "close")];
        paint_footer(&mut p, &t, (0, 10, 580, FOOT_H), &hints);
        assert!(fb[10 * 600 + 300] >> 24 != 0, "the hairline");
        assert_eq!(fb[9 * 600 + 300], 0);
        assert!((10..10 + FOOT_H as usize).any(|y| fb[y * 600..][..600].iter().filter(|&&px| px != 0).count() > 40));
    }
}
