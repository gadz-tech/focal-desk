//! The frame's pixels (2026-09-04, REQUESTS §3): a software rasterizer
//! for one band of a [`Frame`], pure so it runs — and is tested — on
//! Linux CI; the adapter blits the result into a per-pixel-alpha layered
//! window.
//!
//! The look, from Ryan's pins on the mock: frosted glass as **one smooth
//! tint that varies gently** — no sheen, no horizon line — with the
//! inner edge **fading into the window** rather than a hard line; the
//! neon is the wires' business: a thin bright strip on the thick side
//! (the port) with a soft glow, and a silver ring at each connection
//! hole while it is free. Rounded corners come from coverage, not from a
//! window region: a fully transparent pixel of a layered window is not
//! part of it, so the corners are click-through for free. The active
//! window's frame (the OS foreground, `Event::Foreground`) is a touch
//! brighter — the one thing that event is for.
//!
//! This is the geometry-and-tint stage. Real blur behind the glass (the
//! DWM backdrop) cannot be combined with a per-pixel-alpha layered
//! window; it needs a DirectComposition-backed host and is the next step.

use crate::geometry::Rect;
use crate::tab::Frame;

/// A premultiplied BGRA pixel buffer, `w` × `h`, top-down rows — the
/// layout a 32-bit DIB and `UpdateLayeredWindow` expect: each `u32` is
/// `0xAARRGGBB`, which in memory (little-endian) is B, G, R, A.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u32>,
}

impl Pixels {
    /// The alpha of the pixel at `(x, y)`, 0–255.
    pub fn alpha(&self, x: usize, y: usize) -> u8 {
        (self.data[y * self.w + x] >> 24) as u8
    }

    /// The un-premultiplied `(r, g, b)` of the pixel at `(x, y)`, each
    /// 0–255; black where the pixel is fully transparent.
    pub fn rgb(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let p = self.data[y * self.w + x];
        let a = (p >> 24) as f32;
        if a == 0.0 {
            return (0, 0, 0);
        }
        let un = |c: u32| ((c as f32 / a) * 255.0).round().min(255.0) as u8;
        (un((p >> 16) & 0xFF), un((p >> 8) & 0xFF), un(p & 0xFF))
    }
}

/// The glass at the top of a frame, `(r, g, b)` 0–1.
const TINT_TOP: [f32; 3] = [0xE4 as f32 / 255.0, 0xEE as f32 / 255.0, 0xF7 as f32 / 255.0];
/// The glass at the bottom of a frame.
const TINT_BOTTOM: [f32; 3] = [0x93 as f32 / 255.0, 0xA9 as f32 / 255.0, 0xBE as f32 / 255.0];
/// Glass opacity at the top and at the bottom (the one gentle gradient).
const ALPHA_TOP: f32 = 0.30;
const ALPHA_BOTTOM: f32 = 0.16;
/// Extra opacity for the active window's frame.
const ACTIVE_BOOST: f32 = 0.10;
/// The faint light edge along the outside of the glass.
const EDGE_ALPHA: f32 = 0.22;
/// The neon (cyan) of the strip and a lit hole.
const NEON: [f32; 3] = [0x2F as f32 / 255.0, 0xE6 as f32 / 255.0, 1.0];
/// The silver of a free hole's ring.
const SILVER: [f32; 3] = [0xC9 as f32 / 255.0, 0xCD as f32 / 255.0, 0xD2 as f32 / 255.0];

/// Signed distance from a point to a rounded rectangle: negative inside,
/// zero on the edge, positive outside, in pixels.
pub fn sd_round_rect(px: f32, py: f32, r: Rect, radius: f32) -> f32 {
    let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let (cx, cy) = r.center();
    let (hx, hy) = (r.w / 2.0 - radius, r.h / 2.0 - radius);
    let (qx, qy) = ((px - cx).abs() - hx, (py - cy).abs() - hy);
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    let inside = qx.max(qy).min(0.0);
    outside + inside - radius
}

/// Linear interpolation between two colors.
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Smooth 0→1 ramp of `x` between `lo` and `hi`.
fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One straight-alpha working pixel.
#[derive(Clone, Copy)]
struct Px {
    rgb: [f32; 3],
    a: f32,
}

impl Px {
    /// Composite `color` at `alpha` over this pixel (straight alpha).
    fn over(&mut self, color: [f32; 3], alpha: f32) {
        let alpha = alpha.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let out_a = alpha + self.a * (1.0 - alpha);
        if out_a <= 0.0 {
            return;
        }
        for c in 0..3 {
            self.rgb[c] = (color[c] * alpha + self.rgb[c] * self.a * (1.0 - alpha)) / out_a;
        }
        self.a = out_a;
    }

    /// Pack as premultiplied `0xAARRGGBB`.
    fn packed(self) -> u32 {
        let a = self.a.clamp(0.0, 1.0);
        let ch = |c: f32| ((c.clamp(0.0, 1.0) * a * 255.0).round() as u32) & 0xFF;
        (((a * 255.0).round() as u32) << 24) | (ch(self.rgb[0]) << 16) | (ch(self.rgb[1]) << 8) | ch(self.rgb[2])
    }
}

/// Paint the part of `frame` that lies in `band` (both in the same pixel
/// space) into a `band.w` × `band.h` buffer; `active` brightens the
/// glass a touch. Pixels outside the rounded ring stay fully transparent.
pub fn paint_band(frame: &Frame, band: Rect, active: bool) -> Pixels {
    let w = band.w.round().max(0.0) as usize;
    let h = band.h.round().max(0.0) as usize;
    let mut data = Vec::with_capacity(w * h);
    let fade_px = frame.fade_px();
    let strip_glow = frame.strip.map(|s| s.w.min(s.h) * 2.0).unwrap_or(0.0);
    let ring_w = (frame.hole_r * 0.28).max(1.5);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (band.x + x as f32 + 0.5, band.y + y as f32 + 0.5);
            let d_out = sd_round_rect(px, py, frame.outer, frame.radius_out);
            let d_in = sd_round_rect(px, py, frame.inner, frame.radius_in);
            // Coverage of the ring: inside the outer edge, outside the window.
            let cov = (0.5 - d_out).clamp(0.0, 1.0) * (d_in + 0.5).clamp(0.0, 1.0);
            if cov <= 0.0 {
                data.push(0);
                continue;
            }
            let mut p = Px { rgb: [0.0; 3], a: 0.0 };
            // The glass: one gentle vertical gradient, fading into the window.
            let t = ((py - frame.outer.y) / frame.outer.h.max(1.0)).clamp(0.0, 1.0);
            let fade = smoothstep(0.0, fade_px, d_in);
            let mut glass_a = ALPHA_TOP + (ALPHA_BOTTOM - ALPHA_TOP) * t;
            if active {
                glass_a += ACTIVE_BOOST;
            }
            p.over(mix(TINT_TOP, TINT_BOTTOM, t), cov * fade * glass_a);
            // A faint light edge just inside the outer rim.
            let edge = (1.5 - (d_out + 1.0).abs()).clamp(0.0, 1.0);
            p.over([1.0, 1.0, 1.0], cov * fade * edge * EDGE_ALPHA);
            // The strip: a soft glow, then the bright core.
            if let Some(s) = frame.strip {
                let ds = sd_round_rect(px, py, s, s.w.min(s.h) / 2.0);
                if ds < strip_glow {
                    let g = ds.max(0.0) / strip_glow.max(0.001);
                    p.over(NEON, cov * 0.35 * (1.0 - g) * (1.0 - g));
                    p.over(NEON, cov * 0.9 * (0.5 - ds).clamp(0.0, 1.0));
                }
            }
            // The holes: a silver ring each while free.
            for &(hx, hy) in &frame.holes {
                let dist = (px - hx).hypot(py - hy);
                let ring = (ring_w / 2.0 + 0.5 - (dist - (frame.hole_r - ring_w / 2.0)).abs())
                    .clamp(0.0, 1.0);
                p.over(SILVER, cov * 0.75 * ring);
            }
            data.push(p.packed());
        }
    }
    Pixels { w, h, data }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::layout::{self, FOCAL};
    use crate::tab::{frame, Edge};

    /// The frame and bands of the left-top window (thick side: right).
    fn left_top() -> (Frame, [Rect; 4]) {
        let cfg = Config::default();
        let f = frame(&cfg, layout::window_rect(&cfg, layout::slot_from_name("left-top").unwrap()), false);
        let bands = f.bands();
        (f, bands)
    }

    /// The pixel of `band` at frame-space point `(x, y)`.
    fn at(band: Rect, x: f32, y: f32) -> (usize, usize) {
        ((x - band.x).floor() as usize, (y - band.y).floor() as usize)
    }

    #[test]
    fn signed_distance_is_negative_inside_and_zero_on_the_edge() {
        let r = Rect::new(10.0, 10.0, 100.0, 50.0);
        assert!(sd_round_rect(60.0, 35.0, r, 8.0) < 0.0);
        assert!((sd_round_rect(10.0, 35.0, r, 8.0)).abs() < 1e-4, "on the left edge");
        assert!(sd_round_rect(5.0, 35.0, r, 8.0) > 0.0);
        // The corner is rounded away: the rect's own corner point is outside.
        assert!(sd_round_rect(10.0, 10.0, r, 8.0) > 0.0);
        assert!(sd_round_rect(10.0, 10.0, r, 0.0).abs() < 1e-4, "square corner");
    }

    #[test]
    fn buffers_are_band_sized_and_corners_are_transparent() {
        let (f, bands) = left_top();
        let top = paint_band(&f, bands[0], false);
        assert_eq!((top.w, top.h), (bands[0].w.round() as usize, bands[0].h.round() as usize));
        assert_eq!(top.data.len(), top.w * top.h);
        // The band's own corner pixel is outside the rounded outer edge.
        assert_eq!(top.alpha(0, 0), 0, "rounded corner is click-through");
        assert_eq!(top.alpha(top.w - 1, 0), 0);
        // Mid-band, on the ring, there is glass.
        let (x, y) = at(bands[0], f.inner.center().0, bands[0].y + 1.0);
        assert!(top.alpha(x, y) > 0);
        assert_eq!(f.thick, Some(Edge::Right));
    }

    #[test]
    fn the_inner_edge_fades_into_the_window() {
        let (f, bands) = left_top();
        // The left band is slim: compare a pixel touching the window with
        // one at the band's outer side.
        let left = paint_band(&f, bands[2], false);
        let cy = f.inner.center().1;
        let (x_near, y) = at(bands[2], f.inner.x - 1.0, cy);
        let (x_far, _) = at(bands[2], f.outer.x + 3.0, cy);
        assert!(left.alpha(x_near, y) < left.alpha(x_far, y), "near {} far {}", left.alpha(x_near, y), left.alpha(x_far, y));
        assert!(left.alpha(x_far, y) > 20, "the glass is visible: {}", left.alpha(x_far, y));
        assert!(left.alpha(x_far, y) < 128, "…and translucent: {}", left.alpha(x_far, y));
    }

    #[test]
    fn the_strip_is_neon_and_the_holes_are_silver() {
        let (f, bands) = left_top();
        let right = paint_band(&f, bands[3], false);
        let s = f.strip.unwrap();
        let (sx, sy) = s.center();
        let (x, y) = at(bands[3], sx, sy);
        let (r, g, b) = right.rgb(x, y);
        assert!(g > 180 && b > 200 && r < 120, "strip centre is cyan, got {r},{g},{b}");
        assert!(right.alpha(x, y) > 200, "strip is bright: {}", right.alpha(x, y));
        // A point on a free hole's ring is silver (grey, all channels alike).
        let (hx, hy) = f.holes[0];
        let ring_r = f.hole_r - (f.hole_r * 0.28).max(1.5) / 2.0;
        let (x, y) = at(bands[3], hx, hy - ring_r);
        let (r, g, b) = right.rgb(x, y);
        assert!(r > 150 && g > 150 && b > 150, "ring is light, got {r},{g},{b}");
        assert!((r as i32 - b as i32).abs() < 25, "ring is grey, got {r},{g},{b}");
        // The hole's centre is plain glass again (a ring, not a disc).
        let (x, y) = at(bands[3], hx, hy);
        assert!(right.alpha(x, y) < 128);
        // The slim left band carries no neon at all.
        let left = paint_band(&f, bands[2], false);
        let neon = left.data.iter().filter(|&&p| {
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            b > 150 && g > 120 && r < 60
        }).count();
        assert_eq!(neon, 0, "no neon off the thick side");
    }

    #[test]
    fn the_active_frame_is_brighter_and_the_stage_has_no_furniture() {
        let (f, bands) = left_top();
        let cy = f.inner.center().1;
        let (x, y) = at(bands[2], f.outer.x + 3.0, cy);
        let idle = paint_band(&f, bands[2], false).alpha(x, y);
        let active = paint_band(&f, bands[2], true).alpha(x, y);
        assert!(active > idle, "active {active} idle {idle}");
        let cfg = Config::default();
        let stage = frame(&cfg, layout::window_rect(&cfg, FOCAL), true);
        for band in stage.bands() {
            let px = paint_band(&stage, band, false);
            let bright = px.data.iter().filter(|&&p| (p >> 24) > 200).count();
            assert_eq!(bright, 0, "the stage ring is glass only: no strip, no rings");
        }
    }
}
