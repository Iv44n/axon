//! Keyboard -> bytes for the PTY. `encode_key` takes a resolved key (the own window layers: xkb on Wayland, the system's
//! layout on Windows); `encode` takes a winit event and goes away with winit.
use alacritty_terminal::term::TermMode;
use crate::plat::Key as XKey;
#[cfg(target_os = "linux")]
use winit::{
    event::KeyEvent,
    keyboard::{Key, ModifiersState, NamedKey},
};

fn csi(mods: u8, app_cursor: bool, final_byte: char) -> Vec<u8> {
    if mods > 1 {
        format!("\x1b[1;{mods}{final_byte}").into_bytes()
    } else if app_cursor {
        format!("\x1bO{final_byte}").into_bytes()
    } else {
        format!("\x1b[{final_byte}").into_bytes()
    }
}

/// A press of the Left or Right arrow, no modifier, as a program in `mode` expects it (`caret.rs` moves the shell's cursor
/// with these).
pub fn arrow(forward: bool, mode: TermMode) -> &'static [u8] {
    match (forward, mode.contains(TermMode::APP_CURSOR)) {
        (true, true) => b"\x1bOC",
        (true, false) => b"\x1b[C",
        (false, true) => b"\x1bOD",
        (false, false) => b"\x1b[D",
    }
}

fn tilde(mods: u8, n: u8) -> Vec<u8> {
    if mods > 1 { format!("\x1b[{n};{mods}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
}

#[cfg(target_os = "linux")]
pub fn encode(ev: &KeyEvent, m: ModifiersState, mode: TermMode) -> Option<Vec<u8>> {
    let mods = 1 + m.shift_key() as u8 + 2 * m.alt_key() as u8 + 4 * m.control_key() as u8;
    let app = mode.contains(TermMode::APP_CURSOR);

    if let Key::Named(n) = &ev.logical_key {
        let out = match n {
            NamedKey::Enter => b"\r".to_vec(),
            NamedKey::Backspace => if m.alt_key() { b"\x1b\x7f".to_vec() } else { b"\x7f".to_vec() },
            NamedKey::Tab => if m.shift_key() { b"\x1b[Z".to_vec() } else { b"\t".to_vec() },
            NamedKey::Escape => b"\x1b".to_vec(),
            NamedKey::ArrowUp => csi(mods, app, 'A'),
            NamedKey::ArrowDown => csi(mods, app, 'B'),
            NamedKey::ArrowRight => csi(mods, app, 'C'),
            NamedKey::ArrowLeft => csi(mods, app, 'D'),
            NamedKey::Home => csi(mods, app, 'H'),
            NamedKey::End => csi(mods, app, 'F'),
            NamedKey::PageUp => tilde(mods, 5),
            NamedKey::PageDown => tilde(mods, 6),
            NamedKey::Insert => tilde(mods, 2),
            NamedKey::Delete => tilde(mods, 3),
            NamedKey::F1 => csi_ss3(mods, 'P'),
            NamedKey::F2 => csi_ss3(mods, 'Q'),
            NamedKey::F3 => csi_ss3(mods, 'R'),
            NamedKey::F4 => csi_ss3(mods, 'S'),
            NamedKey::F5 => tilde(mods, 15),
            NamedKey::F6 => tilde(mods, 17),
            NamedKey::F7 => tilde(mods, 18),
            NamedKey::F8 => tilde(mods, 19),
            NamedKey::F9 => tilde(mods, 20),
            NamedKey::F10 => tilde(mods, 21),
            NamedKey::F11 => tilde(mods, 23),
            NamedKey::F12 => tilde(mods, 24),
            NamedKey::Space => return Some(space(m)),
            _ => return text_bytes(ev, m),
        };
        return Some(out);
    }

    if m.control_key() {
        if let Key::Character(s) = &ev.logical_key {
            if let Some(b) = ctrl_byte(s.chars().next()?) {
                return Some(if m.alt_key() { vec![0x1b, b] } else { vec![b] });
            }
        }
    }
    text_bytes(ev, m)
}

fn csi_ss3(mods: u8, f: char) -> Vec<u8> {
    if mods > 1 { format!("\x1b[1;{mods}{f}").into_bytes() } else { format!("\x1bO{f}").into_bytes() }
}

#[cfg(target_os = "linux")]
fn space(m: ModifiersState) -> Vec<u8> {
    if m.control_key() { vec![0] } else if m.alt_key() { b"\x1b ".to_vec() } else { b" ".to_vec() }
}

fn ctrl_byte(c: char) -> Option<u8> {
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '7' | '/' => Some(0x1f),
        '8' => Some(0x7f),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn text_bytes(ev: &KeyEvent, m: ModifiersState) -> Option<Vec<u8>> {
    let t = ev.text.as_ref()?;
    if t.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(t.len() + 1);
    if m.alt_key() {
        out.push(0x1b);
    }
    out.extend_from_slice(t.as_bytes());
    Some(out)
}

// ------------------------------------------------------------ xkb keys --

/// X11 keysyms the terminal cares about (`X11/keysymdef.h`).
pub mod sym {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const HOME: u32 = 0xff50;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const END: u32 = 0xff57;
    pub const INSERT: u32 = 0xff63;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const KP_HOME: u32 = 0xff95;
    pub const KP_LEFT: u32 = 0xff96;
    pub const KP_UP: u32 = 0xff97;
    pub const KP_RIGHT: u32 = 0xff98;
    pub const KP_DOWN: u32 = 0xff99;
    pub const KP_PAGE_UP: u32 = 0xff9a;
    pub const KP_PAGE_DOWN: u32 = 0xff9b;
    pub const KP_END: u32 = 0xff9c;
    pub const KP_INSERT: u32 = 0xff9e;
    pub const KP_DELETE: u32 = 0xff9f;
    pub const F1: u32 = 0xffbe;
    pub const F12: u32 = 0xffc9;
    pub const DELETE: u32 = 0xffff;
    pub const ISO_LEFT_TAB: u32 = 0xfe20;
    pub const SPACE: u32 = 0x20;
}

/// The bytes a key press sends, or `None` for keys that type nothing (modifiers, dead keys).
pub fn encode_key(k: &XKey, mode: TermMode) -> Option<Vec<u8>> {
    use sym::*;
    let m = k.mods;
    let mods = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8;
    let app = mode.contains(TermMode::APP_CURSOR);
    let out = match k.sym {
        RETURN | KP_ENTER => b"\r".to_vec(),
        BACKSPACE => if m.alt { b"\x1b\x7f".to_vec() } else { b"\x7f".to_vec() },
        TAB if !m.shift => b"\t".to_vec(),
        TAB | ISO_LEFT_TAB => b"\x1b[Z".to_vec(),
        ESCAPE => b"\x1b".to_vec(),
        UP | KP_UP => csi(mods, app, 'A'),
        DOWN | KP_DOWN => csi(mods, app, 'B'),
        RIGHT | KP_RIGHT => csi(mods, app, 'C'),
        LEFT | KP_LEFT => csi(mods, app, 'D'),
        HOME | KP_HOME => csi(mods, app, 'H'),
        END | KP_END => csi(mods, app, 'F'),
        PAGE_UP | KP_PAGE_UP => tilde(mods, 5),
        PAGE_DOWN | KP_PAGE_DOWN => tilde(mods, 6),
        INSERT | KP_INSERT => tilde(mods, 2),
        DELETE | KP_DELETE => tilde(mods, 3),
        F1..=F12 => match k.sym - F1 {
            n @ 0..=3 => csi_ss3(mods, ['P', 'Q', 'R', 'S'][n as usize]),
            n => tilde(mods, [15, 17, 18, 19, 20, 21, 23, 24][n as usize - 4]),
        },
        SPACE if m.ctrl => vec![0],
        SPACE if m.alt => b"\x1b ".to_vec(),
        _ => return text_key(k),
    };
    Some(out)
}

/// Letters, digits, symbols and whatever else types text; Control turns them into C0 codes.
fn text_key(k: &XKey) -> Option<Vec<u8>> {
    let m = k.mods;
    if m.ctrl {
        if let Some(b) = char::from_u32(k.sym).filter(|c| c.is_ascii() && !c.is_ascii_control()).and_then(ctrl_byte) {
            return Some(if m.alt { vec![0x1b, b] } else { vec![b] });
        }
    }
    if k.text.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(k.text.len() + 1);
    if m.alt {
        out.push(0x1b);
    }
    out.extend_from_slice(k.text.as_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plat::Mods;

    fn key(sym: u32, text: &str, shift: bool, ctrl: bool, alt: bool) -> XKey {
        XKey { sym, text: text.into(), mods: Mods { shift, ctrl, alt } }
    }
    #[test]
    fn the_plain_arrows_follow_the_cursor_keys_mode() {
        assert_eq!(arrow(false, TermMode::empty()), b"\x1b[D");
        assert_eq!(arrow(true, TermMode::empty()), b"\x1b[C");
        assert_eq!(arrow(false, TermMode::APP_CURSOR), b"\x1bOD");
        assert_eq!(arrow(true, TermMode::APP_CURSOR), b"\x1bOC");
    }

    fn enc(k: XKey) -> Vec<u8> {
        encode_key(&k, TermMode::empty()).unwrap()
    }

    #[test]
    fn text_and_enter() {
        assert_eq!(enc(key(0x61, "a", false, false, false)), b"a");
        assert_eq!(enc(key(0xf1, "\u{f1}", false, false, false)), "\u{f1}".as_bytes());
        assert_eq!(enc(key(sym::RETURN, "\r", false, false, false)), b"\r");
    }

    #[test]
    fn control_and_alt() {
        assert_eq!(enc(key(0x63, "\x03", false, true, false)), [3]);
        assert_eq!(enc(key(0x78, "x", false, false, true)), b"\x1bx");
        assert_eq!(enc(key(0x62, "b", false, true, true)), [0x1b, 2]);
        assert_eq!(enc(key(sym::SPACE, " ", false, true, false)), [0]);
        assert_eq!(enc(key(sym::BACKSPACE, "\x08", false, false, true)), b"\x1b\x7f");
    }

    #[test]
    fn cursor_keys_follow_the_mode() {
        let up = key(sym::UP, "", false, false, false);
        assert_eq!(encode_key(&up, TermMode::empty()).unwrap(), b"\x1b[A");
        assert_eq!(encode_key(&up, TermMode::APP_CURSOR).unwrap(), b"\x1bOA");
        assert_eq!(enc(key(sym::UP, "", true, false, false)), b"\x1b[1;2A");
        assert_eq!(enc(key(sym::RIGHT, "", false, true, false)), b"\x1b[1;5C");
    }

    #[test]
    fn function_navigation_and_tab_keys() {
        assert_eq!(enc(key(sym::F1, "", false, false, false)), b"\x1bOP");
        assert_eq!(enc(key(sym::F1 + 4, "", false, false, false)), b"\x1b[15~");
        assert_eq!(enc(key(sym::F12, "", true, false, false)), b"\x1b[24;2~");
        assert_eq!(enc(key(sym::PAGE_UP, "", false, false, false)), b"\x1b[5~");
        assert_eq!(enc(key(sym::DELETE, "", false, false, false)), b"\x1b[3~");
        assert_eq!(enc(key(sym::TAB, "\t", false, false, false)), b"\t");
        assert_eq!(enc(key(sym::ISO_LEFT_TAB, "", true, false, false)), b"\x1b[Z");
    }

    #[test]
    fn keys_without_text_send_nothing() {
        assert_eq!(encode_key(&key(0xffe1, "", true, false, false), TermMode::empty()), None); // Shift_L
        assert_eq!(encode_key(&key(0xfe51, "", false, false, false), TermMode::empty()), None); // dead_acute
    }
}
