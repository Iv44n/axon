//! The panel of SSH servers ([0048](../knowledge/decisions/workspace/0048-ssh-servers-by-alias.md)), after the reference's
//! `#hosts` (`.prefs.hosts`): docked beside the grid where the settings go (the two share the place, one at a time), not
//! modal. Four views:
//!
//! - **List**: a search box and a line of keys that stay where they are while the servers scroll between them; each server
//!   with its status dot (green with a glow while a window has a session to it), its alias and `user@address:port`, and how
//!   long ago it was used, or "connected"; the selected one and the one under the pointer show two buttons instead: open
//!   in a new window, edit. A click or Enter connects (`Effect::Connect`; the app decides where), `Ctrl+Enter` always in a
//!   new window. The footer has *More* (a menu: import, export, copy the file) and *+ Server*. The first `Esc` clears the
//!   search, the second closes the panel.
//! - **Edit** (*New server* or the server's alias): a field to paste a long ssh command into, which fills the others and
//!   proposes a name (new servers only), alias, address, user and port, key, what to run on arrival, the command it amounts
//!   to, and what is wrong (errors once the form was touched, advice always). *Delete* at the left in red, *Duplicate*,
//!   *Cancel*, *Save* (off while something is wrong). Enter in any field saves; `Esc` goes back.
//! - **Import**: what `~/.ssh/config` has (axon reads it; the reference has the user paste it), what it defines and its
//!   mistakes by line, and *Import n* for the servers axon has not got.
//! - **Export**: axon's own file, which `~/.ssh/config` includes, and *Copy*.
//!
//! Like the settings, the panel is a pure function of its state, the servers (`&[Host]`, the app's `ssh::Book`) and its
//! box, painted into its cleared box when something in it changed; the geometry is computed when asked. It never writes
//! a file: what it asks for comes back to the app as an `Effect`. Coordinates are panel-local unless a function says
//! window pixels.
use std::{borrow::Cow, cell::RefCell, time::Instant};

use crate::{
    config::FontCfg,
    field::{self, Edit, Field, H_BOX, Look},
    font::Fonts,
    i18n::{Str, tr},
    icons::{self, Icon},
    input::sym,
    overlay::{Cap, Fade, Ink, PX, PX_HINT, PX_SMALL, PX_TITLE, Text, accent, dim, faint, fg, kbd},
    paint::{Paint, WHITE},
    palette,
    plat::Key,
    ssh::{self, Check, Found, Host, Issue, IssueKind, Level, Problem},
};

type Rect = (i32, i32, i32, i32);

fn inside(x: i32, y: i32, (rx, ry, rw, rh): Rect) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

// ----------------------------------------------------------------- layout --

/// The header: 10 px above its 28 px buttons (`.hosts .dlg-head`).
const HEAD_H: i32 = 38;
const SQ: i32 = 28;
/// `.hosts .dlg-body`: 8 px above and below, 10 at the sides.
const PAD: i32 = 10;
const BODY_TOP: i32 = 8;
const BODY_BOTTOM: i32 = 8;
/// The list: the search box (`.h-search`, 2 px margin, 12 below), then the rows from `LIST_Y`.
const SEARCH_Y: i32 = HEAD_H + BODY_TOP + 2;
const SEARCH_H: i32 = field::H_SEARCH;
const LIST_Y: i32 = SEARCH_Y + SEARCH_H + 12;
/// A server's row: 7 px above and below a 13 px name and an 11.5 px address (`.h-row`).
pub const ROW_H: i32 = 49;
/// The line of keys under the list (`.h-tip`, 8 px above it).
const TIP_H: i32 = 18;
/// The two buttons a row shows at its right (`.h-acts .b-tool`: 26 x 24, 1 px apart, 6 px from the edge).
const TOOL_W: i32 = 26;
const TOOL_H: i32 = 24;
/// A field: 8 px of padding, a 12 px label, 7 px, the 36 px box (`.field`, `.f-input`).
const FIELD_PAD: i32 = 8;
const LABEL_H: i32 = 17;
const LABEL_GAP: i32 = 7;
const FIELD_H: i32 = FIELD_PAD + LABEL_H + LABEL_GAP + H_BOX + FIELD_PAD;
/// The port's column (`.h-two`: the rest, 8 px, 96 px).
const PORT_W: i32 = 96;
/// A hint line (`.h-hint`, 11.5 px at 1.5) and a message line (`.h-msg`, 12 px at 1.5).
const HINT_LINE: i32 = 17;
const MSG_LINE: i32 = 18;
/// The command and the files are in the grid's family at 12 px (`.h-code`, `.h-area`), 1.55 apart.
const MONO_PX: f32 = 12.0;
const MONO_LINE: i32 = 19;
/// The tallest file the import and export views show (lines).
const AREA_MAX: usize = 400;
/// The footer's buttons (`.btn`: 34 px, 16 px each side of the label, 8 apart) and its padding (`.prefs-foot`).
const BTN_H: i32 = 34;
const BTN_PAD: i32 = 16;
const BTN_GAP: i32 = 8;
/// The red of a danger button and of an error (`#ff8d8d`).
const RED: [f32; 3] = [1.0, 0.553, 0.553];
/// A wheel notch, in pixels.
pub const WHEEL_PX: i32 = 48;
/// A window this narrow or narrower puts the footer's buttons two by two (the reference's `@container term (max-width: 480px)`).
pub const NARROW: u32 = 480;

// ------------------------------------------------------------------- state --

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    List,
    Edit,
    Import,
    Export,
}

/// A field of the form, in the order Tab goes through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fld {
    Paste,
    Alias,
    Address,
    User,
    Port,
    Key,
    Cmd,
}

const FIELDS: [Fld; 7] = [Fld::Paste, Fld::Alias, Fld::Address, Fld::User, Fld::Port, Fld::Key, Fld::Cmd];

/// A button of the footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    More,
    Add,
    Delete,
    Duplicate,
    Cancel,
    Save,
    Import,
    Back,
    Copy,
}

/// One of the two buttons a row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Window,
    Edit,
}

/// What is at a point of the panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hit {
    #[default]
    Outside,
    Panel,
    Close,
    Back,
    Search,
    /// The row at this place in the list (not the book's index).
    Row(usize),
    Tool(usize, Tool),
    Field(Fld),
    Btn(Btn),
}

/// Where the keyboard is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Search,
    Field(Fld),
    Btn(Btn),
}

/// What the app has to do after an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Redraw,
    Close,
    /// Connect to the server with this index in the book: where it makes sense, or in a new window.
    Connect { host: usize, window: bool },
    /// Save this server, in place of the one at `at` or as a new one.
    Save { at: Option<usize>, host: Host },
    Delete(usize),
    /// Add these servers (the import's new ones).
    Import(Vec<Host>),
    /// Open the *More* menu with its corner at this window pixel.
    Menu(i32, i32),
    /// Put axon's file on the clipboard and say so.
    CopyConfig,
    /// Put this on the clipboard (a field's selection).
    Clip(String),
    /// Hand the clipboard's text to `paste`.
    Paste,
}

/// What the line under the paste field says.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Note {
    Hint,
    Read(String),
    Bad,
}

pub struct Hosts {
    pub view: View,
    pub focus: Focus,
    /// The keyboard is on the panel. Clicking the terminal takes it away.
    pub focused: bool,
    /// The keyboard moved the focus (Tab): a button with it shows its ring (`:focus-visible`); a click hides it again.
    ring: bool,
    query: Field,
    /// The servers the list shows (indices into the book: the last used first, filtered), the selected one, and whether
    /// a window has a session to each server of the book.
    shown: Vec<usize>,
    sel: usize,
    live: Vec<bool>,
    /// The form: one field per `Fld`, the server being edited (none: a new one), whether it was touched (errors show).
    form: [Field; 7],
    editing: Option<usize>,
    touched: bool,
    note: Note,
    /// The import view: the text read, what it defines and its mistakes, or why it could not be read.
    import_text: String,
    found: Vec<Found>,
    issues: Vec<Issue>,
    import_err: Option<String>,
    /// The export view: axon's file and where it is.
    export_text: String,
    export_path: String,
    /// How far the list (in the list view) or the body (in the others) is scrolled.
    scroll: i32,
    pub hover: Hit,
    fade: Fade,
    rect: Rect,
    hairline_left: bool,
    narrow: bool,
    text: Text,
    /// The grid's family at 12 px, for the command and the files; loaded only while the panel is open.
    mono: Fonts,
    scratch: RefCell<Vec<u8>>,
    local: String,
}

/// How long ago a server was used, as its row says it: "now", "3 min ago", "5 h ago", "yesterday", "4 d ago".
pub fn ago(last: u64, now: u64) -> String {
    if last == 0 {
        return String::new();
    }
    let m = (now.saturating_sub(last) as f64 / 60.0).round() as u64;
    if m < 1 {
        return tr(Str::HostsNow).to_string();
    }
    if m < 60 {
        return tr(Str::HostsMinAgo).replace("{}", &m.to_string());
    }
    let h = (m as f64 / 60.0).round() as u64;
    if h < 24 {
        return tr(Str::HostsHoursAgo).replace("{}", &h.to_string());
    }
    let d = (h as f64 / 24.0).round() as u64;
    if d == 1 { tr(Str::HostsYesterday).to_string() } else { tr(Str::HostsDaysAgo).replace("{}", &d.to_string()) }
}

/// "1 server", "3 servers" and their kin: the singular string for one, the plural with the number for the rest.
fn count(n: usize, one: Str, many: Str) -> String {
    if n == 1 { tr(one).to_string() } else { tr(many).replace("{}", &n.to_string()) }
}

/// What a problem of the form says.
fn problem_text(p: &Problem) -> String {
    match p {
        Problem::NoAlias => tr(Str::HostsNoAlias).into(),
        Problem::BadAlias => tr(Str::HostsBadAlias).into(),
        Problem::TakenAlias(a) => tr(Str::HostsTaken).replace("{}", a),
        Problem::NoAddress => tr(Str::HostsNoAddress).into(),
        Problem::BadAddress => tr(Str::HostsBadAddress).into(),
        Problem::BadPort => tr(Str::HostsBadPort).into(),
        Problem::LongAlias => tr(Str::HostsLongAlias).into(),
        Problem::SameAs(a) => tr(Str::HostsSameAs).replace("{}", a),
    }
}

/// What a mistake of a config file says.
fn issue_text(i: &Issue) -> String {
    let (s, v) = match &i.kind {
        IssueKind::NoValue(k) => (Str::HostsIssueNoValue, k),
        IssueKind::Repeated(a) => (Str::HostsIssueRepeated, a),
        IssueKind::Outside(k) => (Str::HostsIssueOutside, k),
        IssueKind::BadPort(p) => (Str::HostsIssuePort, p),
        IssueKind::Unknown(k) => (Str::HostsIssueUnknown, k),
    };
    tr(s).replace("{}", v)
}

/// The colours a message can have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Fg,
    Dim,
    Faint,
    Err,
    Warn,
}

fn tone(t: Tone) -> [f32; 3] {
    match t {
        Tone::Fg => fg(),
        Tone::Dim => dim(),
        Tone::Faint => faint(),
        Tone::Err => RED,
        Tone::Warn => palette::unit(palette::YELLOW),
    }
}

/// A piece of a view's body, in content coordinates (before scrolling; 0 is the top of the body's padding box).
enum Block {
    Field { f: Fld, x: i32, y: i32, w: i32 },
    /// Lines of text at `x`, `y` (the first line's top), `px` high, `line` apart; the first `bold` bytes of the first line are medium.
    Lines { x: i32, y: i32, px: f32, line: i32, tone: Tone, lines: Vec<String>, bold: usize },
    /// The command the form amounts to (`.h-code`): `$` and its first line, then the rest dim.
    Code { x: i32, y: i32, w: i32, h: i32, lines: Vec<String>, first: usize },
    /// A file (`.h-area`).
    Area { x: i32, y: i32, w: i32, h: i32, export: bool },
}

impl Block {
    fn bottom(&self) -> i32 {
        match self {
            Block::Field { y, .. } => y + FIELD_H,
            Block::Lines { y, line, lines, .. } => y + line * lines.len() as i32,
            Block::Code { y, h, .. } | Block::Area { y, h, .. } => y + h,
        }
    }
}

/// A button of the footer as it is laid out.
struct Button {
    b: Btn,
    label: Cow<'static, str>,
    primary: bool,
    danger: bool,
    off: bool,
    rect: Rect,
}

impl Hosts {
    /// The list view over `hosts`, the search box with the keyboard.
    pub fn new(hosts: &[Host], live: Vec<bool>) -> Hosts {
        let mut h = Hosts {
            view: View::List,
            focus: Focus::Search,
            focused: true,
            ring: false,
            query: Field::new(64, tr(Str::HostsSearch)),
            shown: Vec::new(),
            sel: 0,
            live,
            form: Default::default(),
            editing: None,
            touched: false,
            note: Note::Hint,
            import_text: String::new(),
            found: Vec::new(),
            issues: Vec::new(),
            import_err: None,
            export_text: String::new(),
            export_path: String::new(),
            scroll: 0,
            hover: Hit::Outside,
            fade: Fade::full(),
            rect: (0, 0, crate::settings::PANEL_W, 570),
            hairline_left: true,
            narrow: false,
            text: Text::load(),
            mono: Fonts::new(&FontCfg::sized(MONO_PX)),
            scratch: RefCell::default(),
            local: ssh::local_user(),
        };
        h.sync(hosts, None);
        h
    }

    /// The user an empty user field stands for (the tests and the board fix it).
    #[cfg(test)]
    pub fn with_local(mut self, user: &str) -> Hosts {
        self.local = user.to_string();
        self
    }

    /// The pointer over `h` (the design board's scenes).
    #[cfg(test)]
    pub fn over(&mut self, h: Hit) {
        self.hover = h;
    }

    /// The search says `q` (the design board's scenes).
    #[cfg(test)]
    pub fn search(&mut self, q: &str, hosts: &[Host]) {
        self.query.set(q);
        self.sync(hosts, None);
    }

    /// The body scrolled to its end (the design board's scenes).
    #[cfg(test)]
    pub fn scroll_to_end(&mut self, hosts: &[Host]) {
        self.scroll = self.max_scroll(hosts);
    }

    /// The keyboard is on `f` (the design board's scenes).
    #[cfg(test)]
    pub fn focus_on(&mut self, f: Focus) {
        self.focus = f;
    }

    /// Fades in from nothing, as the panel does when it opens.
    pub fn start_faded(&mut self) {
        self.fade = Fade::rising(0.16);
    }

    pub fn step(&mut self, now: Instant) -> bool {
        self.fade.step(now)
    }

    pub fn animating(&self) -> bool {
        self.fade.animating()
    }

    /// Puts the panel's box (window pixels), says whether it is at the window's left edge (the hairline goes on the grid's
    /// side) and whether the window is narrow (the footer's buttons go two by two).
    pub fn place(&mut self, rect: Rect, left: bool, narrow: bool, hosts: &[Host]) {
        self.rect = rect;
        self.hairline_left = !left;
        self.narrow = narrow;
        self.scroll = self.scroll.clamp(0, self.max_scroll(hosts));
    }

    /// The servers changed, or the sessions did (`live`, when given): the list is made again, the selection kept on the
    /// same server when it is still listed.
    pub fn sync(&mut self, hosts: &[Host], live: Option<Vec<bool>>) {
        if let Some(l) = live {
            self.live = l;
        }
        let was = self.shown.get(self.sel).copied();
        self.shown = ssh::filter(hosts, &ssh::by_recent(hosts), self.query.text());
        self.sel = was.and_then(|h| self.shown.iter().position(|&i| i == h)).unwrap_or(0).min(self.shown.len().saturating_sub(1));
        if let Some(e) = self.editing
            && e >= hosts.len()
        {
            self.editing = None;
        }
        self.scroll = self.scroll.clamp(0, self.max_scroll(hosts));
    }

    /// Whether a session to the server is open in some window.
    pub fn is_live(&self, host: usize) -> bool {
        self.live.get(host).copied().unwrap_or(false)
    }

    /// Back to the list, the search as it was.
    pub fn list(&mut self, hosts: &[Host]) {
        self.view = View::List;
        self.focus = Focus::Search;
        self.scroll = 0;
        self.sync(hosts, None);
    }

    /// The form: for server `at`, or for a new one (`form` fills it: a duplicate, an ssh the shell ran); `touched` shows its
    /// errors from the start.
    pub fn edit(&mut self, hosts: &[Host], at: Option<usize>, form: Option<ssh::Form>, touched: bool) {
        let f = form.unwrap_or_else(|| at.and_then(|i| hosts.get(i)).map(ssh::Form::of).unwrap_or_default());
        let user_ph = if self.local.is_empty() { tr(Str::HostsUser).to_lowercase() } else { self.local.clone() };
        self.form = [
            Field::new(0, tr(Str::HostsPastePh)),
            Field::new(32, tr(Str::HostsAliasPh)).with_text_of(&f.alias),
            Field::new(0, "192.168.1.100").with_text_of(&f.hostname),
            Field::new(0, "").with_placeholder(user_ph).with_text_of(&f.user),
            Field::new(5, "22").with_text_of(&f.port),
            Field::new(0, "~/.ssh/id_ed25519").with_text_of(&f.identity),
            Field::new(0, tr(Str::HostsCmdPh)).with_text_of(&f.cmd),
        ];
        self.view = View::Edit;
        self.editing = at.filter(|&i| i < hosts.len());
        self.touched = touched;
        self.note = Note::Hint;
        self.scroll = 0;
        self.focus = Focus::Field(if self.editing.is_none() && f.alias.is_empty() { Fld::Paste } else { Fld::Alias });
        if self.editing.is_none() && !f.alias.is_empty() {
            self.form[Fld::Alias as usize].select_all();
        }
    }

    /// The import view on `text` (`~/.ssh/config`), or on why it could not be read.
    pub fn import(&mut self, text: Result<String, String>) {
        self.view = View::Import;
        self.scroll = 0;
        self.focus = Focus::Btn(Btn::Import);
        (self.found, self.issues) = (Vec::new(), Vec::new());
        match text {
            Ok(t) => {
                (self.found, self.issues) = ssh::read_config(&t);
                self.import_text = t;
                self.import_err = None;
            }
            Err(e) => {
                self.import_text.clear();
                self.import_err = Some(e);
            }
        }
    }

    /// The export view on axon's file, `text`, kept at `path`.
    pub fn export(&mut self, text: String, path: String) {
        self.view = View::Export;
        self.scroll = 0;
        self.focus = Focus::Btn(Btn::Copy);
        self.export_text = text;
        self.export_path = path;
    }

    /// The servers the import would add: well formed, and with an alias axon has not got.
    fn fresh<'a>(&'a self, hosts: &'a [Host]) -> impl Iterator<Item = &'a Found> + 'a {
        self.found.iter().filter(move |f| !f.bad && !hosts.iter().any(|h| h.alias.eq_ignore_ascii_case(&f.host.alias)))
    }

    /// The form as text.
    fn form(&self) -> ssh::Form {
        let t = |f: Fld| self.form[f as usize].text().to_string();
        ssh::Form { alias: t(Fld::Alias), hostname: t(Fld::Address), user: t(Fld::User), port: t(Fld::Port), identity: t(Fld::Key), cmd: t(Fld::Cmd) }
    }

    fn check(&self, hosts: &[Host]) -> Check {
        ssh::check(&self.form(), hosts, self.editing, &self.local)
    }

    /// What was pasted into the first field fills the others (the reference's `applyPasted`).
    fn apply_paste(&mut self, hosts: &[Host]) {
        let t = self.form[Fld::Paste as usize].text().trim().to_string();
        if t.is_empty() {
            self.note = Note::Hint;
            return;
        }
        let Some(mut f) = ssh::pasted(&t) else {
            self.note = Note::Bad;
            return;
        };
        let alias = self.form[Fld::Alias as usize].text().trim().to_string();
        f.alias = if alias.is_empty() { ssh::suggest_alias(&f, hosts, tr(Str::HostsDefaultAlias)) } else { alias };
        for (fld, v) in [(Fld::Alias, &f.alias), (Fld::Address, &f.hostname), (Fld::User, &f.user), (Fld::Port, &f.port), (Fld::Key, &f.identity)] {
            self.form[fld as usize].set(v);
        }
        let user = if f.user.is_empty() { &self.local } else { &f.user };
        let mut read = if user.is_empty() { f.hostname.clone() } else { format!("{user}@{}", f.hostname) };
        if !f.port.is_empty() {
            read.push(':');
            read.push_str(&f.port);
        }
        if !f.identity.is_empty() {
            read.push_str(" \u{b7} ");
            read.push_str(&f.identity);
        }
        self.note = Note::Read(read);
        self.touched = true;
    }

    // ------------------------------------------------------------- geometry --

    fn x0(&self) -> i32 {
        if self.hairline_left { 1 } else { 0 }
    }

    /// The content's width (the box without its hairline).
    fn cw(&self) -> i32 {
        self.rect.2 - 1
    }

    fn close_rect(&self) -> Rect {
        (self.x0() + self.cw() - 8 - SQ, 10, SQ, SQ)
    }

    fn back_rect(&self) -> Rect {
        (self.x0() + 10, 10, SQ, SQ)
    }

    /// The footer's buttons as they are laid out, and where the footer starts (its hairline).
    fn buttons(&self, hosts: &[Host]) -> (Vec<Button>, i32) {
        let mk = |b, label: Cow<'static, str>, primary, danger, off| Button { b, label, primary, danger, off, rect: (0, 0, 0, 0) };
        let mut v: Vec<Button> = match self.view {
            View::List => vec![mk(Btn::More, tr(Str::HostsMore).into(), false, false, false), mk(Btn::Add, tr(Str::HostsAdd).into(), true, false, false)],
            View::Edit => {
                let mut v = Vec::new();
                if self.editing.is_some() {
                    v.push(mk(Btn::Delete, tr(Str::HostsDelete).into(), false, true, false));
                    v.push(mk(Btn::Duplicate, tr(Str::HostsDuplicate).into(), false, false, false));
                }
                v.push(mk(Btn::Cancel, tr(Str::HostsCancel).into(), false, false, false));
                v.push(mk(Btn::Save, tr(Str::HostsSave).into(), true, false, !self.check(hosts).errors.is_empty()));
                v
            }
            View::Import => {
                let n = self.fresh(hosts).count();
                let label: Cow<'static, str> = if n > 0 { tr(Str::HostsImportN).replace("{}", &n.to_string()).into() } else { tr(Str::HostsImport).into() };
                vec![mk(Btn::Cancel, tr(Str::HostsCancel).into(), false, false, false), mk(Btn::Import, label, true, false, n == 0)]
            }
            View::Export => vec![mk(Btn::Back, tr(Str::HostsBack).into(), false, false, false), mk(Btn::Copy, tr(Str::HostsCopy).into(), true, false, false)],
        };
        let (side, top, bottom) = if self.narrow { (12, 10, 12) } else { (18, 10, 14) };
        let avail = self.cw() - 2 * side;
        let widths: Vec<i32> = v.iter().map(|b| self.text.width(&b.label, PX) + 2 * BTN_PAD).collect();
        // Rows: two by two on a narrow window, each half the width (a lone one the whole); else as many as fit, flush right,
        // the danger button pushed to the left of its row (`.btn.push`).
        let mut rows: Vec<std::ops::Range<usize>> = Vec::new();
        if self.narrow {
            let mut i = 0;
            while i < v.len() {
                rows.push(i..(i + 2).min(v.len()));
                i += 2;
            }
        } else {
            let mut start = 0;
            let mut w = 0;
            for (i, &bw) in widths.iter().enumerate() {
                if i > start && w + BTN_GAP + bw > avail {
                    rows.push(start..i);
                    start = i;
                    w = 0;
                }
                w += if i > start { BTN_GAP } else { 0 } + bw;
            }
            rows.push(start..v.len());
        }
        let foot_h = 1 + top + rows.len() as i32 * BTN_H + (rows.len() as i32 - 1).max(0) * BTN_GAP + bottom;
        let foot_top = self.rect.3 - foot_h;
        let left = self.x0() + side;
        for (r, range) in rows.iter().enumerate() {
            let y = foot_top + 1 + top + r as i32 * (BTN_H + BTN_GAP);
            if self.narrow {
                let n = range.len() as i32;
                let bw = (avail - (n - 1) * BTN_GAP) / n;
                for (k, i) in range.clone().enumerate() {
                    v[i].rect = (left + k as i32 * (bw + BTN_GAP), y, bw, BTN_H);
                }
                continue;
            }
            let total: i32 = range.clone().map(|i| widths[i]).sum::<i32>() + (range.len() as i32 - 1) * BTN_GAP;
            let mut x = left + avail - total;
            for i in range.clone() {
                if v[i].danger && i == range.start {
                    v[i].rect = (left, y, widths[i], BTN_H);
                    continue;
                }
                v[i].rect = (x, y, widths[i], BTN_H);
                x += widths[i] + BTN_GAP;
            }
            // With the danger button pushed away, the others close up at the right.
            if v[range.start].danger && range.len() > 1 {
                let rest: i32 = (range.start + 1..range.end).map(|i| widths[i]).sum::<i32>() + (range.len() as i32 - 2) * BTN_GAP;
                let mut x = left + avail - rest;
                for i in range.start + 1..range.end {
                    v[i].rect.0 = x;
                    x += widths[i] + BTN_GAP;
                }
            }
        }
        (v, foot_top)
    }

    fn foot_top(&self, hosts: &[Host]) -> i32 {
        self.buttons(hosts).1
    }

    fn search_rect(&self) -> Rect {
        (self.x0() + PAD + 2, SEARCH_Y, self.cw() - 2 * PAD - 4, SEARCH_H)
    }

    /// The bottom of the list's viewport: above the line of keys when there is one.
    fn list_bottom(&self, hosts: &[Host]) -> i32 {
        let ft = self.foot_top(hosts) - BODY_BOTTOM;
        if self.shown.is_empty() { ft } else { ft - TIP_H - 8 }
    }

    /// Row `i` of the list, scrolled.
    fn row_rect(&self, i: usize) -> Rect {
        (self.x0() + PAD, LIST_Y + i as i32 * ROW_H - self.scroll, self.cw() - 2 * PAD, ROW_H)
    }

    fn tool_rect(&self, i: usize, t: Tool) -> Rect {
        let (x, y, w, h) = self.row_rect(i);
        let right = x + w - 6;
        let cy = y + h / 2 - TOOL_H / 2;
        match t {
            Tool::Edit => (right - TOOL_W, cy, TOOL_W, TOOL_H),
            Tool::Window => (right - 2 * TOOL_W - 1, cy, TOOL_W, TOOL_H),
        }
    }

    /// Whether row `i` shows its buttons: it is selected or under the pointer.
    fn acts(&self, i: usize) -> bool {
        i == self.sel || matches!(self.hover, Hit::Row(r) | Hit::Tool(r, _) if r == i)
    }

    /// `s` cut into lines at most `max` pixels wide, at spaces (a word wider than that is cut where it must be).
    fn wrap(&self, s: &str, px: f32, max: i32) -> Vec<String> {
        let mut out = Vec::new();
        let mut line = String::new();
        for word in s.split(' ') {
            let try_line = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if self.text.width(&try_line, px) <= max || line.is_empty() && self.text.width(word, px) <= max {
                line = try_line;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            // A word too long for a line on its own is cut by characters.
            let mut piece = String::new();
            for c in word.chars() {
                piece.push(c);
                if self.text.width(&piece, px) > max && piece.chars().count() > 1 {
                    piece.pop();
                    out.push(std::mem::take(&mut piece));
                    piece.push(c);
                }
            }
            line = piece;
        }
        if !line.is_empty() || out.is_empty() {
            out.push(line);
        }
        out
    }

    /// `s` cut every `cols` characters (`word-break: break-all`).
    fn chop(s: &str, cols: usize) -> Vec<String> {
        let cols = cols.max(1);
        let chars: Vec<char> = s.chars().collect();
        if chars.is_empty() {
            return vec![String::new()];
        }
        chars.chunks(cols).map(|c| c.iter().collect()).collect()
    }

    fn mono_w(&self) -> i32 {
        (self.mono.cell_w as i32).max(1)
    }

    /// The body of the edit, import and export views, in content coordinates, and its height.
    fn blocks(&self, hosts: &[Host]) -> (Vec<Block>, i32) {
        let x = self.x0() + PAD;
        let w = self.cw() - 2 * PAD;
        let mut v = Vec::new();
        let mut y = 0;
        let lines = |v: &mut Vec<Block>, y: &mut i32, (mt, mx, mb): (i32, i32, i32), px: f32, line: i32, tone: Tone, s: &str, bold: usize| {
            let ls = self.wrap(s, px, w - 2 * mx);
            let b = Block::Lines { x: x + mx, y: *y + mt, px, line, tone, lines: ls, bold };
            *y = b.bottom() + mb;
            v.push(b);
        };
        match self.view {
            View::List => {}
            View::Edit => {
                let fw = w - 2 * PAD;
                if self.editing.is_none() {
                    v.push(Block::Field { f: Fld::Paste, x: x + PAD, y, w: fw });
                    // The hint is the field's: 7 px under the box, 2 px of its own margin, 10 below and the field's 8.
                    let (s, t) = match &self.note {
                        Note::Hint => (tr(Str::HostsPasteHint).to_string(), Tone::Faint),
                        Note::Read(r) => (tr(Str::HostsPasteRead).replace("{}", r), Tone::Faint),
                        Note::Bad => (tr(Str::HostsPasteBad).to_string(), Tone::Err),
                    };
                    y += FIELD_H - FIELD_PAD + LABEL_GAP;
                    lines(&mut v, &mut y, (2, 2 * PAD, 10 + FIELD_PAD), PX_SMALL, HINT_LINE, t, &s, 0);
                }
                for f in [Fld::Alias, Fld::Address] {
                    v.push(Block::Field { f, x: x + PAD, y, w: fw });
                    y += FIELD_H;
                }
                let col1 = w - PORT_W - 8;
                v.push(Block::Field { f: Fld::User, x: x + PAD, y, w: col1 - 2 * PAD });
                v.push(Block::Field { f: Fld::Port, x: x + col1 + 8 + PAD, y, w: PORT_W - 2 * PAD });
                y += FIELD_H;
                for f in [Fld::Key, Fld::Cmd] {
                    v.push(Block::Field { f, x: x + PAD, y, w: fw });
                    y += FIELD_H;
                }
                // The command it amounts to.
                let form = self.form();
                let alias = form.alias.trim();
                let h = ssh::Form { hostname: if form.hostname.trim().is_empty() { "\u{2026}".into() } else { form.hostname.clone() }, ..form.clone() }.host();
                let mut second = h.target(&self.local);
                if !h.identity.is_empty() {
                    second.push_str(" \u{b7} ");
                    second.push_str(&h.identity);
                }
                if !h.cmd.is_empty() {
                    second.push_str(" \u{b7} ");
                    second.push_str(&h.cmd);
                }
                let cols = ((w - 2 * PAD - 2 - 24) / self.mono_w()) as usize;
                let mut ls = Self::chop(&format!("$ ssh {}", if alias.is_empty() { "alias" } else { alias }), cols);
                let first = ls.len();
                ls.extend(Self::chop(&second, cols));
                let ch = 2 + 20 + MONO_LINE * ls.len() as i32;
                v.push(Block::Code { x: x + PAD, y: y + 4, w: w - 2 * PAD, h: ch, lines: ls, first });
                y += 4 + ch + 8;
                let c = self.check(hosts);
                let errors = if self.touched { c.errors.as_slice() } else { &[] };
                for (p, t) in errors.iter().map(|p| (p, Tone::Err)).chain(c.warnings.iter().map(|p| (p, Tone::Warn))) {
                    lines(&mut v, &mut y, (2, PAD, 6), PX_HINT, MSG_LINE, t, &problem_text(p), 0);
                }
            }
            View::Import => {
                lines(&mut v, &mut y, (2, PAD, 10), PX_SMALL, HINT_LINE, Tone::Faint, tr(Str::HostsImportHint), 0);
                if let Some(e) = &self.import_err {
                    lines(&mut v, &mut y, (2, PAD, 6), PX_HINT, MSG_LINE, Tone::Err, &tr(Str::HostsImportFailed).replace("{}", e), 0);
                } else if self.import_text.trim().is_empty() {
                    lines(&mut v, &mut y, (2, PAD, 6), PX_HINT, MSG_LINE, Tone::Dim, tr(Str::HostsImportNone), 0);
                } else {
                    let n = self.import_text.lines().count().min(AREA_MAX) as i32;
                    let h = (20 + 2 + n * MONO_LINE).max(200);
                    v.push(Block::Area { x, y, w, h, export: false });
                    y += h;
                    let ok = self.found.iter().filter(|f| !f.bad).count();
                    let fresh = self.fresh(hosts).count();
                    let mut sum = count(self.found.len(), Str::HostsFoundOne, Str::HostsFoundMany);
                    sum.push_str(" \u{b7} ");
                    sum.push_str(&count(fresh, Str::HostsNewOne, Str::HostsNewMany));
                    if ok > fresh {
                        sum.push_str(" \u{b7} ");
                        sum.push_str(&count(ok - fresh, Str::HostsExistsOne, Str::HostsExistsMany));
                    }
                    lines(&mut v, &mut y, (10, PAD, 6), PX, 18, Tone::Fg, &sum, 0);
                    for i in &self.issues {
                        let head = tr(Str::HostsLine).replace("{}", &i.line.to_string());
                        let s = format!("{head} \u{b7} {}", issue_text(i));
                        let t = if i.level == Level::Error { Tone::Err } else { Tone::Warn };
                        lines(&mut v, &mut y, (2, PAD, 6), PX_HINT, MSG_LINE, t, &s, head.len());
                    }
                }
            }
            View::Export => {
                lines(&mut v, &mut y, (2, PAD, 10), PX_SMALL, HINT_LINE, Tone::Faint, &tr(Str::HostsExportHint).replace("{}", &self.export_path), 0);
                let n = self.export_text.lines().count().min(AREA_MAX) as i32;
                let h = (20 + 2 + n * MONO_LINE).max(300);
                v.push(Block::Area { x, y, w, h, export: true });
                y += h;
                lines(&mut v, &mut y, (2, PAD, 10), PX_SMALL, HINT_LINE, Tone::Faint, &count(hosts.len(), Str::HostsCountOne, Str::HostsCountMany), 0);
            }
        }
        (v, y)
    }

    /// The box of field `f` in panel pixels (scrolled), when the view has it.
    fn field_rect(&self, f: Fld, hosts: &[Host]) -> Option<Rect> {
        let (blocks, _) = self.blocks(hosts);
        let top = HEAD_H + BODY_TOP - self.scroll;
        blocks.iter().find_map(|b| match *b {
            Block::Field { f: g, x, y, w } if g == f => Some((x, top + y + FIELD_PAD + LABEL_H + LABEL_GAP, w, H_BOX)),
            _ => None,
        })
    }

    fn max_scroll(&self, hosts: &[Host]) -> i32 {
        if self.view == View::List {
            let view = self.list_bottom(hosts) - LIST_Y;
            return (self.shown.len() as i32 * ROW_H - view).max(0);
        }
        let view = self.foot_top(hosts) - HEAD_H;
        (BODY_TOP + self.blocks(hosts).1 + BODY_BOTTOM - view).max(0)
    }

    /// Brings the selected row into the list's view.
    fn reveal_sel(&mut self, hosts: &[Host]) {
        let (top, h) = (self.sel as i32 * ROW_H, ROW_H);
        let view = self.list_bottom(hosts) - LIST_Y;
        if top < self.scroll {
            self.scroll = top;
        } else if top + h > self.scroll + view {
            self.scroll = top + h - view;
        }
        self.scroll = self.scroll.clamp(0, self.max_scroll(hosts));
    }

    /// Brings the field with the keyboard into the body's view.
    fn reveal_focus(&mut self, hosts: &[Host]) {
        let Focus::Field(f) = self.focus else { return };
        let Some((_, y, _, h)) = self.field_rect(f, hosts) else { return };
        let (top, bottom) = (HEAD_H, self.foot_top(hosts));
        let (y0, y1) = (y - FIELD_PAD - LABEL_H - LABEL_GAP, y + h + FIELD_PAD);
        if y0 < top {
            self.scroll -= top - y0;
        } else if y1 > bottom {
            self.scroll += y1 - bottom;
        }
        self.scroll = self.scroll.clamp(0, self.max_scroll(hosts));
    }

    // ------------------------------------------------------------- hit test --

    /// What is at window pixel `(x, y)`.
    pub fn hit(&self, x: f32, y: f32, hosts: &[Host]) -> Hit {
        let (lx, ly) = (x as i32 - self.rect.0, y as i32 - self.rect.1);
        if lx < 0 || ly < 0 || lx >= self.rect.2 || ly >= self.rect.3 {
            return Hit::Outside;
        }
        if inside(lx, ly, self.close_rect()) {
            return Hit::Close;
        }
        if self.view != View::List && inside(lx, ly, self.back_rect()) {
            return Hit::Back;
        }
        let (buttons, foot_top) = self.buttons(hosts);
        if ly >= foot_top {
            return buttons.iter().find(|b| inside(lx, ly, b.rect)).map_or(Hit::Panel, |b| Hit::Btn(b.b));
        }
        if ly < HEAD_H {
            return Hit::Panel;
        }
        match self.view {
            View::List => {
                if inside(lx, ly, self.search_rect()) {
                    return Hit::Search;
                }
                if ly < LIST_Y || ly >= self.list_bottom(hosts) {
                    return Hit::Panel;
                }
                for i in 0..self.shown.len() {
                    let r = self.row_rect(i);
                    if !inside(lx, ly, r) {
                        continue;
                    }
                    for t in [Tool::Window, Tool::Edit] {
                        if self.acts(i) && inside(lx, ly, self.tool_rect(i, t)) {
                            return Hit::Tool(i, t);
                        }
                    }
                    return Hit::Row(i);
                }
                Hit::Panel
            }
            View::Edit => FIELDS.into_iter().find(|&f| self.field_rect(f, hosts).is_some_and(|r| inside(lx, ly, r))).map_or(Hit::Panel, Hit::Field),
            View::Import | View::Export => Hit::Panel,
        }
    }

    // ---------------------------------------------------------------- input --

    /// The focusable things of the view, in Tab's order.
    fn stops(&self, hosts: &[Host]) -> Vec<Focus> {
        let mut v = Vec::new();
        match self.view {
            View::List => v.push(Focus::Search),
            View::Edit => {
                let skip_paste = self.editing.is_some();
                v.extend(FIELDS.into_iter().filter(|&f| !(skip_paste && f == Fld::Paste)).map(Focus::Field));
            }
            _ => {}
        }
        v.extend(self.buttons(hosts).0.iter().filter(|b| !b.off).map(|b| Focus::Btn(b.b)));
        v
    }

    fn tab(&mut self, d: i32, hosts: &[Host]) -> Effect {
        let stops = self.stops(hosts);
        if stops.is_empty() {
            return Effect::None;
        }
        let at = stops.iter().position(|&s| s == self.focus).map_or(0, |i| (i as i32 + d).rem_euclid(stops.len() as i32) as usize);
        self.focus = stops[at];
        self.ring = true;
        self.reveal_focus(hosts);
        Effect::Redraw
    }

    /// A key. `Esc` clears the search, then closes; in the other views it goes back to the list.
    pub fn key(&mut self, k: &Key, hosts: &[Host]) -> Effect {
        let m = k.mods;
        if k.sym == sym::ESCAPE {
            if self.view != View::List {
                self.list(hosts);
                return Effect::Redraw;
            }
            if !self.query.is_empty() {
                self.query.set("");
                self.sel = 0;
                self.scroll = 0;
                self.sync(hosts, None);
                return Effect::Redraw;
            }
            return Effect::Close;
        }
        if k.sym == sym::TAB || k.sym == sym::ISO_LEFT_TAB {
            return self.tab(if m.shift || k.sym == sym::ISO_LEFT_TAB { -1 } else { 1 }, hosts);
        }
        let enter = matches!(k.sym, sym::RETURN | sym::KP_ENTER);
        if let Focus::Btn(b) = self.focus {
            if enter || k.sym == sym::SPACE {
                return self.press_btn(b, hosts);
            }
            // Typing in the list goes back to the search.
            if self.view == View::List && !k.text.is_empty() && !m.ctrl && !m.alt {
                self.focus = Focus::Search;
            } else {
                return Effect::None;
            }
        }
        match (self.view, self.focus) {
            (View::List, Focus::Search) => {
                let n = self.shown.len();
                match k.sym {
                    sym::UP | sym::DOWN if n > 0 => {
                        self.sel = if k.sym == sym::DOWN { (self.sel + 1) % n } else { (self.sel + n - 1) % n };
                        self.reveal_sel(hosts);
                        return Effect::Redraw;
                    }
                    _ if enter => {
                        return match self.shown.get(self.sel) {
                            Some(&host) => Effect::Connect { host, window: m.ctrl },
                            None => Effect::None,
                        };
                    }
                    _ => {}
                }
                let e = self.query.key(k);
                if e == Edit::Changed {
                    self.sel = 0;
                    self.scroll = 0;
                    self.sync(hosts, None);
                }
                self.field_effect(e)
            }
            (View::Edit, Focus::Field(f)) => {
                if enter {
                    return self.save(hosts);
                }
                let e = self.form[f as usize].key(k);
                if e == Edit::Changed {
                    self.touched = true;
                    if f == Fld::Paste {
                        self.apply_paste(hosts);
                    }
                    self.scroll = self.scroll.clamp(0, self.max_scroll(hosts));
                }
                self.field_effect(e)
            }
            _ => Effect::None,
        }
    }

    fn field_effect(&self, e: Edit) -> Effect {
        match e {
            Edit::Ignored | Edit::Same => Effect::None,
            Edit::Moved | Edit::Changed => Effect::Redraw,
            Edit::Paste => Effect::Paste,
            Edit::Copy(s) | Edit::Cut(s) => Effect::Clip(s),
        }
    }

    /// The clipboard's text, asked for by `Effect::Paste`, goes where the caret is.
    pub fn paste(&mut self, text: &str, hosts: &[Host]) -> Effect {
        match (self.view, self.focus) {
            (View::List, Focus::Search) => {
                if self.query.insert(text) {
                    self.sel = 0;
                    self.scroll = 0;
                    self.sync(hosts, None);
                }
            }
            (View::Edit, Focus::Field(f)) => {
                if self.form[f as usize].insert(text) {
                    self.touched = true;
                    if f == Fld::Paste {
                        self.apply_paste(hosts);
                    }
                }
            }
            _ => return Effect::None,
        }
        Effect::Redraw
    }

    /// Save, if nothing is wrong; else the errors show.
    fn save(&mut self, hosts: &[Host]) -> Effect {
        if !self.check(hosts).errors.is_empty() {
            // What is wrong is said under the command: brought into view.
            self.touched = true;
            self.scroll = self.max_scroll(hosts);
            return Effect::Redraw;
        }
        let mut host = self.form().host();
        if let Some(old) = self.editing.and_then(|i| hosts.get(i)) {
            (host.last, host.uses) = (old.last, old.uses);
        }
        Effect::Save { at: self.editing, host }
    }

    fn press_btn(&mut self, b: Btn, hosts: &[Host]) -> Effect {
        let (buttons, _) = self.buttons(hosts);
        let Some(btn) = buttons.iter().find(|x| x.b == b) else { return Effect::None };
        if btn.off {
            return Effect::None;
        }
        match b {
            Btn::More => Effect::Menu(self.rect.0 + btn.rect.0, self.rect.1 + btn.rect.1 - 90),
            Btn::Add => {
                self.edit(hosts, None, None, false);
                Effect::Redraw
            }
            Btn::Delete => self.editing.map_or(Effect::None, Effect::Delete),
            Btn::Duplicate => {
                let mut f = self.form();
                let seed = ssh::Form { hostname: f.alias.clone(), ..ssh::Form::default() };
                f.alias = ssh::suggest_alias(&seed, hosts, tr(Str::HostsDefaultAlias));
                self.edit(hosts, None, Some(f), true);
                Effect::Redraw
            }
            Btn::Cancel | Btn::Back => {
                self.list(hosts);
                Effect::Redraw
            }
            Btn::Save => self.save(hosts),
            Btn::Import => Effect::Import(self.fresh(hosts).map(|f| f.host.clone()).collect()),
            Btn::Copy => Effect::CopyConfig,
        }
    }

    /// The pointer moved to window pixel `(x, y)`; a field being dragged in extends its selection.
    pub fn pointer(&mut self, x: f32, y: f32, hosts: &[Host]) -> Effect {
        let mut changed = false;
        if let Some((f, r)) = self.dragging(hosts) {
            let lx = x - self.rect.0 as f32;
            changed |= match f {
                None => self.query.drag_at(lx, r, Look::Find, &self.text),
                Some(f) => self.form[f as usize].drag_at(lx, r, Look::Box, &self.text),
            };
        }
        let now = self.hit(x, y, hosts);
        changed |= now != self.hover;
        self.hover = now;
        if changed { Effect::Redraw } else { Effect::None }
    }

    /// The field whose text is being selected with the pointer (none: the search), and its box.
    fn dragging(&self, hosts: &[Host]) -> Option<(Option<Fld>, Rect)> {
        if self.query.dragging() {
            return Some((None, self.search_rect()));
        }
        FIELDS.into_iter().find(|&f| self.form[f as usize].dragging()).and_then(|f| Some((Some(f), self.field_rect(f, hosts)?)))
    }

    /// The primary button went down at window pixel `(x, y)`, the `clicks`-th time in a row there.
    pub fn press(&mut self, x: f32, y: f32, clicks: u8, hosts: &[Host]) -> Effect {
        let hit = self.hit(x, y, hosts);
        self.hover = hit;
        self.focused = true;
        self.ring = false;
        let lx = x - self.rect.0 as f32;
        match hit {
            Hit::Outside => {
                self.focused = false;
                Effect::Redraw
            }
            Hit::Panel => Effect::Redraw,
            Hit::Close => Effect::Close,
            Hit::Back => {
                self.list(hosts);
                Effect::Redraw
            }
            Hit::Search => {
                self.focus = Focus::Search;
                self.query.press_at(lx, self.search_rect(), Look::Find, &self.text, clicks, false);
                Effect::Redraw
            }
            Hit::Row(i) => {
                self.sel = i;
                self.focus = Focus::Search;
                match self.shown.get(i) {
                    Some(&host) => Effect::Connect { host, window: false },
                    None => Effect::Redraw,
                }
            }
            Hit::Tool(i, t) => {
                self.sel = i;
                let Some(&host) = self.shown.get(i) else { return Effect::Redraw };
                match t {
                    Tool::Window => Effect::Connect { host, window: true },
                    Tool::Edit => {
                        self.edit(hosts, Some(host), None, false);
                        Effect::Redraw
                    }
                }
            }
            Hit::Field(f) => {
                self.focus = Focus::Field(f);
                if let Some(r) = self.field_rect(f, hosts) {
                    self.form[f as usize].press_at(lx, r, Look::Box, &self.text, clicks, false);
                }
                Effect::Redraw
            }
            Hit::Btn(b) => {
                if self.buttons(hosts).0.iter().any(|x| x.b == b && !x.off) {
                    self.focus = Focus::Btn(b);
                }
                self.press_btn(b, hosts)
            }
        }
    }

    pub fn release(&mut self) {
        self.query.release();
        for f in &mut self.form {
            f.release();
        }
    }

    /// The wheel: the list scrolls in the list view, the body in the others.
    pub fn scroll_by(&mut self, px: i32, hosts: &[Host]) -> Effect {
        let s = (self.scroll + px).clamp(0, self.max_scroll(hosts));
        if s == self.scroll {
            return Effect::None;
        }
        self.scroll = s;
        Effect::Redraw
    }

    /// Whether the pointer is over text (an I-beam) or over something that takes a click (a hand).
    pub fn over_text(&self) -> bool {
        matches!(self.hover, Hit::Search | Hit::Field(_))
    }

    pub fn over_button(&self) -> bool {
        matches!(self.hover, Hit::Close | Hit::Back | Hit::Row(_) | Hit::Tool(..) | Hit::Btn(_))
    }

    // ----------------------------------------------------------------- paint --

    /// Paints the panel into its box (cleared by the caller); `(ox, oy)` is where the window starts in the buffer.
    pub fn draw(&self, fb: &mut [u32], w: usize, h: usize, (ox, oy): (usize, usize), hosts: &[Host]) {
        let (_, _, bw, bh) = self.rect;
        if fb.len() < w * h || bw <= 0 || bh <= 0 {
            return;
        }
        let fade = self.fade.value();
        let (px0, py0) = (ox as i32 + self.rect.0, oy as i32 + self.rect.1);
        let x0 = self.x0();
        let (buttons, foot_top) = self.buttons(hosts);

        // Header.
        {
            let mut p = Paint { fb: &mut *fb, w, h, ox: px0, oy: py0, fade };
            let hx = if self.hairline_left { 0 } else { bw - 1 };
            p.shape(hx, 0, hx + 1, bh, WHITE, 0.07, &|_, _| 1.0);
            let mut tx = x0 + 10;
            if self.view != View::List {
                self.square(&mut p, self.back_rect(), Icon::Back, self.hover == Hit::Back);
                tx += SQ + 2;
            }
            let title: Cow<str> = match self.view {
                View::List => tr(Str::HostsTitle).into(),
                View::Edit if self.editing.is_none() => tr(Str::HostsNew).into(),
                View::Edit => {
                    let a = self.form[Fld::Alias as usize].text().trim();
                    if a.is_empty() { tr(Str::HostsServer).into() } else { a.to_string().into() }
                }
                View::Import => tr(Str::HostsImportTitle).into(),
                View::Export => tr(Str::HostsExportTitle).into(),
            };
            let room = self.close_rect().0 - 12 - tx;
            let title = self.text.fit(&title, PX_TITLE, room);
            self.text.draw(&mut p, tx, 10 + SQ / 2, &title, Ink::new(PX_TITLE, fg()).bold());
            self.square(&mut p, self.close_rect(), Icon::Cross, self.hover == Hit::Close);
        }

        // Body, clipped between the header and the footer.
        let (r0, r1) = ((py0 + HEAD_H).max(0) as usize, ((py0 + foot_top).max(0) as usize).min(h));
        if r0 < r1 {
            match self.view {
                View::List => {
                    {
                        let mut p = Paint { fb: &mut fb[r0 * w..r1 * w], w, h: r1 - r0, ox: px0, oy: py0 - r0 as i32, fade };
                        let on = self.focused && self.focus == Focus::Search;
                        field::paint(&mut p, &self.text, &self.query, self.search_rect(), Look::Find, on);
                        if !self.shown.is_empty() {
                            self.draw_tip(&mut p, self.list_bottom(hosts) + 8 + TIP_H / 2);
                        } else {
                            self.draw_empty(&mut p, hosts);
                        }
                    }
                    let (l0, l1) = ((py0 + LIST_Y).max(0) as usize, ((py0 + self.list_bottom(hosts)).max(0) as usize).min(h));
                    if l0 < l1 {
                        let mut p = Paint { fb: &mut fb[l0 * w..l1 * w], w, h: l1 - l0, ox: px0, oy: py0 - l0 as i32, fade };
                        self.draw_rows(&mut p, hosts);
                    }
                }
                _ => {
                    let mut p = Paint { fb: &mut fb[r0 * w..r1 * w], w, h: r1 - r0, ox: px0, oy: py0 - r0 as i32, fade };
                    self.draw_blocks(&mut p, hosts, (HEAD_H, foot_top));
                }
            }
        }

        // Footer.
        let mut p = Paint { fb: &mut *fb, w, h, ox: px0, oy: py0, fade };
        p.shape(x0, foot_top, x0 + self.cw(), foot_top + 1, WHITE, 0.07, &|_, _| 1.0);
        for b in &buttons {
            self.draw_button(&mut p, b);
        }
    }

    /// A 28 px icon button of the header: dim, lit on a rounded highlight under the pointer.
    fn square(&self, p: &mut Paint, (x, y, w, h): Rect, icon: Icon, lit: bool) {
        if lit {
            p.rrect(x, y, w, h, 8.0, WHITE, 0.07);
        }
        icons::paint(p, icon, x as f32 + w as f32 / 2.0, y as f32 + h as f32 / 2.0, 16.0, 1.5, if lit { fg() } else { dim() }, 1.0);
    }

    fn draw_button(&self, p: &mut Paint, b: &Button) {
        let (x, y, w, h) = b.rect;
        let lit = self.hover == Hit::Btn(b.b) && !b.off;
        let keep = p.fade;
        if b.off {
            p.fade *= 0.4;
        }
        let ink = if b.primary {
            p.rrect(x, y, w, h, 9.0, [if lit { 1.0 } else { 0.957 }; 3], 1.0);
            [0.067, 0.067, 0.075]
        } else if b.danger {
            if lit {
                p.rrect(x, y, w, h, 9.0, palette::unit(palette::RED), 0.14);
            } else {
                p.rrect(x, y, w, h, 9.0, WHITE, 0.07);
            }
            RED
        } else {
            p.rrect(x, y, w, h, 9.0, WHITE, if lit { 0.12 } else { 0.07 });
            fg()
        };
        let label = self.text.fit(&b.label, PX, w - 8);
        self.text.draw(p, x + (w - self.text.width(&label, PX)) / 2, y + h / 2, &label, Ink::new(PX, ink).medium());
        p.fade = keep;
        if self.focused && self.ring && self.focus == Focus::Btn(b.b) {
            p.outline(x - 2, y - 2, w + 4, h + 4, 11.0, accent(), 0.7);
            p.outline(x - 3, y - 3, w + 6, h + 6, 12.0, accent(), 0.7);
        }
    }

    /// `↑ ↓ choose · ↵ connect · Ctrl ↵ new window`, centred on `cy`.
    fn draw_tip(&self, p: &mut Paint, cy: i32) {
        let ink = Ink::new(PX_SMALL, faint());
        let mut x = self.x0() + PAD + 8;
        let text = |p: &mut Paint, x: &mut i32, s: &str| {
            self.text.draw(p, *x, cy, s, ink);
            *x += self.text.width(s, PX_SMALL);
        };
        let key = |p: &mut Paint, x: &mut i32, c: Cap| {
            *x += kbd(p, &self.text, *x, cy, c, 1.0) + 2;
        };
        key(p, &mut x, Cap::Icon(Icon::Up));
        key(p, &mut x, Cap::Icon(Icon::Down));
        text(p, &mut x, &format!(" {} \u{b7} ", tr(Str::HostsChoose)));
        key(p, &mut x, Cap::Icon(Icon::Enter));
        text(p, &mut x, &format!(" {} \u{b7} ", tr(Str::HostsConnect)));
        key(p, &mut x, Cap::Text("Ctrl"));
        key(p, &mut x, Cap::Icon(Icon::Enter));
        text(p, &mut x, &format!(" {}", tr(Str::HostsNewWindow)));
    }

    /// No servers, or none that matches the search (`.h-empty`).
    fn draw_empty(&self, p: &mut Paint, hosts: &[Host]) {
        let x = self.x0() + PAD + 8;
        let max = self.cw() - 2 * PAD - 16;
        let mut lines = Vec::new();
        if hosts.is_empty() {
            lines.extend(self.wrap(tr(Str::HostsEmpty), PX, max));
            lines.extend(self.wrap(tr(Str::HostsEmptyHint), PX, max));
        } else {
            lines.extend(self.wrap(&tr(Str::HostsNoMatch).replace("{}", self.query.text()), PX, max));
        }
        for (i, l) in lines.iter().enumerate() {
            self.text.draw(p, x, LIST_Y + 16 + 9 + i as i32 * 18, l, Ink::new(PX, faint()));
        }
    }

    fn draw_rows(&self, p: &mut Paint, hosts: &[Host]) {
        let now = ssh::now();
        let green = palette::unit(palette::GREEN);
        for (i, &hi) in self.shown.iter().enumerate() {
            let (x, y, w, h) = self.row_rect(i);
            if y + h < LIST_Y || y > self.list_bottom(hosts) {
                continue;
            }
            let Some(host) = hosts.get(hi) else { continue };
            let over = matches!(self.hover, Hit::Row(r) | Hit::Tool(r, _) if r == i);
            if i == self.sel {
                p.rrect(x, y, w, h, 8.0, WHITE, 0.08);
            } else if over {
                p.rrect(x, y, w, h, 8.0, WHITE, 0.05);
            }
            let cy = y + h / 2;
            let (dx, dy) = (x as f32 + 10.0 + 3.5, cy as f32);
            let live = self.is_live(hi);
            if live {
                // A green light with a glow (`box-shadow: 0 0 8px`).
                p.disc(dx, dy, 8.0, green, 0.10);
                p.disc(dx, dy, 6.0, green, 0.18);
                p.disc(dx, dy, 3.5, green, 1.0);
            } else {
                p.ring(dx, dy, 2.75, 1.5, faint(), 1.0);
            }
            let acts = self.acts(i);
            let meta = if live { tr(Str::HostsLive).to_string() } else { ago(host.last, now) };
            let meta_w = self.text.width(&meta, PX_SMALL);
            let tx = x + 10 + 7 + 10;
            let right = x + w - 8 - meta_w - 10 - if acts { 56 } else { 0 };
            let alias = self.text.fit(&host.alias, PX, right - tx);
            self.text.draw(p, tx, y + 7 + 9, &alias, Ink::new(PX, fg()).medium());
            let target = host.target(&self.local);
            let target = self.text.fit(&target, PX_SMALL, right - tx);
            self.text.draw(p, tx, y + 7 + 18 + 1 + 8, &target, Ink::new(PX_SMALL, dim()));
            if acts {
                for t in [Tool::Window, Tool::Edit] {
                    let (bx, by, bw2, bh2) = self.tool_rect(i, t);
                    let lit = self.hover == Hit::Tool(i, t);
                    if lit {
                        p.rrect(bx, by, bw2, bh2, 6.0, WHITE, 0.10);
                    }
                    let icon = if t == Tool::Window { Icon::Open } else { Icon::Pencil };
                    icons::paint(p, icon, bx as f32 + bw2 as f32 / 2.0, by as f32 + bh2 as f32 / 2.0, 13.0, 1.5, if lit { fg() } else { dim() }, 1.0);
                }
            } else if !meta.is_empty() {
                self.text.draw(p, x + w - 8 - meta_w, cy, &meta, Ink::new(PX_SMALL, if live { green } else { faint() }));
            }
        }
    }

    /// The body of the edit, import and export views, scrolled; `(top, bottom)` is the viewport in panel pixels.
    fn draw_blocks(&self, p: &mut Paint, hosts: &[Host], (top, bottom): (i32, i32)) {
        let (blocks, _) = self.blocks(hosts);
        let dy = HEAD_H + BODY_TOP - self.scroll;
        for b in &blocks {
            match b {
                Block::Field { f, x, y, w } => {
                    let label = match f {
                        Fld::Paste => tr(Str::HostsPaste),
                        Fld::Alias => tr(Str::HostsAlias),
                        Fld::Address => tr(Str::HostsAddress),
                        Fld::User => tr(Str::HostsUser),
                        Fld::Port => tr(Str::HostsPort),
                        Fld::Key => tr(Str::HostsKey),
                        Fld::Cmd => tr(Str::HostsCmd),
                    };
                    let y = dy + y;
                    let label = self.text.fit(label, PX_HINT, *w);
                    self.text.draw(p, *x, y + FIELD_PAD + LABEL_H / 2, &label, Ink::new(PX_HINT, dim()));
                    let on = self.focused && self.focus == Focus::Field(*f);
                    field::paint(p, &self.text, &self.form[*f as usize], (*x, y + FIELD_PAD + LABEL_H + LABEL_GAP, *w, H_BOX), Look::Box, on);
                }
                Block::Lines { x, y, px, line, tone: t, lines, bold } => {
                    for (i, l) in lines.iter().enumerate() {
                        let cy = dy + y + i as i32 * line + line / 2;
                        if cy < top - line || cy > bottom + line {
                            continue;
                        }
                        let ink = Ink::new(*px, tone(*t));
                        if i == 0 && *bold > 0 && l.len() >= *bold && l.is_char_boundary(*bold) {
                            let (a, rest) = l.split_at(*bold);
                            self.text.draw(p, *x, cy, a, ink.medium());
                            self.text.draw(p, x + self.text.width(a, *px), cy, rest, ink);
                        } else {
                            self.text.draw(p, *x, cy, l, ink);
                        }
                    }
                }
                Block::Code { x, y, w, h, lines, first } => {
                    let y = dy + y;
                    p.rrect(*x, y, *w, *h, 9.0, [0.0; 3], 0.35);
                    p.outline(*x, y, *w, *h, 9.0, WHITE, 0.07);
                    for (i, l) in lines.iter().enumerate() {
                        let cy = y + 1 + 10 + i as i32 * MONO_LINE + MONO_LINE / 2;
                        let lx = x + 1 + 12;
                        if i == 0 && l.starts_with('$') {
                            self.mono_text(p, lx, cy, "$", accent(), true, (*x, x + w));
                            // `.h-code .ps { margin-right: .8ch }`: the `$` and its space, a little closer.
                            self.mono_text(p, lx + self.mono_w() * 18 / 10, cy, l[1..].trim_start(), fg(), false, (*x, x + w));
                        } else {
                            self.mono_text(p, lx, cy, l, if i < *first { fg() } else { dim() }, false, (*x, x + w));
                        }
                    }
                }
                Block::Area { x, y, w, h, export } => {
                    let y = dy + y;
                    p.rrect(*x, y, *w, *h, 10.0, [0.0; 3], 0.35);
                    p.outline(*x, y, *w, *h, 10.0, WHITE, 0.07);
                    let text = if *export { &self.export_text } else { &self.import_text };
                    let clip = (x + 1 + 12, x + w - 1 - 12);
                    for (i, l) in text.lines().take(AREA_MAX).enumerate() {
                        let cy = y + 1 + 10 + i as i32 * MONO_LINE + MONO_LINE / 2;
                        if cy < top - MONO_LINE || cy > bottom + MONO_LINE {
                            continue;
                        }
                        self.mono_text(p, clip.0, cy, l, fg(), false, clip);
                    }
                }
            }
        }
    }

    /// `s` in the grid's family at 12 px, its left edge at `x`, centred on `cy`, only between `clip.0` and `clip.1`.
    #[allow(clippy::too_many_arguments)]
    fn mono_text(&self, p: &mut Paint, x: i32, cy: i32, s: &str, c: [f32; 3], bold: bool, clip: (i32, i32)) {
        let m = self.mono.grid();
        let (cw, ch) = (m.cell_w as i32, m.cell_h as i32);
        let mut buf = self.scratch.borrow_mut();
        let mut gx = x;
        for c0 in s.chars() {
            if gx >= clip.1 {
                break;
            }
            if c0 != ' ' {
                self.mono.raster_into(&mut buf, &m, c0, bold, false);
                let gy = cy - ch / 2;
                for yy in 0..ch {
                    for xx in 0..cw {
                        let i = ((yy * cw + xx) * 3) as usize;
                        let Some(px) = buf.get(i..i + 3) else { continue };
                        let cov = (px[0] as u32 + px[1] as u32 + px[2] as u32) / 3;
                        if cov != 0 && gx + xx >= clip.0 && gx + xx < clip.1 {
                            p.put(gx + xx, gy + yy, c, cov as f32 / 255.0);
                        }
                    }
                }
            }
            gx += cw;
        }
    }
}

trait WithText {
    fn with_text_of(self, s: &str) -> Self;
}

impl WithText for Field {
    fn with_text_of(mut self, s: &str) -> Field {
        self.set(s);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{i18n, plat::Mods};

    fn sample() -> Vec<Host> {
        ssh::sample()
    }

    fn panel(hosts: &[Host]) -> Hosts {
        let mut p = Hosts::new(hosts, vec![false; hosts.len()]).with_local("ivan");
        p.place((600, 40, 372, 580), false, false, hosts);
        p
    }

    fn key(sym: u32) -> Key {
        Key { sym, text: String::new(), mods: Mods::default() }
    }

    fn ctrl(sym: u32) -> Key {
        Key { sym, text: String::new(), mods: Mods { ctrl: true, ..Mods::default() } }
    }

    fn typed(s: &str) -> Key {
        Key { sym: s.chars().next().map_or(0, |c| c as u32), text: s.into(), mods: Mods::default() }
    }

    fn type_all(p: &mut Hosts, s: &str, hosts: &[Host]) {
        for c in s.chars() {
            p.key(&typed(&c.to_string()), hosts);
        }
    }

    /// The window pixel at the middle of a panel rectangle.
    fn mid(p: &Hosts, (x, y, w, h): Rect) -> (f32, f32) {
        ((p.rect.0 + x + w / 2) as f32, (p.rect.1 + y + h / 2) as f32)
    }

    fn render(p: &Hosts, hosts: &[Host]) -> Vec<u32> {
        let (w, h) = (1000, 640);
        let mut fb = vec![0u32; w * h];
        p.draw(&mut fb, w, h, (0, 0), hosts);
        fb
    }

    #[test]
    fn the_reference_measures_come_out_of_the_layout() {
        let hosts = sample();
        let p = panel(&hosts);
        assert_eq!(p.search_rect(), (13, 48, 347, 30), "`.h-search`: 2 px inside the body's 10 px, under a 38 px header");
        assert_eq!(p.row_rect(0), (11, 90, 351, 49));
        assert_eq!(p.row_rect(1).1, 139, "rows follow each other");
        assert_eq!(p.close_rect(), (336, 10, 28, 28));
        let (b, top) = p.buttons(&hosts);
        assert_eq!(top, 580 - 59, "a footer of one row is 59 px with its hairline");
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].rect.0 + b[1].rect.2, 1 + 371 - 18, "flush right, 18 px in");
        assert!(b[0].rect.0 + b[0].rect.2 + BTN_GAP == b[1].rect.0);
        assert_eq!(p.list_bottom(&hosts), top - 8 - TIP_H - 8, "the line of keys is fixed under the list");
    }

    #[test]
    fn the_list_puts_the_last_used_first_and_the_search_filters_it() {
        let mut hosts = sample();
        hosts[3].last = ssh::now() - 3 * 3600;
        let mut p = panel(&hosts);
        assert_eq!(p.shown, [3, 1, 2, 0], "produccion, used, then the others by name");
        type_all(&mut p, "st", &hosts);
        assert_eq!(p.shown, [2]);
        // The first Escape clears the search, the second closes.
        assert_eq!(p.key(&key(sym::ESCAPE), &hosts), Effect::Redraw);
        assert_eq!(p.shown.len(), 4);
        assert_eq!(p.key(&key(sym::ESCAPE), &hosts), Effect::Close);
    }

    #[test]
    fn enter_connects_the_selected_one_and_ctrl_enter_in_a_new_window() {
        let hosts = sample();
        let mut p = panel(&hosts);
        assert_eq!(p.key(&key(sym::RETURN), &hosts), Effect::Connect { host: p.shown[0], window: false });
        p.key(&key(sym::DOWN), &hosts);
        p.key(&key(sym::DOWN), &hosts);
        assert_eq!(p.sel, 2);
        assert_eq!(p.key(&ctrl(sym::RETURN), &hosts), Effect::Connect { host: p.shown[2], window: true });
        p.key(&key(sym::UP), &hosts);
        p.key(&key(sym::UP), &hosts);
        p.key(&key(sym::UP), &hosts);
        assert_eq!(p.sel, 3, "it wraps");
    }

    #[test]
    fn a_click_on_a_row_connects_and_its_buttons_open_a_window_or_the_form() {
        let hosts = sample();
        let mut p = panel(&hosts);
        let (x, y) = mid(&p, p.row_rect(1));
        p.pointer(x, y, &hosts);
        assert_eq!(p.hover, Hit::Row(1));
        assert!(p.acts(1) && p.acts(0), "the row under the pointer and the selected one show their buttons");
        let (tx, ty) = mid(&p, p.tool_rect(1, Tool::Window));
        assert_eq!(p.press(tx, ty, 1, &hosts), Effect::Connect { host: p.shown[1], window: true });
        let (tx, ty) = mid(&p, p.tool_rect(1, Tool::Edit));
        assert_eq!(p.press(tx, ty, 1, &hosts), Effect::Redraw);
        assert_eq!((p.view, p.editing), (View::Edit, Some(p.shown[1])));
        assert_eq!(p.form[Fld::Alias as usize].text(), hosts[p.shown[1]].alias);
        assert_eq!(p.key(&key(sym::ESCAPE), &hosts), Effect::Redraw);
        assert_eq!(p.view, View::List, "Esc goes back");
        let (x, y) = mid(&p, p.row_rect(2));
        assert_eq!(p.press(x, y, 1, &hosts), Effect::Connect { host: p.shown[2], window: false });
    }

    #[test]
    fn a_pasted_command_fills_the_form_and_enter_saves_it_with_the_proposed_name() {
        i18n::set(i18n::Lang::En);
        let hosts = sample();
        let mut p = panel(&hosts);
        let (x, y) = mid(&p, p.buttons(&hosts).0[1].rect);
        assert_eq!(p.press(x, y, 1, &hosts), Effect::Redraw);
        assert_eq!((p.view, p.focus), (View::Edit, Focus::Field(Fld::Paste)));
        assert!(p.buttons(&hosts).0.iter().any(|b| b.b == Btn::Save && b.off), "nothing to save yet");
        assert_eq!(p.key(&ctrl('v' as u32), &hosts), Effect::Paste);
        p.paste("ssh -i ~/.ssh/keys/api.pem root@203.0.113.9 -p 2200", &hosts);
        assert_eq!(p.form().alias, "api");
        assert_eq!(p.note, Note::Read("root@203.0.113.9:2200 \u{b7} ~/.ssh/keys/api.pem".into()));
        let Effect::Save { at: None, host } = p.key(&key(sym::RETURN), &hosts) else { panic!("saves") };
        assert_eq!((host.alias.as_str(), host.hostname.as_str(), host.user.as_str(), host.port), ("api", "203.0.113.9", "root", 2200));
        // Something that is not an ssh command says so.
        p.edit(&hosts, None, None, false);
        type_all(&mut p, "hello", &hosts);
        assert_eq!(p.note, Note::Bad);
    }

    #[test]
    fn errors_show_once_touched_and_keep_it_from_saving() {
        let hosts = sample();
        let mut p = panel(&hosts);
        p.edit(&hosts, None, None, false);
        let (blocks, _) = p.blocks(&hosts);
        assert!(!blocks.iter().any(|b| matches!(b, Block::Lines { tone: Tone::Err, .. })), "a new form says nothing wrong");
        p.focus = Focus::Field(Fld::Alias);
        type_all(&mut p, "web", &hosts);
        assert_eq!(p.key(&key(sym::RETURN), &hosts), Effect::Redraw, "taken and no address: not saved");
        let (blocks, _) = p.blocks(&hosts);
        assert_eq!(blocks.iter().filter(|b| matches!(b, Block::Lines { tone: Tone::Err, .. })).count(), 2);
        // Editing a server: its own alias is fine; Delete and Duplicate are there, Delete at the left.
        p.edit(&hosts, Some(0), None, false);
        let (b, _) = p.buttons(&hosts);
        let names: Vec<Btn> = b.iter().map(|b| b.b).collect();
        assert_eq!(names, [Btn::Delete, Btn::Duplicate, Btn::Cancel, Btn::Save]);
        assert_eq!(b[0].rect.0, 1 + 18, "Delete is pushed to the left");
        assert!(b.iter().all(|b| !b.off));
        assert_eq!(p.press_btn(Btn::Delete, &hosts), Effect::Delete(0));
        assert!(matches!(p.press_btn(Btn::Save, &hosts), Effect::Save { at: Some(0), .. }));
        p.press_btn(Btn::Duplicate, &hosts);
        assert_eq!((p.editing, p.form().alias.as_str(), p.form().hostname.as_str()), (None, "web-2", "10.10.0.11"));
    }

    #[test]
    fn tab_goes_through_the_fields_then_the_buttons() {
        let hosts = sample();
        let mut p = panel(&hosts);
        p.edit(&hosts, Some(1), None, false);
        assert_eq!(p.focus, Focus::Field(Fld::Alias), "no paste field when editing");
        let mut seen = vec![p.focus];
        for _ in 0..10 {
            p.key(&key(sym::TAB), &hosts);
            seen.push(p.focus);
        }
        let fields = [Fld::Alias, Fld::Address, Fld::User, Fld::Port, Fld::Key, Fld::Cmd].map(Focus::Field);
        assert_eq!(&seen[..6], &fields);
        assert_eq!(&seen[6..10], &[Btn::Delete, Btn::Duplicate, Btn::Cancel, Btn::Save].map(Focus::Btn));
        assert_eq!(seen[10], Focus::Field(Fld::Alias), "and round again");
    }

    #[test]
    fn the_import_offers_the_servers_axon_has_not_got() {
        let hosts = sample();
        let mut p = panel(&hosts);
        p.import(Ok("Host web\n    HostName 1.1.1.1\nHost new1\n    HostName 2.2.2.2\nHost bad\n    Port x\nHost new2\n    HostName 3.3.3.3\n".into()));
        let (b, _) = p.buttons(&hosts);
        assert_eq!(b[1].label, "Import 2");
        let Effect::Import(new) = p.press_btn(Btn::Import, &hosts) else { panic!() };
        assert_eq!(new.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["new1", "new2"]);
        p.import(Ok("Host web\n".into()));
        assert!(p.buttons(&hosts).0[1].off, "nothing new");
        p.import(Err("denied".into()));
        assert!(p.buttons(&hosts).0[1].off);
    }

    #[test]
    fn a_narrow_window_puts_the_buttons_two_by_two() {
        let hosts = sample();
        let mut p = panel(&hosts);
        p.place((0, 40, 400, 500), false, true, &hosts);
        p.edit(&hosts, Some(0), None, false);
        let (b, top) = p.buttons(&hosts);
        assert_eq!(b[0].rect.1, b[1].rect.1);
        assert_eq!(b[2].rect.1, b[0].rect.1 + BTN_H + BTN_GAP);
        assert_eq!(b[0].rect.2, b[1].rect.2);
        assert_eq!(500 - top, 1 + 10 + 2 * BTN_H + BTN_GAP + 12);
    }

    #[test]
    fn ago_says_how_long_in_words() {
        i18n::set(i18n::Lang::Es);
        let n = 1_000_000;
        assert_eq!(ago(0, n), "");
        assert_eq!(ago(n - 10, n), "ahora");
        assert_eq!(ago(n - 3 * 60, n), "hace 3 min");
        assert_eq!(ago(n - 3 * 3600, n), "hace 3 h");
        assert_eq!(ago(n - 30 * 3600, n), "ayer");
        assert_eq!(ago(n - 4 * 86400, n), "hace 4 d");
        i18n::set(i18n::Lang::En);
        assert_eq!(ago(n - 3 * 60, n), "3 min ago");
    }

    #[test]
    fn every_view_paints_inside_its_box() {
        let hosts = sample();
        let mut p = panel(&hosts);
        let live: Vec<bool> = (0..hosts.len()).map(|i| i == 0).collect();
        p.sync(&hosts, Some(live));
        let views: [&dyn Fn(&mut Hosts); 4] = [
            &|_| {},
            &|p| p.edit(&ssh::sample(), None, None, true),
            &|p| p.import(Ok("Host a\n    HostName b\nFoo bar\n".into())),
            &|p| p.export(ssh::write_config(&ssh::sample()), "~/.config/axon/ssh_config".into()),
        ];
        for (n, v) in views.iter().enumerate() {
            v(&mut p);
            let fb = render(&p, &hosts);
            let (x0, y0, w, h) = p.rect;
            let mut inside_n = 0;
            for (i, &px) in fb.iter().enumerate() {
                let (x, y) = ((i % 1000) as i32, (i / 1000) as i32);
                if px >> 24 != 0 {
                    assert!(x >= x0 && x < x0 + w && y >= y0 && y < y0 + h, "view {n} painted outside at {x},{y}");
                    inside_n += 1;
                }
            }
            assert!(inside_n > 2000, "view {n} painted something ({inside_n})");
        }
    }
}
