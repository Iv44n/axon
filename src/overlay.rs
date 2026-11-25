//! The overlay kit ([0025](../knowledge/decisions/workspace/0025-overlay-kit.md)): what the command palette, the
//! overview, the dialogs, the context menus and the toasts are built from.
//!
//! * [`Overlay`] is what a screen over the grid implements: a key, the pointer, a wheel, a draw. The app holds
//!   a [`Stack`] of them (`wlapp.rs`): the top one gets the keyboard and the pointer before anything else,
//!   `Esc` closes it, a press outside it closes it, and the terminal hears nothing while one is open.
//! * [`Modal`] is the frame of a dialog: the scrim over the window, a glass card with its border, radius and
//!   shadow, and the fade-in of the settings screen.
//! * [`Text`] draws the kit's text (the desktop's proportional font, loaded while an overlay is open and
//!   dropped after), [`Ink`] says how; [`kbd`] draws a key cap.
//! * The pieces that go inside a card live apart: `field.rs` (a line of text being edited), `list.rs` (a
//!   filtered, scrolling list), `menu.rs` (a context menu, an overlay itself) and `toast.rs` (a message that
//!   goes by itself; the stack owns it next to the overlays).
//!
//! Like the settings screen, nothing here is kept between frames: an overlay is a pure function of its state
//! and the window size, painted over a complete grid on every redraw while it is open; its coordinates are
//! window pixels (the origin is the window's top-left corner, not the shadow's).

use std::{any::Any, borrow::Cow, cell::RefCell, rc::Rc, time::Instant};

use crate::{
    chrome::{self, TITLE_H},
    config::FontCfg,
    font::{Fonts, UiFont},
    icons::{self, Icon},
    paint::{BLACK, Paint, WHITE},
    palette,
    toast::Toast,
    xkb::Key,
};

pub type Rect = (i32, i32, i32, i32);

/// Whether window pixel `(px, py)` is inside `r`.
pub fn inside((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x as f32 && px < (x + w) as f32 && py >= y as f32 && py < (y + h) as f32
}

/// The smoothstep the fades use.
pub fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

// ----------------------------------------------------------------- tokens --

/// The reference's surfaces: a dialog or palette (`#141416`), a menu or toast (`#17171a`), the dimmed scrim.
pub const CARD: [f32; 3] = [0.078, 0.078, 0.086];
pub const POP: [f32; 3] = [0.090, 0.090, 0.102];
pub const SCRIM: [f32; 3] = [0.024, 0.024, 0.027];
/// Text sizes of the reference's overlays, in pixels: the base size, the menu and toast, a hint, small text,
/// a group heading, a key cap, the palette's input, a dialog's title.
pub const PX: f32 = 13.0;
pub const PX_MENU: f32 = 12.5;
pub const PX_HINT: f32 = 12.0;
pub const PX_SMALL: f32 = 11.5;
pub const PX_HEAD: f32 = 11.0;
pub const PX_KBD: f32 = 10.5;
pub const PX_INPUT: f32 = 14.5;
pub const PX_TITLE: f32 = 15.0;

pub fn fg() -> [f32; 3] {
    palette::unit(palette::FG)
}

pub fn dim() -> [f32; 3] {
    palette::unit(palette::DIM)
}

pub fn faint() -> [f32; 3] {
    palette::unit(palette::FAINT)
}

pub fn accent() -> [f32; 3] {
    palette::unit(palette::ACCENT)
}

// ------------------------------------------------------------------- text --

/// The font the kit draws in. The desktop's proportional UI font when the machine has one (like the settings
/// screen), else the grid's monospace font at the size asked for. ~5 MB of parsed outlines that the terminal
/// itself never needs: the stack loads it when the first overlay opens and drops it when the last one closes.
pub struct Text {
    prop: Option<Rc<UiFont>>,
    mono: Option<Fonts>,
    /// One glyph of the monospace fallback.
    scratch: RefCell<Vec<u8>>,
}

/// The monospace fallback's base size; others are `Fonts::small` of it.
const MONO_PX: f32 = 13.0;

/// How a piece of text is drawn: size, colour, alpha, weight and the columns it may touch.
#[derive(Clone, Copy, Debug)]
pub struct Ink {
    pub px: f32,
    pub c: [f32; 3],
    pub a: f32,
    /// 400 regular, `font::MEDIUM`, `font::BOLD`: synthesized from the regular file.
    pub weight: u16,
    /// Pixels outside `clip.0..clip.1` (window x) are left alone: a field showing the end of a long text.
    pub clip: (i32, i32),
}

impl Ink {
    pub fn new(px: f32, c: [f32; 3]) -> Ink {
        Ink { px, c, a: 1.0, weight: 400, clip: (i32::MIN, i32::MAX) }
    }

    pub fn alpha(mut self, a: f32) -> Ink {
        self.a = a;
        self
    }

    pub fn bold(mut self) -> Ink {
        self.weight = crate::font::BOLD;
        self
    }

    /// The reference's `font-weight: 500`.
    pub fn medium(mut self) -> Ink {
        self.weight = crate::font::MEDIUM;
        self
    }

    pub fn clip(mut self, x0: i32, x1: i32) -> Ink {
        self.clip = (x0, x1);
        self
    }
}

impl Text {
    pub fn load() -> Text {
        let prop = UiFont::shared();
        let mono = prop.is_none().then(|| Fonts::new(&FontCfg::sized(MONO_PX)));
        Text { prop, mono, scratch: RefCell::default() }
    }

    /// Width in pixels of `s` at `px`.
    pub fn width(&self, s: &str, px: f32) -> i32 {
        match (&self.prop, &self.mono) {
            (Some(f), _) => f.width(s, px),
            (None, Some(m)) => s.chars().count() as i32 * m.small(px / MONO_PX).cell_w as i32,
            _ => 0,
        }
    }

    /// `s` cut with `…` so that it is at most `max` pixels wide at `px`.
    pub fn fit<'a>(&self, s: &'a str, px: f32, max: i32) -> Cow<'a, str> {
        fit(s, max, |t| self.width(t, px))
    }

    /// `s` with its left edge at `x` and its vertical centre at `cy` (window pixels).
    pub fn draw(&self, p: &mut Paint, x: i32, cy: i32, s: &str, k: Ink) {
        let (x0, x1) = k.clip;
        if let Some(f) = &self.prop {
            // Centred on the middle of the capitals, the way the eye reads a label.
            let base = cy + (f.cap_height(k.px) / 2.0).round() as i32;
            f.draw(s, k.px, x, base, k.weight, |gx, gy, cov| {
                if gx >= x0 && gx < x1 {
                    p.put(gx, gy, k.c, k.a * cov as f32 / 255.0);
                }
            });
            return;
        }
        let Some(fonts) = &self.mono else { return };
        let m = fonts.small(k.px / MONO_PX);
        let (cw, ch) = (m.cell_w as i32, m.cell_h as i32);
        let mut buf = self.scratch.borrow_mut();
        for (n, c) in s.chars().enumerate() {
            if c == ' ' {
                continue;
            }
            fonts.raster_into(&mut buf, &m, c, k.weight > crate::font::MEDIUM, false);
            let (gx, gy) = (x + n as i32 * cw, cy - ch / 2);
            for yy in 0..ch {
                for xx in 0..cw {
                    let i = ((yy * cw + xx) * 3) as usize;
                    let cov = (buf[i] as u32 + buf[i + 1] as u32 + buf[i + 2] as u32) / 3;
                    if cov != 0 && gx + xx >= x0 && gx + xx < x1 {
                        p.put(gx + xx, gy + yy, k.c, k.a * cov as f32 / 255.0);
                    }
                }
            }
        }
    }

}

/// `s` cut with `…` to at most `max` pixels, `width` measuring a string. Pure, so it is tested without a font.
pub fn fit<'a>(s: &'a str, max: i32, width: impl Fn(&str) -> i32) -> Cow<'a, str> {
    if width(s) <= max {
        return Cow::Borrowed(s);
    }
    let ell = width("…");
    // The longest prefix that fits with the ellipsis. A longer prefix is never narrower, so the cut is found by halving
    // (measuring the text once for each halving, not once for each of its characters).
    let prefix = |k: usize| s.char_indices().nth(k).map_or(s, |(b, _)| &s[..b]);
    let (mut lo, mut hi) = (0, s.chars().count());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if width(prefix(mid)) + ell <= max {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let kept = prefix(lo);
    let mut out = String::with_capacity(kept.len() + '…'.len_utf8());
    out.push_str(kept);
    out.push('…');
    Cow::Owned(out)
}

/// `n` in decimal in `buf`, which has room for any: a number that is painted again with every frame needs no
/// string of its own.
pub fn dec(n: usize, buf: &mut [u8; 20]) -> &str {
    let (mut at, mut n) = (buf.len(), n);
    loop {
        at -= 1;
        buf[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    std::str::from_utf8(&buf[at..]).unwrap_or_default()
}

// ------------------------------------------------------------------- fade --

/// A fade-in clock, the settings screen's mechanism: `step` moves it to `now` once per frame and says
/// whether there is more to show, so the caller asks for another frame only while it runs (idle stays at
/// 0 % CPU); `value` is the eased 0 to 1 that multiplies every alpha (`Paint::fade`).
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    t: f32,
    clock: Option<Instant>,
    secs: f32,
}

impl Fade {
    /// Starts invisible and takes `secs` to appear. The first frame is the invisible one: time counts from the next.
    pub const fn rising(secs: f32) -> Fade {
        Fade { t: 0.0, clock: None, secs }
    }

    /// Already fully shown.
    #[cfg(test)]
    pub const fn full() -> Fade {
        Fade { t: 1.0, clock: None, secs: 0.0 }
    }

    pub fn step(&mut self, now: Instant) -> bool {
        let dt = self.clock.map_or(0.0, |c| now.saturating_duration_since(c).as_secs_f32());
        self.clock = Some(now);
        self.t = if self.secs > 0.0 { (self.t + dt / self.secs).min(1.0) } else { 1.0 };
        self.t < 1.0
    }

    #[cfg(test)]
    pub fn animating(&self) -> bool {
        self.t < 1.0
    }

    pub fn value(&self) -> f32 {
        ease(self.t)
    }
}

// ------------------------------------------------------------ the capture --

/// The window an overlay is in: its size in pixels (without the shadow margin) and whether it is square
/// (maximized, fullscreen or tiled).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geo {
    pub w: i32,
    pub h: i32,
    pub square: bool,
}

/// What every call of an overlay gets: the window and the font.
pub struct Cx<'a> {
    pub g: Geo,
    pub text: &'a Text,
}

/// A primary-button press or release at window pixel `(x, y)`; `time` is the compositor's millisecond clock
/// (two presses close together are a double click).
#[derive(Clone, Copy, Debug)]
pub struct Click {
    pub x: f32,
    pub y: f32,
    pub pressed: bool,
    pub time: u32,
}

/// What an overlay asks the stack to do after an input.
#[derive(Debug, PartialEq, Eq)]
pub enum Out {
    /// Nothing changed.
    None,
    /// Something changed: paint again.
    Redraw,
    /// Close this overlay (it was cancelled).
    Close,
    /// Close this overlay and run the app's command `id` (a palette entry, a menu item, a button).
    Run(u32),
    /// Run the app's command `id` and keep this overlay open (the overview's "new window": the list goes on).
    Act(u32),
    /// Wants the clipboard's text: the stack reads it and calls `Overlay::paste`.
    Paste,
    /// Put this text on the clipboard.
    Copy(String),
}

/// The pointer image an overlay wants over a place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    Default,
    Pointer,
    Text,
}

/// The app's clipboard, handed in by the app: read only when an overlay asks for a paste.
pub trait Clipboard {
    fn get(&mut self) -> Option<String>;
    fn set(&mut self, text: &str);
}

/// A screen over the grid. The stack calls these in the order the user does things; every method has a
/// default so that a toast-like or a keyboard-only overlay implements only what it uses.
pub trait Overlay {
    /// A key press (and its repeats). Everything is yours except `Esc`, which the stack turns into a close
    /// unless `handles_escape` (an overlay with a rename box inside it, say).
    fn key(&mut self, k: &Key, cx: &Cx) -> Out;

    /// The clipboard's text, after asking for it with `Out::Paste`.
    fn paste(&mut self, _text: &str, _cx: &Cx) -> Out {
        Out::None
    }

    /// The pointer moved to `(x, y)` (also while a button is held).
    fn pointer(&mut self, _x: f32, _y: f32, _cx: &Cx) -> Out {
        Out::None
    }

    /// The primary button went down or up inside the overlay (a press outside it closes it before it gets here).
    fn button(&mut self, _c: Click, _cx: &Cx) -> Out {
        Out::None
    }

    /// The wheel moved `dy` pixels (down is positive) with the pointer at `(x, y)` inside the overlay.
    fn scroll(&mut self, _dy: f32, _x: f32, _y: f32, _cx: &Cx) -> Out {
        Out::None
    }

    /// Whether `(x, y)` is on the overlay itself; a press anywhere else is on the scrim and closes it. A
    /// full-window overlay (the overview) says true everywhere, and so does the default.
    fn contains(&self, _x: f32, _y: f32, _cx: &Cx) -> bool {
        true
    }

    /// The pointer image at `(x, y)`.
    fn cursor(&self, _x: f32, _y: f32, _cx: &Cx) -> Cursor {
        Cursor::Default
    }

    /// Moves the overlay's animations to `now` (once per frame, before `draw`). True while there is more to
    /// show: the app then asks for another frame, and at rest nothing does.
    fn step(&mut self, _now: Instant) -> bool {
        false
    }

    /// Paints the overlay over the finished frame: set `p.fade` from the fade, draw the scrim and the card
    /// (`Modal::paint`), then the contents.
    fn draw(&self, p: &mut Paint, cx: &Cx);

    /// Whether `Esc` is the overlay's own key (the stack then passes it on instead of closing).
    fn handles_escape(&self) -> bool {
        false
    }

    /// Paints what only the terminal can give (the overview's preview of a window's last rows) right after `draw`,
    /// through `grid`, with the same paint.
    fn draw_grid(&self, _p: &mut Paint, _grid: &mut dyn Grid, _cx: &Cx) {}

    /// The overlay as `Any`, for the app to refresh what it shows while it is open (`Stack::with`): the
    /// overview lists the windows, which change under it. Most overlays are a snapshot and say `None`.
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
}

/// What an overlay needs from the terminal and its grid font, which the kit does not have: the app gives it at
/// draw time (`Stack::draw_with`), so an overlay never holds a terminal or a font of the grid.
pub trait Grid {
    /// Appends to `out` the last `rows` text rows of window `id`'s focused pane (what is on its screen, ending at
    /// the last row with text or the cursor's, trailing blanks cut), one `\n` between rows; nothing when its terminal
    /// is busy (the app does not wait for it). True while a command runs in it.
    fn tail(&mut self, id: u32, rows: usize, out: &mut String) -> bool;

    /// `s` in the grid's font at `scale` of its size, its left edge at `x`, its vertical centre at `cy`, in the
    /// colour `c`, fully inside `p`.
    fn text(&mut self, p: &mut Paint, x: i32, cy: i32, s: &str, scale: f32, c: [f32; 3]);

    /// The width of a character and the height of a line at `scale`.
    fn metrics(&self, scale: f32) -> (i32, i32);
}

/// No terminal: the previews are left out (the kit's own tests and screens).
#[cfg(test)]
pub struct NoGrid;

#[cfg(test)]
impl Grid for NoGrid {
    fn tail(&mut self, _id: u32, _rows: usize, _out: &mut String) -> bool {
        false
    }

    fn text(&mut self, _p: &mut Paint, _x: i32, _cy: i32, _s: &str, _scale: f32, _c: [f32; 3]) {}

    fn metrics(&self, scale: f32) -> (i32, i32) {
        ((7.0 * scale / 0.75) as i32, (16.0 * scale / 0.75) as i32)
    }
}

/// What the app learns from an input: whether the overlays took it, what to repaint and what to run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reply {
    /// The overlays took the event: the app must not use it (the shell hears no key while one is open).
    pub taken: bool,
    pub redraw: bool,
    /// An overlay went away: the grid under it has to be painted again.
    pub closed: bool,
    pub ran: Option<Ran>,
    pub cursor: Option<Cursor>,
}

/// A command an overlay chose, and the tag of the overlay (or [`TOAST`]) it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ran {
    pub tag: u32,
    pub id: u32,
}

/// What `Stack::tick` found: nothing to do, a frame to draw (a fade is on), or a toast that went (the grid
/// under it has to be painted again).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    Rest,
    Repaint,
    Gone,
}

/// The `tag` of a command that came from a toast's action button.
pub const TOAST: u32 = u32::MAX;

struct Slot {
    tag: u32,
    o: Box<dyn Overlay>,
}

/// What the overlays paint over a frame (`Stack::extent`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Extent {
    Nothing,
    Window,
    /// A box, in window pixels.
    Part(Rect),
}

impl Extent {
    /// What the frame under the overlays has to give back when they were `self` in the last frame and are `now` in this
    /// one: the whole window, or one box. None when there is nothing to give back (neither painted anything, or only
    /// the first frame of a toast, which paints over what is clean).
    pub fn to_restore(self, now: Extent) -> Option<Extent> {
        match (self, now) {
            (Extent::Nothing, Extent::Nothing | Extent::Part(_)) => None,
            (Extent::Window, _) | (_, Extent::Window) => Some(Extent::Window),
            (Extent::Part(a), Extent::Nothing) => Some(Extent::Part(a)),
            (Extent::Part((ax, ay, aw, ah)), Extent::Part((bx, by, bw, bh))) => {
                let (x, y) = (ax.min(bx), ay.min(by));
                Some(Extent::Part((x, y, (ax + aw).max(bx + bw) - x, (ay + ah).max(by + bh) - y)))
            }
        }
    }
}

/// The overlays the app has open, bottom to top, and the toast. The app owns one; `wlapp.rs` sends it every
/// key, pointer move, button and wheel event first and uses what it did not take.
#[derive(Default)]
pub struct Stack {
    slots: Vec<Slot>,
    toast: Option<Toast>,
    text: Option<Text>,
}

impl Stack {
    /// Opens `o` on top of the others under `tag` (the app's own number for it: what it matches `Ran` on). An
    /// overlay already open under the same tag is replaced.
    pub fn open(&mut self, tag: u32, o: Box<dyn Overlay>) {
        self.slots.retain(|s| s.tag != tag);
        self.slots.push(Slot { tag, o });
        self.text.get_or_insert_with(Text::load);
    }

    /// Closes the overlay opened under `tag`. True if there was one.
    pub fn close(&mut self, tag: u32) -> bool {
        let n = self.slots.len();
        self.slots.retain(|s| s.tag != tag);
        self.trim();
        n != self.slots.len()
    }

    pub fn has(&self, tag: u32) -> bool {
        self.slots.iter().any(|s| s.tag == tag)
    }

    /// The tag of the overlay on top: the one with the keyboard.
    pub fn top(&self) -> Option<u32> {
        self.slots.last().map(|s| s.tag)
    }

    /// The overlay opened under `tag`, as its own type, for the app to bring up to date what it lists.
    pub fn with<T: Any>(&mut self, tag: u32) -> Option<&mut T> {
        self.slots.iter_mut().find(|s| s.tag == tag)?.o.as_any_mut()?.downcast_mut::<T>()
    }

    /// An overlay is open: it has the keyboard and the pointer.
    pub fn is_open(&self) -> bool {
        !self.slots.is_empty()
    }

    /// Nothing is shown, not even a toast: no frame has to carry the kit.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty() && self.toast.is_none()
    }

    /// Shows a message at the bottom centre for a few seconds, with an optional button that runs command
    /// `id` (reported as `Ran { tag: TOAST, id }`). It replaces the one on screen.
    pub fn toast(&mut self, msg: &str, action: Option<&str>, id: u32, now: Instant) {
        self.toast = Some(Toast::new(msg, action, id, now));
        self.text.get_or_insert_with(Text::load);
    }

    /// Drops the font once nothing is shown, and gives its memory back (it is kept by whoever else shares it).
    fn trim(&mut self) {
        if self.is_empty() && self.text.take().is_some() {
            crate::font::release_memory();
        }
    }

    /// What `out` from the overlay under `tag` means for the stack.
    fn apply(&mut self, out: Out, tag: u32, clip: Option<&mut dyn Clipboard>, g: Geo) -> Reply {
        let mut r = Reply { taken: true, ..Reply::default() };
        match out {
            Out::None => {}
            Out::Redraw => r.redraw = true,
            Out::Close => {
                self.close(tag);
                r.closed = true;
            }
            Out::Run(id) => {
                self.close(tag);
                (r.closed, r.ran) = (true, Some(Ran { tag, id }));
            }
            Out::Act(id) => (r.redraw, r.ran) = (true, Some(Ran { tag, id })),
            Out::Copy(text) => {
                if let Some(c) = clip {
                    c.set(&text);
                }
                // A cut changed the field it came from.
                r.redraw = true;
            }
            Out::Paste => {
                let text = clip.and_then(|c| c.get());
                let Stack { slots, text: font, .. } = self;
                if let (Some(text), Some(font), Some(top)) = (text, font.as_ref(), slots.last_mut()) {
                    let out = top.o.paste(&text, &Cx { g, text: font });
                    // A paste does not ask for a paste again.
                    if out == Out::Redraw {
                        r.redraw = true;
                    } else if let Out::Run(id) = out {
                        self.close(tag);
                        (r.closed, r.ran) = (true, Some(Ran { tag, id }));
                    } else if out == Out::Close {
                        self.close(tag);
                        r.closed = true;
                    }
                }
            }
        }
        self.trim();
        r
    }

    /// A key press. With an overlay open it is always taken: `Esc` closes the top one (unless it handles
    /// `Esc` itself), everything else is the overlay's. `clip` is read only if the overlay asks to paste.
    pub fn key(&mut self, k: &Key, g: Geo, clip: &mut dyn Clipboard) -> Reply {
        let Stack { slots, text, .. } = self;
        let (Some(font), Some(top)) = (text.as_ref(), slots.last_mut()) else { return Reply::default() };
        let tag = top.tag;
        let out = if k.sym == crate::input::sym::ESCAPE && !top.o.handles_escape() { Out::Close } else { top.o.key(k, &Cx { g, text: font }) };
        self.apply(out, tag, Some(clip), g)
    }

    /// The pointer moved. Only what is under it is told; the reply's `taken` says the pointer is on a toast.
    pub fn pointer(&mut self, x: f32, y: f32, g: Geo) -> Reply {
        let Stack { slots, text, toast } = self;
        let Some(font) = text.as_ref() else { return Reply::default() };
        let cx = Cx { g, text: font };
        let mut r = Reply::default();
        if let Some(t) = toast.as_mut() {
            let (on, changed) = t.hover(x, y, &cx);
            r.redraw |= changed;
            if on {
                r.taken = true;
                r.cursor = Some(if t.on_action(x, y, &cx) { Cursor::Pointer } else { Cursor::Default });
                return r;
            }
        }
        let Some(top) = slots.last_mut() else { return r };
        let tag = top.tag;
        let out = top.o.pointer(x, y, &cx);
        let cursor = top.o.cursor(x, y, &cx);
        let mut a = self.apply(out, tag, None, g);
        a.redraw |= r.redraw;
        a.cursor = Some(cursor);
        a
    }

    /// Whether a toast is under window pixel `(x, y)`.
    pub fn on_toast(&self, x: f32, y: f32, g: Geo) -> bool {
        match (&self.toast, &self.text) {
            (Some(t), Some(font)) => t.contains(x, y, &Cx { g, text: font }),
            _ => false,
        }
    }

    /// The primary button. A press on the scrim closes the top overlay; the release that follows is taken
    /// too, so the app never sees half a click. A toast's action runs on release.
    pub fn button(&mut self, c: Click, g: Geo) -> Reply {
        let Stack { slots, text, toast } = self;
        let Some(font) = text.as_ref() else { return Reply::default() };
        let cx = Cx { g, text: font };
        if let Some(t) = toast.as_mut().filter(|t| t.contains(c.x, c.y, &cx)) {
            let run = t.click(c, &cx);
            let mut r = Reply { taken: true, redraw: true, ..Reply::default() };
            if let Some(id) = run {
                *toast = None;
                (r.closed, r.ran) = (true, Some(Ran { tag: TOAST, id }));
                self.trim();
            }
            return r;
        }
        let Some(top) = slots.last_mut() else { return Reply::default() };
        let tag = top.tag;
        if !top.o.contains(c.x, c.y, &cx) {
            if c.pressed {
                self.close(tag);
                return Reply { taken: true, closed: true, ..Reply::default() };
            }
            return Reply { taken: true, ..Reply::default() };
        }
        let out = top.o.button(c, &cx);
        self.apply(out, tag, None, g)
    }

    /// The wheel, with the pointer at `(x, y)`; `dy` pixels, down positive.
    pub fn scroll(&mut self, dy: f32, x: f32, y: f32, g: Geo) -> Reply {
        let Stack { slots, text, .. } = self;
        let (Some(font), Some(top)) = (text.as_ref(), slots.last_mut()) else { return Reply::default() };
        let tag = top.tag;
        let cx = Cx { g, text: font };
        let out = if top.o.contains(x, y, &cx) { top.o.scroll(dy, x, y, &cx) } else { Out::None };
        self.apply(out, tag, None, g)
    }

    /// Paints everything open over a finished frame: the overlays bottom to top, then the toast. `(w, h)` is
    /// the framebuffer and `(ox, oy)` where the window starts in it. True while an animation wants another frame.
    #[cfg(test)]
    pub fn draw(&mut self, fb: &mut [u32], size: (usize, usize), origin: (usize, usize), g: Geo, now: Instant) -> bool {
        self.draw_with(fb, size, origin, g, now, &mut NoGrid)
    }

    /// `draw`, with the app's terminal text and grid font for the overlays that show some (`Overlay::draw_grid`).
    pub fn draw_with(&mut self, fb: &mut [u32], (w, h): (usize, usize), (ox, oy): (usize, usize), g: Geo, now: Instant, grid: &mut dyn Grid) -> bool {
        let Stack { slots, text, toast } = self;
        let Some(font) = text.as_ref() else { return false };
        if fb.len() < w * h {
            return false;
        }
        let mut animating = false;
        let mut p = Paint { fb, w, h, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        let cx = Cx { g, text: font };
        for s in slots.iter_mut() {
            animating |= s.o.step(now);
            s.o.draw(&mut p, &cx);
            s.o.draw_grid(&mut p, grid, &cx);
            p.fade = 1.0;
        }
        if let Some(t) = toast.as_ref() {
            animating |= t.animating(now);
            t.paint(&mut p, &cx, now);
        }
        animating
    }

    /// What the overlays paint over a frame: all of the window while one is open (it dims it), else only the toast's box,
    /// with its shadow. The app paints the window again under it before each frame, all of it or just that part.
    pub fn extent(&self, g: Geo) -> Extent {
        if self.is_open() {
            return Extent::Window;
        }
        match (&self.toast, &self.text) {
            (Some(t), Some(font)) => Extent::Part(t.extent(&Cx { g, text: font })),
            _ => Extent::Nothing,
        }
    }

    /// When the app must wake next for the toast (to start its fade, to take it away): none while there is
    /// no toast, so nothing runs while nothing is shown.
    pub fn deadline(&self, now: Instant) -> Option<Instant> {
        self.toast.as_ref().map(|t| t.deadline(now))
    }

    /// Called once per turn of the loop: takes away a toast whose time is up, and says when the picture has
    /// to change without any input (the toast is fading).
    pub fn tick(&mut self, now: Instant) -> Tick {
        match &self.toast {
            Some(t) if t.gone(now) => {
                self.toast = None;
                self.trim();
                Tick::Gone
            }
            Some(t) if t.animating(now) => Tick::Repaint,
            _ => Tick::Rest,
        }
    }
}

// ------------------------------------------------------------------ frames --

/// A solid rectangle at alpha `a` (times the paint's fade), edges on whole pixels: opaque is a store, anything
/// else two channels at a time in integers. The shapes of `Paint` test every pixel against a function, which
/// a scrim over the whole window or the inside of a card cannot afford.
pub fn fill(p: &mut Paint, x: i32, y: i32, w: i32, h: i32, c: [f32; 3], a: f32) {
    let a = (a * p.fade).clamp(0.0, 1.0);
    if a <= 0.0 || w <= 0 || h <= 0 {
        return;
    }
    let (x0, y0) = ((x + p.ox).max(0), (y + p.oy).max(0));
    let (x1, y1) = ((x + p.ox + w).min(p.w as i32), (y + p.oy + h).min(p.h as i32));
    if x0 >= x1 || y0 >= y1 || p.fb.len() < p.w * p.h {
        return;
    }
    let ch = |v: f32| (v * 255.0 + 0.5) as u32;
    let src = 0xff00_0000 | ch(c[0]) << 16 | ch(c[1]) << 8 | ch(c[2]);
    let w = p.w;
    if a >= 0.998 {
        for y in y0..y1 {
            p.fb[y as usize * w + x0 as usize..y as usize * w + x1 as usize].fill(src);
        }
        return;
    }
    let (k, inv) = ((a * 256.0 + 0.5) as u32, 256 - (a * 256.0 + 0.5) as u32);
    let (rb, ag) = ((src & 0x00ff_00ff) * k, ((src >> 8) & 0x00ff_00ff) * k);
    for y in y0..y1 {
        for px in &mut p.fb[y as usize * w + x0 as usize..y as usize * w + x1 as usize] {
            let d = *px;
            let o_rb = ((rb + (d & 0x00ff_00ff) * inv) >> 8) & 0x00ff_00ff;
            let o_ag = ((ag + ((d >> 8) & 0x00ff_00ff) * inv) >> 8) & 0x00ff_00ff;
            *px = o_rb | o_ag << 8;
        }
    }
}

/// How opaque the scrim is. The reference's is `rgba(6, 6, 7, .5)` over a 3 px blur, which smears the terminal's
/// text into a faint haze; without the blur half would leave that text sharp and legible behind the dialog. Over
/// the glass, which is almost the scrim's colour, a deeper scrim takes the text down as far and leaves the rest
/// as it was, for a fill instead of a blur of the whole window at every frame of an open dialog.
const SCRIM_A: f32 = 0.78;

/// The dimmed backdrop of a modal: everything under the title bar (the grid and the status bar), with the
/// window's rounded bottom corners. The title bar stays as it is, so the window can still be moved and closed.
pub fn scrim(p: &mut Paint, g: Geo) {
    let r = if g.square { 0.0 } else { chrome::RADIUS };
    let (ww, wh) = (g.w as f32, g.h as f32);
    let top = TITLE_H as i32;
    // The rows above the corners are a plain fill; only the last rows need the corners' coverage.
    let edge = (g.h - r.ceil() as i32).max(top);
    fill(p, 0, top, g.w, edge - top, SCRIM, SCRIM_A);
    p.shape(0, edge, g.w, g.h, SCRIM, SCRIM_A, &|x, y| if y < wh - r { 1.0 } else { (0.5 - chrome::rounded_rect(x, y, ww, wh, r)).clamp(0.0, 1.0) });
}

/// How far a shadow reaches, how far down it is cast and how dark it is at most.
#[derive(Clone, Copy, Debug)]
pub struct Shadow {
    pub reach: f32,
    pub dy: f32,
    pub a: f32,
}

/// A dialog or palette: wide and soft.
pub const SHADOW_CARD: Shadow = Shadow { reach: 30.0, dy: 8.0, a: 0.45 };
/// A menu or toast: tighter.
pub const SHADOW_POP: Shadow = Shadow { reach: 18.0, dy: 8.0, a: 0.45 };

/// A glass card at `r`: its shadow, the body in `fill`, the outline at `outline` alpha of white and the light
/// inside the top edge. The straight parts (the body's inside, the outline along the sides) are filled
/// directly; the per-pixel coverage runs only on the shadow, the corners and the rows at the top and bottom.
pub fn card(p: &mut Paint, (x, y, w, h): Rect, radius: f32, fill_c: [f32; 3], outline: f32, sh: Shadow) {
    let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
    let r = radius.ceil() as i32;
    let reach = sh.reach as i32;
    let shadow = |px: f32, py: f32| {
        let d = chrome::rounded_rect(px - fx, py - fy - sh.dy, fw, fh, radius);
        let t = ((d + 6.0) / sh.reach).clamp(0.0, 1.0);
        (1.0 - t).powi(3) * (0.5 + 0.5 * (d / 6.0).clamp(0.0, 1.0)).min(1.0)
    };
    let body = |px: f32, py: f32| (0.5 - chrome::rounded_rect(px - fx, py - fy, fw, fh, radius)).clamp(0.0, 1.0);
    let line = |px: f32, py: f32| {
        let d = chrome::rounded_rect(px - fx, py - fy, fw, fh, radius);
        (0.5 - d).clamp(0.0, 1.0) * (1.5 + d).clamp(0.0, 1.0)
    };
    let (sx0, sy0, sx1, sy1) = (x - reach, y - reach + 8, x + w + reach, y + h + reach + sh.dy as i32 + 8);
    if w < 2 * r + 2 || h < 2 * r + 2 {
        // Too small to have a straight inside: everything by coverage.
        p.shape(sx0, sy0, sx1, sy1, BLACK, sh.a, &shadow);
        p.shape(x, y, x + w, y + h, fill_c, 1.0, &body);
        p.shape(x, y, x + w, y + h, WHITE, outline, &line);
    } else {
        // The shadow, but not under the body's straight inside, which covers it.
        p.shape(sx0, sy0, sx1, y + r, BLACK, sh.a, &shadow);
        p.shape(sx0, y + r, x + r, y + h - r, BLACK, sh.a, &shadow);
        p.shape(x + w - r, y + r, sx1, y + h - r, BLACK, sh.a, &shadow);
        p.shape(sx0, y + h - r, sx1, sy1, BLACK, sh.a, &shadow);
        // The body: the straight parts, then the four corner squares.
        fill(p, x + r, y, w - 2 * r, h, fill_c, 1.0);
        fill(p, x, y + r, r, h - 2 * r, fill_c, 1.0);
        fill(p, x + w - r, y + r, r, h - 2 * r, fill_c, 1.0);
        for (cx, cy) in [(x, y), (x + w - r, y), (x, y + h - r), (x + w - r, y + h - r)] {
            p.shape(cx, cy, cx + r, cy + r, fill_c, 1.0, &body);
        }
        // The outline: the rows at the top and bottom, and two columns along each side.
        p.shape(x, y, x + w, y + r, WHITE, outline, &line);
        p.shape(x, y + h - r, x + w, y + h, WHITE, outline, &line);
        p.shape(x, y + r, x + 2, y + h - r, WHITE, outline, &line);
        p.shape(x + w - 2, y + r, x + w, y + h - r, WHITE, outline, &line);
    }
    p.shape(x, y, x + w, y + 2, WHITE, 0.05, &|px, py| if py - fy >= 1.0 { body(px, py + 1.0) } else { 0.0 });
}

/// A 3 px ring around the rounded rectangle `r`: the focus glow of a field.
pub fn ring(p: &mut Paint, (x, y, w, h): Rect, radius: f32, width: f32, c: [f32; 3], a: f32) {
    let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
    let m = width.ceil() as i32 + 1;
    p.shape(x - m, y - m, x + w + m, y + h + m, c, a, &|px, py| {
        let d = chrome::rounded_rect(px - fx, py - fy, fw, fh, radius);
        (0.5 - (d - width)).clamp(0.0, 1.0) * (d + 0.5).clamp(0.0, 1.0)
    });
}

/// The frame of a dialog: a card of `w` x `h` pixels, centred under the title bar or `top` pixels from the
/// window's top, over a scrim, rising and fading in over 0.14 s. The overlay sets `h` when its contents change.
#[derive(Clone, Copy, Debug)]
pub struct Modal {
    pub fade: Fade,
    pub w: i32,
    pub h: i32,
    pub top: Option<i32>,
}

/// The gap an overlay keeps to the window's edges.
pub const EDGE: i32 = 16;
/// How far the card rises while it appears.
const RISE: f32 = 6.0;
const CARD_RADIUS: f32 = 16.0;

impl Modal {
    pub fn new(w: i32, h: i32) -> Modal {
        Modal { fade: Fade::rising(0.14), w, h, top: None }
    }

    /// Aligned `y` pixels below the window's top (the palette: 60) instead of centred.
    pub fn at_top(mut self, y: i32) -> Modal {
        self.top = Some(y);
        self
    }

    /// The card, clamped to the window: never wider than the window less its margins, never taller than what
    /// is left below the title bar.
    pub fn rect(&self, g: Geo) -> Rect {
        let w = self.w.min(g.w - 2 * EDGE).max(0);
        let (min_y, max_y) = (TITLE_H as i32 + 8, g.h - EDGE);
        let h = self.h.min(max_y - min_y).max(0);
        let y = match self.top {
            Some(t) => t.clamp(min_y, (max_y - h).max(min_y)),
            None => (TITLE_H as i32 + (g.h - TITLE_H as i32 - h) / 2).clamp(min_y, (max_y - h).max(min_y)),
        };
        ((g.w - w) / 2, y, w, h.min(max_y - y).max(0))
    }

    pub fn contains(&self, g: Geo, x: f32, y: f32) -> bool {
        inside(self.rect(g), x, y)
    }

    /// Sets the fade, paints the scrim and the card, and returns the card's rectangle as drawn (a little
    /// lower while it rises): what the overlay paints its contents in.
    pub fn paint(&self, p: &mut Paint, g: Geo) -> Rect {
        let e = self.fade.value();
        p.fade = e;
        scrim(p, g);
        let (x, y, w, h) = self.rect(g);
        let r = (x, y + ((1.0 - e) * RISE).round() as i32, w, h);
        card(p, r, CARD_RADIUS, CARD, 0.10, SHADOW_CARD);
        r
    }
}

// -------------------------------------------------------------- key caps --

/// What a key cap says.
#[derive(Clone, Copy, Debug)]
pub enum Cap<'a> {
    Text(&'a str),
    Icon(Icon),
}

/// A key cap's height, its side padding and the gap between caps in a row of them.
pub const KBD_H: i32 = 17;
const KBD_PAD: i32 = 5;
pub const KBD_GAP: i32 = 3;

pub fn kbd_width(t: &Text, cap: Cap) -> i32 {
    2 + 2 * KBD_PAD
        + match cap {
            Cap::Text(s) => t.width(s, PX_KBD),
            Cap::Icon(_) => 9,
        }
}

/// A key cap with its left edge at `x`, centred on `cy`: 4 % fill, 7 % outline, a thicker foot. Returns its width.
pub fn kbd(p: &mut Paint, t: &Text, x: i32, cy: i32, cap: Cap, a: f32) -> i32 {
    let w = kbd_width(t, cap);
    let y = cy - KBD_H / 2;
    p.rrect(x, y, w, KBD_H, 4.0, WHITE, 0.04 * a);
    p.outline(x, y, w, KBD_H, 4.0, WHITE, 0.07 * a);
    p.shape(x + 2, y + KBD_H - 2, x + w - 2, y + KBD_H - 1, WHITE, 0.07 * a, &|_, _| 1.0);
    match cap {
        Cap::Text(s) => t.draw(p, x + 1 + KBD_PAD, cy - 1, s, Ink::new(PX_KBD, fg()).alpha(a)),
        Cap::Icon(i) => icons::paint(p, i, x as f32 + w as f32 / 2.0, cy as f32 - 1.0, 10.0, 1.7, fg(), a),
    }
    w
}

/// The caps of a shortcut written `Alt+N` or `Ctrl+Space`: one cap per key, right-aligned to `right`.
/// Returns the left edge of the row.
pub fn kbd_row(p: &mut Paint, t: &Text, right: i32, cy: i32, shortcut: &str, a: f32) -> i32 {
    let keys = || shortcut.split('+').filter(|k| !k.is_empty());
    let total: i32 = keys().map(|k| kbd_width(t, Cap::Text(k))).sum::<i32>() + KBD_GAP * keys().count().saturating_sub(1) as i32;
    let mut x = right - total;
    for k in keys() {
        x += kbd(p, t, x, cy, Cap::Text(k), a) + KBD_GAP;
    }
    right - total
}

// ----------------------------------------------------------------- buttons --

/// A button's height and its side padding (`.btn`: 34 px, 0 16px).
pub const BTN_H: i32 = 34;
const BTN_PAD: i32 = 16;

pub fn button_width(t: &Text, label: &str) -> i32 {
    t.width(label, PX) + 2 * BTN_PAD
}

/// A button after the reference's `.btn`: 7 % white (12 % under the pointer), radius 9, its label medium; the
/// primary one is `#f4f4f5` with dark text (white under the pointer).
pub fn button(p: &mut Paint, t: &Text, (x, y, w, h): Rect, label: &str, primary: bool, lit: bool) {
    if primary {
        p.rrect(x, y, w, h, 9.0, [if lit { 1.0 } else { 0.957 }; 3], 1.0);
    } else {
        p.rrect(x, y, w, h, 9.0, WHITE, if lit { 0.12 } else { 0.07 });
    }
    let ink = if primary { [0.067, 0.067, 0.075] } else { fg() };
    t.draw(p, x + (w - t.width(label, PX)) / 2, y + h / 2, label, Ink::new(PX, ink).medium());
}

/// Counts presses that follow each other closely: one, two (a word), three (everything).
#[derive(Clone, Copy, Debug, Default)]
pub struct Clicks {
    time: u32,
    n: u8,
}

/// Two presses closer than this many milliseconds are a double click.
pub const MULTI_CLICK_MS: u32 = 350;

impl Clicks {
    /// A press at `time` (the compositor's millisecond clock): how many in a row this is, 1 to 3.
    pub fn press(&mut self, time: u32) -> u8 {
        self.n = if self.n > 0 && time.wrapping_sub(self.time) < MULTI_CLICK_MS { self.n % 3 + 1 } else { 1 };
        self.time = time;
        self.n
    }
}

// ----------------------------------------------------------------- tests --

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;
    use crate::input::sym;

    #[test]
    fn the_frame_gives_back_what_the_overlays_painted_before_and_what_they_paint_now() {
        let (a, b) = (Extent::Part((10, 20, 30, 40)), Extent::Part((50, 10, 20, 20)));
        assert_eq!(Extent::Nothing.to_restore(Extent::Nothing), None);
        assert_eq!(Extent::Nothing.to_restore(a), None, "the first frame paints over a clean one");
        assert_eq!(a.to_restore(Extent::Nothing), Some(a), "a toast that went");
        assert_eq!(a.to_restore(b), Some(Extent::Part((10, 10, 60, 50))), "one that moved: both boxes");
        for (was, now) in [(Extent::Window, Extent::Window), (Extent::Window, Extent::Nothing), (a, Extent::Window), (Extent::Nothing, Extent::Window)] {
            assert_eq!(was.to_restore(now), Some(Extent::Window), "{was:?} {now:?}");
        }
    }

    #[test]
    fn a_stack_paints_over_the_window_while_one_is_open_and_over_the_toast_alone_otherwise() {
        let g = Geo { w: 900, h: 620, square: false };
        let mut s = Stack::default();
        assert_eq!(s.extent(g), Extent::Nothing);
        s.toast("hi", None, 0, Instant::now());
        let Extent::Part((x, y, w, h)) = s.extent(g) else { panic!("a toast has a box") };
        assert!(x > 0 && x + w < g.w && y > TITLE_H as i32 && y + h <= g.h, "{:?}", (x, y, w, h));
        s.open(7, Box::new(Probe { log: Rc::default(), on_key: Out::None, card: (0, 0, 1, 1), esc: false }));
        assert_eq!(s.extent(g), Extent::Window);
    }

    const G: Geo = Geo { w: 900, h: 620, square: false };

    #[test]
    fn text_is_cut_with_an_ellipsis_to_fit() {
        let w = |s: &str| s.chars().count() as i32 * 8;
        assert_eq!(fit("short", 100, w), "short");
        assert_eq!(fit("a very long title", 80, w), "a very lo…");
        assert!(w(&fit("a very long title", 80, w)) <= 80);
        assert_eq!(fit("abc", 8, w), "…");
        assert_eq!(fit("", 0, w), "");
        // Halving finds what adding a character at a time did: the longest prefix that fits with the ellipsis.
        for max in 0..200 {
            let slow = {
                let mut out = String::new();
                for c in "a very long title with ünïcode".chars() {
                    out.push(c);
                    if w(&out) + w("…") > max {
                        out.pop();
                        break;
                    }
                }
                out.push('…');
                out
            };
            let got = fit("a very long title with ünïcode", max, w);
            if w("a very long title with ünïcode") > max {
                assert_eq!(got, slow, "max {max}");
            }
        }
    }

    #[test]
    fn numbers_are_written_in_a_buffer() {
        let mut b = [0u8; 20];
        for n in [0, 7, 10, 4096, usize::MAX] {
            assert_eq!(dec(n, &mut b), n.to_string());
        }
    }

    #[test]
    fn a_fade_runs_from_nothing_to_everything_and_then_reports_rest() {
        let t0 = Instant::now();
        let mut f = Fade::rising(0.16);
        assert_eq!(f.value(), 0.0);
        assert!(f.step(t0), "the first frame is the invisible one");
        assert_eq!(f.value(), 0.0);
        assert!(f.step(t0 + std::time::Duration::from_millis(80)));
        assert!(f.value() > 0.3 && f.value() < 0.7, "{}", f.value());
        assert!(!f.step(t0 + std::time::Duration::from_millis(400)));
        assert_eq!(f.value(), 1.0);
        assert!(!Fade::full().animating());
    }

    #[test]
    fn a_modal_card_is_centred_clamped_and_can_hang_from_the_top() {
        let m = Modal::new(500, 300);
        let (x, y, w, h) = m.rect(G);
        assert_eq!((w, h), (500, 300));
        assert_eq!(x, 200);
        // Centred in what is under the title bar.
        assert_eq!(y, TITLE_H as i32 + (620 - TITLE_H as i32 - 300) / 2);
        // The palette: 60 px from the top, wide at most the window less its margins.
        let p = Modal::new(580, 470).at_top(60);
        assert_eq!(p.rect(G).1, 60);
        let narrow = Geo { w: 400, ..G };
        assert_eq!(p.rect(narrow).2, 400 - 2 * EDGE);
        // A short window: the card shrinks to fit under the bar and above the bottom margin.
        let short = Geo { h: 300, ..G };
        let (_, y, _, h) = p.rect(short);
        assert!(y >= TITLE_H as i32 && y + h <= 300 - EDGE, "{y} {h}");
        assert!(p.contains(G, 450.0, 100.0) && !p.contains(G, 450.0, 30.0));
    }

    #[test]
    fn the_scrim_dims_everything_under_the_title_bar_and_nothing_above() {
        let mut fb = vec![0u32; 900 * 620];
        let mut p = Paint { fb: &mut fb, w: 900, h: 620, ox: 0, oy: 0, fade: 1.0 };
        scrim(&mut p, G);
        assert_eq!(fb[(TITLE_H - 1) * 900 + 450], 0, "the title bar is left alone");
        assert_ne!(fb[(TITLE_H + 1) * 900 + 450], 0);
        // Rounded bottom corners: the corner pixel is untouched, the middle of the bottom edge is dimmed.
        assert_eq!(fb[619 * 900], 0);
        assert_ne!(fb[619 * 900 + 450], 0);
    }

    #[test]
    fn a_card_paints_inside_its_shadow_and_fades_with_the_paint() {
        let ink = |fade: f32| {
            let mut fb = vec![0u32; 400 * 300];
            let mut p = Paint { fb: &mut fb, w: 400, h: 300, ox: 0, oy: 0, fade };
            card(&mut p, (100, 80, 200, 120), 16.0, CARD, 0.10, SHADOW_CARD);
            fb[140 * 400 + 200]
        };
        assert_eq!(ink(1.0) >> 24, 255, "an opaque body");
        assert_eq!(ink(0.0), 0);
        let half = ink(0.5) >> 24;
        assert!(half > 100 && half < 160, "{half}");
    }

    #[test]
    fn a_fill_blends_like_put_stores_when_opaque_and_stays_inside_the_buffer() {
        let start = 0xc010_2030u32;
        let (c, w, h) = ([1.0, 0.0, 0.5], 12usize, 6usize);
        for (a, fade) in [(0.5, 1.0), (0.25, 1.0), (1.0, 0.5), (0.9, 0.9), (1.0, 1.0)] {
            let mut fast = vec![start; w * h];
            let mut slow = vec![start; w * h];
            fill(&mut Paint { fb: &mut fast, w, h, ox: 2, oy: 1, fade }, 1, 1, 5, 3, c, a);
            let mut p = Paint { fb: &mut slow, w, h, ox: 2, oy: 1, fade };
            p.shape(1, 1, 6, 4, c, a, &|_, _| 1.0);
            for (i, (f, s)) in fast.iter().zip(&slow).enumerate() {
                for shift in [0, 8, 16, 24] {
                    let d = (f >> shift & 255).abs_diff(s >> shift & 255);
                    assert!(d <= 1, "alpha {a} fade {fade}: pixel {i} channel {shift}: {f:#x} vs {s:#x}");
                }
            }
            // Only the rectangle (window 1..6 x 1..4, shifted by the origin) was touched.
            assert_eq!(fast.iter().filter(|&&px| px != start).count(), 15);
        }
        // Opaque is exactly the colour; a rectangle partly or wholly outside is cut, not wrapped or a panic.
        let mut fb = vec![start; w * h];
        let mut p = Paint { fb: &mut fb, w, h, ox: 0, oy: 0, fade: 1.0 };
        fill(&mut p, -3, -3, 5, 5, [0.0, 1.0, 0.0], 1.0);
        fill(&mut p, 10, 4, 50, 50, [0.0, 1.0, 0.0], 0.5);
        fill(&mut p, 100, 100, 5, 5, [0.0, 1.0, 0.0], 1.0);
        fill(&mut p, 0, 0, 0, 5, [0.0, 1.0, 0.0], 1.0);
        assert_eq!(fb[0], 0xff00_ff00);
        assert_eq!(fb[2 * w + 2], start, "2 px in and 2 down is already outside the 2x2 that was inside");
        assert_ne!(fb[5 * w + 11], start);
    }

    #[test]
    fn presses_close_together_count_up_to_three_and_start_again() {
        let mut c = Clicks::default();
        assert_eq!(c.press(1000), 1);
        assert_eq!(c.press(1200), 2);
        assert_eq!(c.press(1400), 3);
        assert_eq!(c.press(1600), 1, "a fourth is a new first");
        assert_eq!(c.press(3000), 1, "too late");
        assert_eq!(c.press(u32::MAX - 100), 1);
        assert_eq!(c.press(100), 2, "the clock wraps");
    }

    #[test]
    fn buttons_are_as_wide_as_their_label_and_the_primary_one_is_light() {
        let t = Text::load();
        assert_eq!(button_width(&t, "OK"), t.width("OK", PX) + 32);
        let render = |primary: bool, lit: bool| {
            let mut fb = vec![0u32; 200 * 50];
            let mut p = Paint { fb: &mut fb, w: 200, h: 50, ox: 0, oy: 0, fade: 1.0 };
            button(&mut p, &t, (10, 8, 120, BTN_H), "Create", primary, lit);
            fb[20 * 200 + 14]
        };
        assert!(render(true, false) >> 8 & 255 > 200, "light fill");
        assert!(render(false, false) >> 8 & 255 < 60 && render(false, false) >> 24 != 0, "a faint fill");
        assert!(render(false, true) >> 24 > render(false, false) >> 24, "lighter under the pointer");
    }

    #[test]
    fn key_caps_are_one_per_key_and_end_where_they_were_told() {
        let t = Text::load();
        let mut fb = vec![0u32; 300 * 40];
        let mut p = Paint { fb: &mut fb, w: 300, h: 40, ox: 0, oy: 0, fade: 1.0 };
        let left = kbd_row(&mut p, &t, 280, 20, "Ctrl+Space", 1.0);
        let one = kbd_row(&mut p, &t, 280, 20, "Alt", 1.0);
        assert!(left < one && one < 280);
        assert_eq!(280 - one, kbd_width(&t, Cap::Text("Alt")));
        assert!(fb.iter().any(|&px| px >> 24 != 0));
    }

    /// An overlay that writes down what it was told and answers as it was set up to.
    struct Probe {
        log: Rc<RefCell<Vec<String>>>,
        on_key: Out,
        card: Rect,
        esc: bool,
    }

    impl Overlay for Probe {
        fn key(&mut self, k: &Key, _: &Cx) -> Out {
            self.log.borrow_mut().push(format!("key {:#x}", k.sym));
            std::mem::replace(&mut self.on_key, Out::None)
        }
        fn paste(&mut self, text: &str, _: &Cx) -> Out {
            self.log.borrow_mut().push(format!("paste {text}"));
            Out::Redraw
        }
        fn pointer(&mut self, x: f32, y: f32, _: &Cx) -> Out {
            self.log.borrow_mut().push(format!("move {x},{y}"));
            Out::Redraw
        }
        fn button(&mut self, c: Click, _: &Cx) -> Out {
            self.log.borrow_mut().push(format!("button {} {}", c.pressed, c.x));
            if c.pressed { Out::Run(7) } else { Out::None }
        }
        fn scroll(&mut self, dy: f32, _: f32, _: f32, _: &Cx) -> Out {
            self.log.borrow_mut().push(format!("scroll {dy}"));
            Out::Redraw
        }
        fn contains(&self, x: f32, y: f32, _: &Cx) -> bool {
            inside(self.card, x, y)
        }
        fn draw(&self, _: &mut Paint, _: &Cx) {
            self.log.borrow_mut().push("draw".into());
        }
        fn handles_escape(&self) -> bool {
            self.esc
        }
    }

    struct Clip(Option<String>, Vec<String>);

    impl Clipboard for Clip {
        fn get(&mut self) -> Option<String> {
            self.0.clone()
        }
        fn set(&mut self, text: &str) {
            self.1.push(text.into());
        }
    }

    fn key(sym: u32) -> Key {
        Key { sym, ..Key::default() }
    }

    fn open(on_key: Out, esc: bool) -> (Stack, Rc<RefCell<Vec<String>>>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut s = Stack::default();
        s.open(1, Box::new(Probe { log: log.clone(), on_key, card: (100, 100, 300, 200), esc }));
        (s, log)
    }

    #[test]
    fn with_nothing_open_nothing_is_taken() {
        let mut s = Stack::default();
        let mut clip = Clip(None, vec![]);
        assert!(!s.is_open() && s.is_empty());
        assert_eq!(s.key(&key(0x61), G, &mut clip), Reply::default());
        assert_eq!(s.pointer(1.0, 1.0, G), Reply::default());
        assert_eq!(s.button(Click { x: 1.0, y: 1.0, pressed: true, time: 0 }, G), Reply::default());
        assert_eq!(s.scroll(10.0, 1.0, 1.0, G), Reply::default());
        assert!(!s.draw(&mut [], (0, 0), (0, 0), G, Instant::now()));
    }

    /// An overlay the app can reach by its type, that paints through the app's grid.
    struct Typed {
        n: u32,
    }

    impl Overlay for Typed {
        fn key(&mut self, _: &Key, _: &Cx) -> Out {
            Out::None
        }
        fn draw(&self, _: &mut Paint, _: &Cx) {}
        fn draw_grid(&self, p: &mut Paint, grid: &mut dyn Grid, _: &Cx) {
            let mut rows = String::new();
            let running = grid.tail(self.n, 3, &mut rows);
            grid.text(p, 4, 5, &format!("{rows}{running}"), 0.75, [1.0; 3]);
        }
        fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
            Some(self)
        }
    }

    /// A grid that records what it was asked and says one row of text is running.
    #[derive(Default)]
    struct Spy(Vec<String>);

    impl Grid for Spy {
        fn tail(&mut self, id: u32, rows: usize, out: &mut String) -> bool {
            self.0.push(format!("tail {id} {rows}"));
            out.push_str("row");
            true
        }
        fn text(&mut self, _: &mut Paint, x: i32, cy: i32, s: &str, scale: f32, _: [f32; 3]) {
            self.0.push(format!("text {x},{cy} {s} {scale}"));
        }
        fn metrics(&self, _: f32) -> (i32, i32) {
            (7, 16)
        }
    }

    #[test]
    fn an_act_runs_the_command_and_keeps_the_overlay_open() {
        let (mut s, _) = open(Out::Act(9), false);
        let mut clip = Clip(None, vec![]);
        let r = s.key(&key(0x61), G, &mut clip);
        assert_eq!(r, Reply { taken: true, redraw: true, ran: Some(Ran { tag: 1, id: 9 }), ..Reply::default() });
        assert!(s.is_open() && s.has(1), "unlike Run, the overlay stays");
        let (mut s, _) = open(Out::Run(9), false);
        let r = s.key(&key(0x61), G, &mut clip);
        assert!(r.closed && !s.is_open(), "Run closes it");
    }

    #[test]
    fn the_app_reaches_an_open_overlay_by_its_tag_and_type_and_the_top_one_is_known() {
        let mut s = Stack::default();
        assert_eq!(s.top(), None);
        s.open(5, Box::new(Typed { n: 1 }));
        let (log, _) = (Rc::new(RefCell::new(Vec::new())), ());
        s.open(6, Box::new(Probe { log, on_key: Out::None, card: (0, 0, 1, 1), esc: false }));
        assert_eq!(s.top(), Some(6));
        s.with::<Typed>(5).expect("the overlay under tag 5 is a Typed").n = 42;
        assert_eq!(s.with::<Typed>(5).map(|t| t.n), Some(42));
        assert!(s.with::<Typed>(6).is_none(), "the probe does not offer itself, and is not a Typed");
        assert!(s.with::<Typed>(7).is_none(), "nothing under tag 7");
        assert!(s.with::<Probe>(5).is_none(), "the wrong type");
        s.close(6);
        assert_eq!(s.top(), Some(5));
    }

    #[test]
    fn the_grid_is_handed_to_each_overlay_after_it_is_drawn() {
        let mut s = Stack::default();
        s.open(5, Box::new(Typed { n: 7 }));
        let mut fb = vec![0u32; 20 * 20];
        let mut spy = Spy::default();
        s.draw_with(&mut fb, (20, 20), (0, 0), G, Instant::now(), &mut spy);
        assert_eq!(spy.0, ["tail 7 3", "text 4,5 rowtrue 0.75"]);
        // Plain `draw` has no terminal: an overlay that asks gets nothing and draws nothing.
        s.draw(&mut fb, (20, 20), (0, 0), G, Instant::now());
        assert_eq!(spy.0.len(), 2);
        let mut none = NoGrid;
        let mut rows = String::new();
        assert!(!none.tail(1, 3, &mut rows) && rows.is_empty());
        assert!(none.metrics(0.75).0 > 0 && none.metrics(0.75).1 > 0);
    }

    #[test]
    fn an_open_overlay_takes_every_key_and_escape_closes_it() {
        let (mut s, log) = open(Out::None, false);
        let mut clip = Clip(None, vec![]);
        let r = s.key(&key(0x61), G, &mut clip);
        assert!(r.taken && !r.closed && !r.redraw);
        assert_eq!(log.borrow().as_slice(), ["key 0x61"]);
        let r = s.key(&key(sym::ESCAPE), G, &mut clip);
        assert!(r.taken && r.closed && r.ran.is_none());
        assert_eq!(log.borrow().len(), 1, "the overlay never saw the Esc");
        assert!(!s.is_open() && s.is_empty(), "and the font went with it");
        assert_eq!(s.key(&key(0x61), G, &mut clip), Reply::default(), "the next key is the terminal's");
    }

    #[test]
    fn an_overlay_with_its_own_escape_gets_it() {
        let (mut s, log) = open(Out::Redraw, true);
        let r = s.key(&key(sym::ESCAPE), G, &mut Clip(None, vec![]));
        assert!(r.taken && r.redraw && !r.closed);
        assert_eq!(log.borrow().as_slice(), ["key 0xff1b"]);
        assert!(s.is_open());
    }

    #[test]
    fn running_a_command_closes_the_overlay_and_names_who_ran_it() {
        let (mut s, _) = open(Out::Run(42), false);
        let r = s.key(&key(sym::RETURN), G, &mut Clip(None, vec![]));
        assert_eq!(r.ran, Some(Ran { tag: 1, id: 42 }));
        assert!(r.closed && !s.is_open());
    }

    #[test]
    fn a_paste_reads_the_clipboard_only_when_asked_and_hands_the_text_in() {
        let (mut s, log) = open(Out::Paste, false);
        let mut clip = Clip(Some("hello".into()), vec![]);
        let r = s.key(&key(0x76), G, &mut clip);
        assert!(r.taken && r.redraw);
        assert_eq!(log.borrow().as_slice(), ["key 0x76", "paste hello"]);
        // An empty clipboard: nothing is pasted.
        let (mut s, log) = open(Out::Paste, false);
        let r = s.key(&key(0x76), G, &mut Clip(None, vec![]));
        assert!(r.taken && !r.redraw);
        assert_eq!(log.borrow().len(), 1);
        // A copy goes to the clipboard.
        let (mut s, _) = open(Out::Copy("sel".into()), false);
        let mut clip = Clip(None, vec![]);
        s.key(&key(0x63), G, &mut clip);
        assert_eq!(clip.1, ["sel"]);
    }

    #[test]
    fn a_press_outside_closes_and_the_release_is_swallowed_a_press_inside_is_the_overlays() {
        let (mut s, log) = open(Out::None, false);
        let r = s.button(Click { x: 10.0, y: 10.0, pressed: true, time: 1 }, G);
        assert!(r.taken && r.closed);
        assert!(!s.is_open());
        // A release with nothing open belongs to the app.
        assert_eq!(s.button(Click { x: 10.0, y: 10.0, pressed: false, time: 2 }, G), Reply::default());
        assert!(log.borrow().is_empty());
        // Inside: the overlay hears it, and its answer closes it.
        let (mut s, log) = open(Out::None, false);
        let r = s.button(Click { x: 150.0, y: 150.0, pressed: true, time: 1 }, G);
        assert_eq!(r.ran, Some(Ran { tag: 1, id: 7 }));
        assert_eq!(log.borrow().as_slice(), ["button true 150"]);
        // A release outside while open is taken, and nothing closes.
        let (mut s, _) = open(Out::None, false);
        let r = s.button(Click { x: 10.0, y: 10.0, pressed: false, time: 1 }, G);
        assert!(r.taken && !r.closed && s.is_open());
    }

    #[test]
    fn pointer_and_wheel_reach_the_top_overlay_and_the_wheel_only_inside_it() {
        let (mut s, log) = open(Out::None, false);
        let r = s.pointer(120.0, 130.0, G);
        assert!(r.taken && r.redraw && r.cursor == Some(Cursor::Default));
        let r = s.scroll(48.0, 150.0, 150.0, G);
        assert!(r.redraw);
        let r = s.scroll(48.0, 5.0, 5.0, G);
        assert!(r.taken && !r.redraw, "the wheel over the scrim does nothing");
        assert_eq!(log.borrow().as_slice(), ["move 120,130", "scroll 48"]);
    }

    #[test]
    fn the_top_of_the_stack_has_the_input_and_the_one_below_waits() {
        let la = Rc::new(RefCell::new(Vec::new()));
        let lb = Rc::new(RefCell::new(Vec::new()));
        let mut s = Stack::default();
        let probe = |log: &Rc<RefCell<Vec<String>>>| Box::new(Probe { log: log.clone(), on_key: Out::None, card: (0, 0, 900, 620), esc: false });
        s.open(1, probe(&la));
        s.open(2, probe(&lb));
        let mut clip = Clip(None, vec![]);
        s.key(&key(0x61), G, &mut clip);
        assert!(la.borrow().is_empty() && lb.borrow().len() == 1);
        s.key(&key(sym::ESCAPE), G, &mut clip);
        assert!(s.is_open() && s.has(1) && !s.has(2));
        s.key(&key(0x62), G, &mut clip);
        assert_eq!(la.borrow().as_slice(), ["key 0x62"]);
        // Opening again under the same tag replaces it.
        s.open(1, probe(&lb));
        s.key(&key(0x63), G, &mut clip);
        assert_eq!(la.borrow().len(), 1);
    }

    #[test]
    fn drawing_goes_bottom_to_top_and_asks_for_frames_only_while_something_animates() {
        let (mut s, log) = open(Out::None, false);
        let mut fb = vec![0u32; 10 * 10];
        assert!(!s.draw(&mut fb, (10, 10), (0, 0), G, Instant::now()), "the probe does not animate");
        assert_eq!(log.borrow().as_slice(), ["draw"]);
        let now = Instant::now();
        s.toast("hello", None, 1, now);
        assert!(s.draw(&mut fb, (10, 10), (0, 0), G, now), "a toast fading in wants frames");
        assert!(!s.is_empty());
    }

    #[test]
    fn the_toast_is_not_an_overlay_it_takes_neither_keys_nor_the_pointer_away_from_the_grid() {
        let mut s = Stack::default();
        let now = Instant::now();
        s.toast("Window closed", Some("Undo"), 9, now);
        assert!(!s.is_open() && !s.is_empty());
        let mut clip = Clip(None, vec![]);
        assert_eq!(s.key(&key(0x61), G, &mut clip), Reply::default(), "a toast never has the keyboard");
        assert!(!s.pointer(5.0, 5.0, G).taken);
        let at = |ms: u64| now + std::time::Duration::from_millis(ms);
        assert_eq!(s.deadline(now), Some(at(6250)), "the wake that starts the fade out");
        assert_eq!(s.tick(at(3000)), Tick::Rest, "in its stay nothing has to be drawn");
        assert_eq!(s.tick(at(6300)), Tick::Repaint, "fading out");
        assert_eq!(s.deadline(at(6300)), Some(at(6500)));
        assert_eq!(s.tick(at(60_000)), Tick::Gone);
        assert!(s.is_empty() && s.deadline(now).is_none(), "nothing is left to wake the loop");
        assert_eq!(s.tick(at(60_001)), Tick::Rest);
    }
}
