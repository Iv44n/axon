//! Pixel painting for the overlays drawn over the finished grid (the settings screen, the input bar):
//! antialiased shapes composited by hand into the `wl_shm` framebuffer, which holds premultiplied
//! `Argb8888`. Coordinates are window pixels; `ox`, `oy` is where the window starts in the buffer.
use crate::chrome;

pub const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
pub const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

/// A framebuffer of premultiplied `Argb8888` and the window's origin in it.
pub struct Paint<'a> {
    pub fb: &'a mut [u32],
    pub w: usize,
    pub h: usize,
    pub ox: i32,
    pub oy: i32,
    /// Multiplies every alpha: 1 normally, less while an overlay fades in.
    pub fade: f32,
}

impl Paint<'_> {
    /// Colour `c` at alpha `a` over window pixel `(x, y)`; outside the framebuffer is ignored.
    pub fn put(&mut self, x: i32, y: i32, c: [f32; 3], a: f32) {
        let (fx, fy) = (x + self.ox, y + self.oy);
        let a = a * self.fade;
        if a <= 0.0 || fx < 0 || fy < 0 || fx as usize >= self.w || fy as usize >= self.h {
            return;
        }
        let dst = &mut self.fb[fy as usize * self.w + fx as usize];
        let k = 1.0 - a.min(1.0);
        let ch = |shift: u32, src: f32| {
            let d = (*dst >> shift & 255) as f32;
            ((src * a * 255.0 + d * k + 0.5) as u32).min(255)
        };
        let alpha = ((a * 255.0 + (*dst >> 24) as f32 * k + 0.5) as u32).min(255);
        *dst = alpha << 24 | ch(16, c[0]) << 16 | ch(8, c[1]) << 8 | ch(0, c[2]);
    }

    /// Fills the box `x0..x1` x `y0..y1` with `c` at `a` times `cov(x, y)`, sampled at pixel centres.
    pub fn shape(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: [f32; 3], a: f32, cov: &dyn Fn(f32, f32) -> f32) {
        for y in y0..y1 {
            for x in x0..x1 {
                let k = cov(x as f32 + 0.5, y as f32 + 0.5);
                if k > 0.0 {
                    self.put(x, y, c, a * k);
                }
            }
        }
    }

    pub fn rrect(&mut self, x: i32, y: i32, w: i32, h: i32, r: f32, c: [f32; 3], a: f32) {
        let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
        self.shape(x, y, x + w, y + h, c, a, &|px, py| (0.5 - chrome::rounded_rect(px - fx, py - fy, fw, fh, r.min(fh / 2.0))).clamp(0.0, 1.0));
    }

    /// A 1 px outline of the rounded rectangle.
    pub fn outline(&mut self, x: i32, y: i32, w: i32, h: i32, r: f32, c: [f32; 3], a: f32) {
        let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
        self.shape(x, y, x + w, y + h, c, a, &|px, py| {
            let d = chrome::rounded_rect(px - fx, py - fy, fw, fh, r.min(fh / 2.0));
            (0.5 - d).clamp(0.0, 1.0) * (d + 1.5).clamp(0.0, 1.0)
        });
    }

    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, c: [f32; 3], a: f32) {
        let (x0, y0, x1, y1) = ((cx - r - 1.0) as i32, (cy - r - 1.0) as i32, (cx + r + 2.0) as i32, (cy + r + 2.0) as i32);
        self.shape(x0, y0, x1, y1, c, a, &|px, py| (0.5 - ((px - cx).hypot(py - cy) - r)).clamp(0.0, 1.0));
    }

    /// A `<` (`dir` -1) or `>` (+1) about 10 px tall, 1.4 px stroke, centred on `(cx, cy)`.
    pub fn chevron(&mut self, cx: i32, cy: i32, dir: i32, c: [f32; 3], a: f32) {
        let (cx, cy, d) = (cx as f32, cy as f32, dir as f32);
        let tip = (cx + 2.0 * d, cy);
        let (top, bot) = ((cx - 2.5 * d, cy - 5.0), (cx - 2.5 * d, cy + 5.0));
        self.shape(cx as i32 - 7, cy as i32 - 8, cx as i32 + 8, cy as i32 + 9, c, a, &|px, py| {
            let dist = seg(px, py, top, tip).min(seg(px, py, tip, bot));
            (1.2 - dist).clamp(0.0, 1.0)
        });
    }
}

pub fn seg(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - a.0 - dx * t).hypot(py - a.1 - dy * t)
}

