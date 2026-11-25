//! A toast, after the reference's `.toast`: a short message at the bottom centre with an optional action
//! button, gone by itself after a few seconds. It is not an overlay: it never takes a key, and the pointer
//! only where it is, so the terminal keeps working under it. The stack (`overlay.rs`) owns it.
//!
//! Its look over time is a pure function of the moment (`alpha`): it fades in over 0.18 s while it rises 6 px,
//! stays, and fades out over the last 0.25 s. While it stays nothing runs: the app's `poll` wakes at
//! `deadline`, once to start the fade and once to take it away; there is no timer while no toast is shown.
// The kit's API is wider than what the app uses yet: the palette, the overview and the dialogs are built on it next (ADR 0025).
#![allow(dead_code)]

use std::time::{Duration, Instant};

use crate::{
    overlay::{Click, Cx, Ink, PX_MENU, POP, Rect, SHADOW_POP, accent, card, ease, fg, inside},
    paint::Paint,
};

/// How long a toast stays, counted from when it was shown (the reference's 6.5 s).
pub const TTL: Duration = Duration::from_millis(6500);
/// The fade in, and the fade out that ends the stay.
pub const FADE_IN: Duration = Duration::from_millis(180);
pub const FADE_OUT: Duration = Duration::from_millis(250);
const RISE: f32 = 6.0;

const H: i32 = 36;
const PAD_L: i32 = 14;
const PAD_R: i32 = 12;
const GAP: i32 = 14;
/// From the window's bottom edge to the toast's (above the status bar).
const BOTTOM: i32 = 40;
/// The room the toast leaves at the window's sides.
const SIDE: i32 = 12;
const RADIUS: f32 = 11.0;
/// The action's click box is this much wider than its text on each side.
const HIT: i32 = 6;

pub struct Toast {
    msg: String,
    action: Option<String>,
    /// The command the action runs.
    id: u32,
    born: Instant,
    /// The pointer is over the action, and the action was pressed and not yet released.
    over: bool,
    armed: bool,
}

/// Where a toast and its action are, in window pixels.
struct Place {
    card: Rect,
    action: Option<Rect>,
    /// How much of the message fits, in pixels.
    msg_room: i32,
}

impl Toast {
    pub fn new(msg: &str, action: Option<&str>, id: u32, now: Instant) -> Toast {
        Toast { msg: msg.to_string(), action: action.map(str::to_string), id, born: now, over: false, armed: false }
    }

    fn age(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.born)
    }

    /// How visible it is: 0 to 1, up with the fade in, down with the fade out. Zero once its time is up.
    pub fn alpha(&self, now: Instant) -> f32 {
        let age = self.age(now);
        if age >= TTL {
            return 0.0;
        }
        let up = (age.as_secs_f32() / FADE_IN.as_secs_f32()).min(1.0);
        let down = ((TTL - age).as_secs_f32() / FADE_OUT.as_secs_f32()).min(1.0);
        ease(up).min(ease(down))
    }

    /// Whether the picture changes with time now: it is fading in or out.
    pub fn animating(&self, now: Instant) -> bool {
        let age = self.age(now);
        age < FADE_IN || (age > TTL - FADE_OUT && age < TTL)
    }

    pub fn gone(&self, now: Instant) -> bool {
        self.age(now) >= TTL
    }

    /// The next moment something has to happen without any input: the fade out starts, then the toast goes.
    /// Never in the past, so the loop never spins on it.
    pub fn deadline(&self, now: Instant) -> Instant {
        let fade = self.born + TTL - FADE_OUT;
        if now < fade { fade } else { self.born + TTL }
    }

    fn place(&self, cx: &Cx) -> Place {
        let t = cx.text;
        let act = self.action.as_deref().map(|a| t.width(a, PX_MENU) + 2);
        let fixed = 2 + PAD_L + PAD_R + act.map_or(0, |a| GAP + a);
        let want = fixed + t.width(&self.msg, PX_MENU);
        let w = want.min(cx.g.w - 2 * SIDE).max(fixed.min(cx.g.w));
        let (x, y) = ((cx.g.w - w) / 2, cx.g.h - BOTTOM - H);
        let action = act.map(|a| (x + w - 1 - PAD_R - a - HIT, y + 4, a + 2 * HIT, H - 8));
        Place { card: (x, y, w, H), action, msg_room: w - fixed }
    }

    /// Whether window pixel `(x, y)` is on the toast.
    pub fn contains(&self, x: f32, y: f32, cx: &Cx) -> bool {
        inside(self.place(cx).card, x, y)
    }

    /// Whether it is on the action button.
    pub fn on_action(&self, x: f32, y: f32, cx: &Cx) -> bool {
        self.place(cx).action.is_some_and(|a| inside(a, x, y))
    }

    /// The pointer moved. Returns whether it is on the toast and whether what is lit changed.
    pub fn hover(&mut self, x: f32, y: f32, cx: &Cx) -> (bool, bool) {
        let over = self.on_action(x, y, cx);
        let changed = over != std::mem::replace(&mut self.over, over);
        (self.contains(x, y, cx), changed)
    }

    /// A press or release on the toast. Returns the command to run when the action was pressed and released.
    pub fn click(&mut self, c: Click, cx: &Cx) -> Option<u32> {
        let on = self.on_action(c.x, c.y, cx);
        if c.pressed {
            self.armed = on;
            return None;
        }
        (std::mem::take(&mut self.armed) && on).then_some(self.id)
    }

    /// Paints the toast at moment `now`.
    pub fn paint(&self, p: &mut Paint, cx: &Cx, now: Instant) {
        let a = self.alpha(now);
        if a <= 0.0 {
            return;
        }
        p.fade = a;
        let pl = self.place(cx);
        let rise = ((1.0 - ease((self.age(now).as_secs_f32() / FADE_IN.as_secs_f32()).min(1.0))) * RISE).round() as i32;
        let (x, y, w, h) = pl.card;
        let y = y + rise;
        card(p, (x, y, w, h), RADIUS, POP, 0.10, SHADOW_POP);
        let cy = y + h / 2;
        let t = cx.text;
        let msg = t.fit(&self.msg, PX_MENU, pl.msg_room);
        t.draw(p, x + 1 + PAD_L, cy, &msg, Ink::new(PX_MENU, fg()));
        if let (Some(act), Some((ax, _, aw, _))) = (&self.action, pl.action) {
            if self.over {
                p.rrect(ax, y + 4, aw, h - 8, 6.0, accent(), 0.12);
            }
            t.draw(p, ax + HIT, cy, act, Ink::new(PX_MENU, accent()).bold());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{Geo, Text};

    const G: Geo = Geo { w: 900, h: 620, square: false };

    fn at(ms: u64, t0: Instant) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn a_toast_fades_in_stays_and_fades_out_and_is_gone_at_its_time() {
        let t0 = Instant::now();
        let t = Toast::new("hi", None, 0, t0);
        assert_eq!(t.alpha(t0), 0.0);
        assert!(t.alpha(at(90, t0)) > 0.3 && t.alpha(at(90, t0)) < 0.7);
        assert_eq!(t.alpha(at(180, t0)), 1.0);
        assert_eq!(t.alpha(at(3000, t0)), 1.0);
        assert!(t.alpha(at(6400, t0)) < 0.5 && t.alpha(at(6400, t0)) > 0.0);
        assert_eq!(t.alpha(at(6500, t0)), 0.0);
        assert!(!t.gone(at(6499, t0)) && t.gone(at(6500, t0)));
    }

    #[test]
    fn it_asks_for_frames_only_while_it_fades() {
        let t0 = Instant::now();
        let t = Toast::new("hi", None, 0, t0);
        assert!(t.animating(at(0, t0)) && t.animating(at(100, t0)));
        assert!(!t.animating(at(180, t0)) && !t.animating(at(3000, t0)));
        assert!(t.animating(at(6300, t0)));
        assert!(!t.animating(at(6500, t0)), "once gone nothing runs");
    }

    #[test]
    fn the_loop_wakes_to_start_the_fade_then_to_take_it_away_and_never_in_the_past() {
        let t0 = Instant::now();
        let t = Toast::new("hi", None, 0, t0);
        assert_eq!(t.deadline(t0), at(6250, t0));
        assert_eq!(t.deadline(at(3000, t0)), at(6250, t0));
        assert_eq!(t.deadline(at(6250, t0)), at(6500, t0));
        assert_eq!(t.deadline(at(6400, t0)), at(6500, t0));
    }

    #[test]
    fn it_sits_at_the_bottom_centre_and_never_wider_than_the_window() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let t = Toast::new("Window closed", Some("Undo"), 5, Instant::now());
        let (x, y, w, h) = t.place(&cx).card;
        assert!((x * 2 + w - 900).abs() <= 1, "centred");
        assert_eq!(y + h, 620 - BOTTOM);
        let long = Toast::new(&"a very long message ".repeat(20), Some("Undo"), 5, Instant::now());
        let narrow = Cx { g: Geo { w: 300, ..G }, text: &text };
        let pl = long.place(&narrow);
        assert!(pl.card.2 <= 300 - 2 * SIDE && pl.card.0 >= SIDE, "{:?}", pl.card);
        // The action stays inside the card, to the right of the message's room.
        let a = pl.action.unwrap();
        assert!(a.0 + a.2 <= pl.card.0 + pl.card.2 && a.0 >= pl.card.0 + PAD_L + pl.msg_room);
    }

    #[test]
    fn the_action_runs_on_release_over_it_and_only_if_it_was_pressed_there() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut t = Toast::new("Window closed", Some("Undo"), 5, Instant::now());
        let a = t.place(&cx).action.unwrap();
        let (ax, ay) = ((a.0 + a.2 / 2) as f32, (a.1 + a.3 / 2) as f32);
        let c = t.place(&cx).card;
        let msg = ((c.0 + 20) as f32, ay);
        let click = |x, y, pressed| Click { x, y, pressed, time: 0 };
        assert_eq!(t.click(click(ax, ay, true), &cx), None);
        assert_eq!(t.click(click(ax, ay, false), &cx), Some(5));
        // Pressed on the message, released on the action: nothing.
        assert_eq!(t.click(click(msg.0, msg.1, true), &cx), None);
        assert_eq!(t.click(click(ax, ay, false), &cx), None);
        // Pressed on the action, dragged off and released: nothing, and the press is forgotten.
        t.click(click(ax, ay, true), &cx);
        assert_eq!(t.click(click(msg.0, msg.1, false), &cx), None);
        assert_eq!(t.click(click(ax, ay, false), &cx), None);
        // The hover is reported once, as a change.
        assert_eq!(t.hover(ax, ay, &cx), (true, true));
        assert_eq!(t.hover(ax + 1.0, ay, &cx), (true, false));
        assert_eq!(t.hover(msg.0, msg.1, &cx), (true, true));
        assert_eq!(t.hover(5.0, 5.0, &cx), (false, false));
    }

    #[test]
    fn a_toast_without_an_action_has_no_button() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let t = Toast::new("Saved", None, 0, Instant::now());
        assert!(t.place(&cx).action.is_none());
        assert!(!t.on_action(450.0, 600.0, &cx));
    }

    #[test]
    fn it_paints_inside_its_card_and_nothing_once_gone() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let t0 = Instant::now();
        let t = Toast::new("Window closed", Some("Undo"), 5, t0);
        let paint = |now| {
            let mut fb = vec![0u32; 900 * 620];
            let mut p = Paint { fb: &mut fb, w: 900, h: 620, ox: 0, oy: 0, fade: 1.0 };
            t.paint(&mut p, &cx, now);
            fb.iter().filter(|&&px| px >> 24 != 0).count()
        };
        assert!(paint(at(1000, t0)) > 2000);
        assert_eq!(paint(at(7000, t0)), 0);
        assert_eq!(paint(t0), 0, "the first frame is invisible");
    }
}
