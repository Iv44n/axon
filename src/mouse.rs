//! Mouse reports for programs that ask for them (vim, tmux, htop, less): xterm modes 1000, 1002, 1003
//! with the SGR (1006), UTF-8 (1005) or classic X10 encoding. Pure: no state, no I/O.
use crate::xkb::Mods;
use alacritty_terminal::term::TermMode;

/// What happened to which button. Buttons are the xterm numbers: 0 left, 1 middle, 2 right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    Press(u8),
    Release(u8),
    /// The pointer moved to another cell, with a button held or not.
    Move(Option<u8>),
    /// One wheel notch: `true` away from the user (up).
    Wheel(bool),
}

/// A report, in place: the longest one is 6 bytes (classic), 19 (SGR with 4-digit coordinates) or 13 (UTF-8).
pub struct Report {
    buf: [u8; 24],
    len: usize,
}

impl Report {
    pub fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    fn push(&mut self, b: u8) {
        self.buf[self.len] = b;
        self.len += 1;
    }

    fn push_all(&mut self, s: &[u8]) {
        self.buf[self.len..self.len + s.len()].copy_from_slice(s);
        self.len += s.len();
    }

    fn push_num(&mut self, mut n: usize) {
        let mut digits = [0u8; 8];
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        self.push_all(&digits[i..]);
    }

    /// A classic coordinate: one byte, or a UTF-8 character in mode 1005.
    fn push_coord(&mut self, v: usize, utf8: bool) -> Option<()> {
        let v = v + 32;
        if utf8 {
            let c = char::from_u32(v as u32).filter(|_| v <= 2047)?;
            let mut tmp = [0u8; 4];
            self.push_all(c.encode_utf8(&mut tmp).as_bytes());
        } else {
            self.push(u8::try_from(v).ok().filter(|&b| b != 0xff)?);
        }
        Some(())
    }
}

/// Whether the program asked for mouse reports at all.
pub fn reporting(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
}

/// The bytes for `ev` at grid cell `(col, row)` (0-based), or `None` when the mode does not want it
/// or the cell cannot be written in the active encoding.
pub fn encode(mode: TermMode, ev: Event, mods: Mods, col: usize, row: usize) -> Option<Report> {
    if !reporting(mode) {
        return None;
    }
    let sgr = mode.contains(TermMode::SGR_MOUSE);
    // The button part of the code, and whether this is a release.
    let (base, release) = match ev {
        Event::Press(b) => (b, false),
        // Classic encodings cannot say which button was released.
        Event::Release(b) => (if sgr { b } else { 3 }, true),
        Event::Move(held) => {
            let wanted = if held.is_some() {
                mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
            } else {
                mode.contains(TermMode::MOUSE_MOTION)
            };
            if !wanted {
                return None;
            }
            (32 + held.unwrap_or(3), false)
        }
        Event::Wheel(up) => (64 + !up as u8, false),
    };
    let code = base + 4 * mods.shift as u8 + 8 * mods.alt as u8 + 16 * mods.ctrl as u8;

    let mut r = Report { buf: [0; 24], len: 0 };
    if sgr {
        r.push_all(b"\x1b[<");
        r.push_num(code as usize);
        r.push(b';');
        r.push_num(col + 1);
        r.push(b';');
        r.push_num(row + 1);
        r.push(if release { b'm' } else { b'M' });
    } else {
        let utf8 = mode.contains(TermMode::UTF8_MOUSE);
        r.push_all(b"\x1b[M");
        r.push_coord(code as usize, false)?;
        r.push_coord(col + 1, utf8)?;
        r.push_coord(row + 1, utf8)?;
    }
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
    const CLICK: TermMode = TermMode::MOUSE_REPORT_CLICK;

    fn bytes(mode: TermMode, ev: Event, mods: Mods, col: usize, row: usize) -> Option<Vec<u8>> {
        encode(mode, ev, mods, col, row).map(|r| r.bytes().to_vec())
    }

    #[test]
    fn nothing_is_sent_unless_the_program_asked() {
        assert!(bytes(TermMode::SGR_MOUSE, Event::Press(0), NONE, 0, 0).is_none());
        assert!(!reporting(TermMode::empty()));
        assert!(reporting(CLICK));
    }

    #[test]
    fn sgr_press_and_release_name_the_button_and_the_cell() {
        let m = CLICK | TermMode::SGR_MOUSE;
        assert_eq!(bytes(m, Event::Press(0), NONE, 0, 0).unwrap(), b"\x1b[<0;1;1M");
        assert_eq!(bytes(m, Event::Release(0), NONE, 0, 0).unwrap(), b"\x1b[<0;1;1m");
        assert_eq!(bytes(m, Event::Press(2), NONE, 9, 4).unwrap(), b"\x1b[<2;10;5M");
        assert_eq!(bytes(m, Event::Release(1), NONE, 2000, 1000).unwrap(), b"\x1b[<1;2001;1001m");
    }

    #[test]
    fn modifiers_add_to_the_code() {
        let m = CLICK | TermMode::SGR_MOUSE;
        let all = Mods { shift: true, ctrl: true, alt: true };
        assert_eq!(bytes(m, Event::Press(0), all, 0, 0).unwrap(), b"\x1b[<28;1;1M");
        let ctrl = Mods { ctrl: true, ..NONE };
        assert_eq!(bytes(m, Event::Press(2), ctrl, 0, 0).unwrap(), b"\x1b[<18;1;1M");
    }

    #[test]
    fn the_wheel_is_buttons_64_and_65_and_never_released() {
        let m = CLICK | TermMode::SGR_MOUSE;
        assert_eq!(bytes(m, Event::Wheel(true), NONE, 3, 3).unwrap(), b"\x1b[<64;4;4M");
        assert_eq!(bytes(m, Event::Wheel(false), NONE, 3, 3).unwrap(), b"\x1b[<65;4;4M");
    }

    #[test]
    fn motion_follows_the_mode() {
        let drag = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE;
        let any = TermMode::MOUSE_MOTION | TermMode::SGR_MOUSE;
        let click = CLICK | TermMode::SGR_MOUSE;
        assert!(bytes(click, Event::Move(Some(0)), NONE, 1, 1).is_none());
        assert_eq!(bytes(drag, Event::Move(Some(0)), NONE, 1, 1).unwrap(), b"\x1b[<32;2;2M");
        assert!(bytes(drag, Event::Move(None), NONE, 1, 1).is_none());
        assert_eq!(bytes(any, Event::Move(None), NONE, 1, 1).unwrap(), b"\x1b[<35;2;2M");
        assert_eq!(bytes(any, Event::Move(Some(2)), NONE, 1, 1).unwrap(), b"\x1b[<34;2;2M");
    }

    #[test]
    fn the_classic_encoding_adds_32_and_loses_the_released_button() {
        assert_eq!(bytes(CLICK, Event::Press(0), NONE, 0, 0).unwrap(), b"\x1b[M !!");
        assert_eq!(bytes(CLICK, Event::Release(2), NONE, 0, 0).unwrap(), b"\x1b[M#!!");
        assert_eq!(bytes(CLICK, Event::Press(2), NONE, 9, 4).unwrap(), b"\x1b[M\"*%");
    }

    #[test]
    fn the_classic_encoding_stops_at_column_222_and_utf8_goes_further() {
        assert!(bytes(CLICK, Event::Press(0), NONE, 221, 0).is_some());
        assert!(bytes(CLICK, Event::Press(0), NONE, 222, 0).is_none());
        let utf8 = CLICK | TermMode::UTF8_MOUSE;
        // Column index 299 is column 300; 300 + 32 = 332 = U+014C.
        assert_eq!(bytes(utf8, Event::Press(0), NONE, 299, 0).unwrap(), "\x1b[M \u{14c}!".as_bytes());
        assert!(bytes(utf8, Event::Press(0), NONE, 3000, 0).is_none());
    }
}
