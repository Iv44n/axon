//! Keyboard layouts: `libxkbcommon`, loaded at run time with `dlopen`.
//!
//! The compositor sends the layout as xkb keymap text; turning (key, modifiers) into a keysym and
//! text, with dead keys and compose sequences, is what this library does. Nothing is linked: if
//! `libxkbcommon.so.0` is missing the window still opens, it just cannot type. The plan is an own
//! parser of the keymap text (`knowledge/plan/software-renderer.md`, stage C); until then this is
//! the one place that knows the C API, behind a handful of plain functions.
//!
//! Every call into the library is `unsafe`: this module is allowed it (ADR 0049), and the own parser takes it away.
#![allow(unsafe_code)]
use crate::plat::{Key, Mods};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr::NonNull,
};

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

const RTLD_NOW: c_int = 2;
const KEYMAP_FORMAT_TEXT_V1: c_int = 1;
const STATE_MODS_EFFECTIVE: c_int = 8;
const MOD_INVALID: u32 = 0xffff_ffff;
const COMPOSE_FEED_ACCEPTED: c_int = 1;
const COMPOSING: c_int = 1;
const COMPOSE_COMPOSED: c_int = 2;
const COMPOSE_CANCELLED: c_int = 3;
/// Evdev key codes are offset by 8 in xkb.
const EVDEV_OFFSET: u32 = 8;

type Ptr = *mut c_void;

/// The functions we use, resolved once.
struct Fns {
    context_new: unsafe extern "C" fn(c_int) -> Ptr,
    keymap_from_string: unsafe extern "C" fn(Ptr, *const c_char, c_int, c_int) -> Ptr,
    keymap_unref: unsafe extern "C" fn(Ptr),
    keymap_repeats: unsafe extern "C" fn(Ptr, u32) -> c_int,
    mod_index: unsafe extern "C" fn(Ptr, *const c_char) -> u32,
    state_new: unsafe extern "C" fn(Ptr) -> Ptr,
    state_unref: unsafe extern "C" fn(Ptr),
    update_mask: unsafe extern "C" fn(Ptr, u32, u32, u32, u32, u32, u32) -> c_int,
    one_sym: unsafe extern "C" fn(Ptr, u32) -> u32,
    utf8: unsafe extern "C" fn(Ptr, u32, *mut c_char, usize) -> c_int,
    mod_active: unsafe extern "C" fn(Ptr, u32, c_int) -> c_int,
    compose_table: unsafe extern "C" fn(Ptr, *const c_char, c_int) -> Ptr,
    compose_state_new: unsafe extern "C" fn(Ptr, c_int) -> Ptr,
    compose_feed: unsafe extern "C" fn(Ptr, u32) -> c_int,
    compose_status: unsafe extern "C" fn(Ptr) -> c_int,
    compose_utf8: unsafe extern "C" fn(Ptr, *mut c_char, usize) -> c_int,
    compose_reset: unsafe extern "C" fn(Ptr),
}

fn load_fns() -> Option<Fns> {
    // SAFETY: plain dlopen/dlsym of a system library; every symbol is cast to the signature
    // documented in xkbcommon.h and xkbcommon-compose.h, and a missing one aborts the load.
    unsafe {
        let mut lib = std::ptr::null_mut();
        for name in [c"libxkbcommon.so.0", c"libxkbcommon.so"] {
            lib = dlopen(name.as_ptr(), RTLD_NOW);
            if !lib.is_null() {
                break;
            }
        }
        if lib.is_null() {
            return None;
        }
        macro_rules! sym {
            ($name:literal) => {{
                let p = dlsym(lib, concat!($name, "\0").as_ptr().cast());
                if p.is_null() {
                    return None;
                }
                std::mem::transmute(p)
            }};
        }
        Some(Fns {
            context_new: sym!("xkb_context_new"),
            keymap_from_string: sym!("xkb_keymap_new_from_string"),
            keymap_unref: sym!("xkb_keymap_unref"),
            keymap_repeats: sym!("xkb_keymap_key_repeats"),
            mod_index: sym!("xkb_keymap_mod_get_index"),
            state_new: sym!("xkb_state_new"),
            state_unref: sym!("xkb_state_unref"),
            update_mask: sym!("xkb_state_update_mask"),
            one_sym: sym!("xkb_state_key_get_one_sym"),
            utf8: sym!("xkb_state_key_get_utf8"),
            mod_active: sym!("xkb_state_mod_index_is_active"),
            compose_table: sym!("xkb_compose_table_new_from_locale"),
            compose_state_new: sym!("xkb_compose_state_new"),
            compose_feed: sym!("xkb_compose_state_feed"),
            compose_status: sym!("xkb_compose_state_get_status"),
            compose_utf8: sym!("xkb_compose_state_get_utf8"),
            compose_reset: sym!("xkb_compose_state_reset"),
        })
    }
}

/// Where a keymap's modifier bits live (`Shift`, `Control`, `Mod1` = Alt).
#[derive(Clone, Copy)]
struct ModIdx {
    shift: u32,
    ctrl: u32,
    alt: u32,
}

pub struct Xkb {
    f: Fns,
    ctx: NonNull<c_void>,
    keymap: Option<NonNull<c_void>>,
    state: Option<NonNull<c_void>>,
    compose: Option<NonNull<c_void>>,
    idx: ModIdx,
}

impl Xkb {
    /// Loads the library and the compose table of the user's locale. `None`: no libxkbcommon.
    pub fn load() -> Option<Xkb> {
        let f = load_fns()?;
        // SAFETY: the function pointers come from `load_fns`.
        let ctx = NonNull::new(unsafe { (f.context_new)(0) })?;
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
            .unwrap_or_else(|| "C".into());
        let locale = CString::new(locale).ok()?;
        // SAFETY: `ctx` and `locale` are valid; a null table (locale without compose data) is fine.
        let table = unsafe { (f.compose_table)(ctx.as_ptr(), locale.as_ptr(), 0) };
        let compose = NonNull::new(if table.is_null() { table } else { unsafe { (f.compose_state_new)(table, 0) } });
        Some(Xkb { f, ctx, keymap: None, state: None, compose, idx: ModIdx { shift: 0, ctrl: 2, alt: 3 } })
    }

    /// Installs the keymap text the compositor sent. Returns false if it does not compile.
    pub fn set_keymap(&mut self, text: &str) -> bool {
        let Ok(text) = CString::new(text.trim_end_matches('\0')) else { return false };
        // SAFETY: valid context and NUL-terminated text.
        let keymap = unsafe { (self.f.keymap_from_string)(self.ctx.as_ptr(), text.as_ptr(), KEYMAP_FORMAT_TEXT_V1, 0) };
        let Some(keymap) = NonNull::new(keymap) else { return false };
        // SAFETY: `keymap` is valid; the old keymap and state are released after the new ones exist.
        let state = unsafe { (self.f.state_new)(keymap.as_ptr()) };
        let Some(state) = NonNull::new(state) else {
            unsafe { (self.f.keymap_unref)(keymap.as_ptr()) };
            return false;
        };
        let index = |name: &CStr| match unsafe { (self.f.mod_index)(keymap.as_ptr(), name.as_ptr()) } {
            MOD_INVALID => None,
            i => Some(i),
        };
        self.idx = ModIdx {
            shift: index(c"Shift").unwrap_or(0),
            ctrl: index(c"Control").unwrap_or(2),
            alt: index(c"Mod1").unwrap_or(3),
        };
        self.release();
        (self.keymap, self.state) = (Some(keymap), Some(state));
        true
    }

    fn release(&mut self) {
        // SAFETY: pointers were created by this library and are dropped exactly once.
        unsafe {
            if let Some(s) = self.state.take() {
                (self.f.state_unref)(s.as_ptr());
            }
            if let Some(k) = self.keymap.take() {
                (self.f.keymap_unref)(k.as_ptr());
            }
        }
    }

    /// `wl_keyboard.modifiers`: the compositor owns modifier state, we mirror it.
    pub fn update_mods(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if let Some(s) = self.state {
            // SAFETY: valid state.
            unsafe { (self.f.update_mask)(s.as_ptr(), depressed, latched, locked, 0, 0, group) };
        }
    }

    pub fn mods(&self) -> Mods {
        let Some(s) = self.state else { return Mods::default() };
        let on = |i: u32| unsafe { (self.f.mod_active)(s.as_ptr(), i, STATE_MODS_EFFECTIVE) > 0 };
        Mods { shift: on(self.idx.shift), ctrl: on(self.idx.ctrl), alt: on(self.idx.alt) }
    }

    /// Whether holding `key` (an evdev code) repeats it.
    pub fn repeats(&self, key: u32) -> bool {
        // SAFETY: valid keymap.
        self.keymap.is_some_and(|k| unsafe { (self.f.keymap_repeats)(k.as_ptr(), key + EVDEV_OFFSET) } > 0)
    }

    /// Resolves a key press (`key` is an evdev code). `None` while a compose sequence is under way
    /// or was cancelled: the key was consumed.
    pub fn press(&mut self, key: u32) -> Option<Key> {
        let state = self.state?;
        let code = key + EVDEV_OFFSET;
        // SAFETY: valid state; the buffer length is passed along.
        let sym = unsafe { (self.f.one_sym)(state.as_ptr(), code) };
        let mods = self.mods();
        let mut buf = [0 as c_char; 64];
        if let Some(c) = self.compose {
            // SAFETY: valid compose state.
            let fed = unsafe { (self.f.compose_feed)(c.as_ptr(), sym) } == COMPOSE_FEED_ACCEPTED;
            if fed {
                match unsafe { (self.f.compose_status)(c.as_ptr()) } {
                    COMPOSING => return None,
                    COMPOSE_CANCELLED => {
                        unsafe { (self.f.compose_reset)(c.as_ptr()) };
                        return None;
                    }
                    COMPOSE_COMPOSED => {
                        let n = unsafe { (self.f.compose_utf8)(c.as_ptr(), buf.as_mut_ptr(), buf.len()) };
                        unsafe { (self.f.compose_reset)(c.as_ptr()) };
                        return Some(Key { sym, text: text_of(&buf, n), mods });
                    }
                    _ => {}
                }
            }
        }
        let n = unsafe { (self.f.utf8)(state.as_ptr(), code, buf.as_mut_ptr(), buf.len()) };
        Some(Key { sym, text: text_of(&buf, n), mods })
    }
}

fn text_of(buf: &[c_char], n: c_int) -> String {
    if n <= 0 || n as usize >= buf.len() {
        return String::new();
    }
    // SAFETY: c_char and u8 have the same layout; `n` bytes were written.
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), n as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

impl Drop for Xkb {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_Q: u32 = 16;
    const KEY_A: u32 = 30;
    const KEY_LEFTBRACE: u32 = 26; // the dead acute on latam
    const KEY_ENTER: u32 = 28;
    const MOD5: u32 = 0x80;
    const SHIFT: u32 = 1;

    /// A compiled layout, or None where libxkbcommon or the xkb data is not installed.
    fn layout(symbols: &str) -> Option<Xkb> {
        let mut x = Xkb::load()?;
        let text = format!(
            "xkb_keymap {{ xkb_keycodes {{ include \"evdev+aliases(qwerty)\" }}; xkb_types {{ include \"complete\" }}; \
             xkb_compat {{ include \"complete\" }}; xkb_symbols {{ include \"{symbols}\" }}; }};"
        );
        x.set_keymap(&text).then_some(x)
    }

    #[test]
    fn plain_keys_type_their_text() {
        let Some(mut x) = layout("pc+us") else { return };
        assert_eq!(x.press(KEY_A).unwrap().text, "a");
        x.update_mods(SHIFT, 0, 0, 0);
        let k = x.press(KEY_A).unwrap();
        assert_eq!((k.text.as_str(), k.mods.shift), ("A", true));
        assert_eq!(x.press(KEY_ENTER).unwrap().sym, 0xff0d);
    }

    #[test]
    fn altgr_reaches_the_third_level() {
        let Some(mut x) = layout("pc+latam") else { return };
        x.update_mods(MOD5, 0, 0, 0);
        assert_eq!(x.press(KEY_Q).unwrap().text, "@");
        assert!(!x.mods().alt, "AltGr is not Alt");
    }

    #[test]
    fn dead_keys_compose() {
        let Some(mut x) = layout("pc+latam") else { return };
        if x.compose.is_none() {
            return; // no compose table for this locale
        }
        assert_eq!(x.press(KEY_LEFTBRACE), None, "the dead acute is consumed");
        assert_eq!(x.press(KEY_A).unwrap().text, "\u{e1}");
        assert_eq!(x.press(KEY_A).unwrap().text, "a", "compose state is reset");
    }

    #[test]
    fn letters_repeat_and_modifiers_do_not() {
        let Some(x) = layout("pc+us") else { return };
        assert!(x.repeats(KEY_A));
        assert!(!x.repeats(42), "left shift");
    }

    #[test]
    fn a_broken_keymap_is_refused_and_the_old_one_kept() {
        let Some(mut x) = layout("pc+us") else { return };
        assert!(!x.set_keymap("not a keymap"));
        assert_eq!(x.press(KEY_A).unwrap().text, "a");
    }
}
