//! Keyboard -> bytes for the PTY.
use alacritty_terminal::term::TermMode;
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

fn tilde(mods: u8, n: u8) -> Vec<u8> {
    if mods > 1 { format!("\x1b[{n};{mods}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
}

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
