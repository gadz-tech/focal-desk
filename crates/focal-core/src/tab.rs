//! Frame geometry (2026-09-04, REQUESTS §3): where the frame behind a
//! managed window goes and what sits on it. Pure, like everything in
//! this crate — `paint.rs` rasterizes it, the adapter blits the pixels
//! and reads the mouse on them.
//!
//! Ryan, 2026-08-28: "a border around each window which can be a manual
//! focus activator … It has to be pretty big so I can tap it easily."
//! 2026-09-04: "encapsulating the whole window. frosted glass rounded
//! edges … Slim on all sides but the side facing the center will still
//! be thick." Tap → `Event::Promoted`; drag → `Event::MoveTo`.
//!
//! One frame per window: a rounded rectangle behind it, `frame_in` slim
//! on three sides and `tab_in` thick on the side facing screen center.
//! The thick side carries the neon strip (the wire's port,
//! `tab_length_in` long — centred in the band on a vertical side, offset
//! toward the window on a horizontal one) and a connection hole at each
//! end; the one-line notification box (§7, later) has its place computed
//! and nothing drawn. The stage's frame is `stage_in` slim all round —
//! no thick side, no strip, no tab: it already has focus (§6). The old
//! `tap_style` (tab | border) is retired: the frame is both, made one.
//!
//! One rule keeps the geometry safe: **a frame lives in its own window's
//! half of the gutter.** Every window is its region inset by half a
//! gutter, so that margin belongs to the window alone; a frame no deeper
//! than the margin (`Config::tab_px`, `frame_px` and `stage_px` all
//! clamp to it) never touches another window or another frame.
//! `frames_never_collide` pins it for all thirteen slots. Outer and
//! inner corner radii are concentric (outer = inner + the side's width),
//! so the ring keeps its width around the corner — "no intersecting
//! radii" (rev 5).

use crate::config::Config;
use crate::geometry::Rect;

/// Which side of a window faces the screen center (the thick side).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// The window's own corner radius, roughly (Windows 11 rounds top-level
/// windows; this is where the frame's inner edge meets it), in inches.
const RADIUS_IN_IN: f32 = 0.09;
/// Thickness of the neon strip (3 px at the mock's 1080p scale), inches.
const STRIP_IN: f32 = 0.09;
/// Radius of a connection hole, inches.
const HOLE_R_IN: f32 = 0.2;
/// Distance of a hole's centre from the window's corner along the thick
/// side, inches ("at the ends").
const HOLE_INSET_IN: f32 = 0.9;
/// Height of the one-line notification box (§7), inches.
const NOTE_H_IN: f32 = 0.22;

/// One window's frame, in screen-local pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// The window itself: the frame's hole. Nothing is painted inside it.
    pub inner: Rect,
    /// The frame's outside edge.
    pub outer: Rect,
    /// The thick side, facing screen center; `None` for the stage.
    pub thick: Option<Edge>,
    /// Corner radius of the outer edge, in pixels.
    pub radius_out: f32,
    /// Corner radius where the frame meets the window.
    pub radius_in: f32,
    /// The neon strip on the thick side: long, thin, the wire's port.
    pub strip: Option<Rect>,
    /// Connection holes at the two ends of the thick side (centres).
    /// Neon when a wire connects this window to another, a silver ring
    /// while free — all free until the wires are drawn.
    pub holes: Vec<(f32, f32)>,
    /// Radius of a connection hole.
    pub hole_r: f32,
    /// Where the one-line notification box will sit (§7 — computed, not
    /// drawn): inside the thick band under the strip for a horizontal
    /// thick side; for a vertical one, inside the corner window's foot
    /// band (under a top corner's window, above a bottom corner's), and
    /// under the frame for a side window, whose glass has no foot yet.
    pub note: Option<Rect>,
}

impl Frame {
    /// The same frame shifted by `(dx, dy)` — the adapter adds the
    /// managed monitor's virtual-desktop origin this way.
    pub fn offset(&self, dx: f32, dy: f32) -> Self {
        let shift = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        Self {
            inner: shift(self.inner),
            outer: shift(self.outer),
            thick: self.thick,
            radius_out: self.radius_out,
            radius_in: self.radius_in,
            strip: self.strip.map(shift),
            holes: self.holes.iter().map(|&(x, y)| (x + dx, y + dy)).collect(),
            hole_r: self.hole_r,
            note: self.note.map(shift),
        }
    }

    /// The rectangle the whole frame covers.
    pub fn bounds(&self) -> Rect {
        self.outer
    }

    /// The four bands that tile the ring — top and bottom span the full
    /// outer width (they own the corners), left and right sit between
    /// them — so the adapter can host each in a small window instead of
    /// one window the size of the whole frame with a transparent middle.
    pub fn bands(&self) -> [Rect; 4] {
        let (o, i) = (self.outer, self.inner);
        [
            Rect::new(o.x, o.y, o.w, i.y - o.y),
            Rect::new(o.x, i.bottom(), o.w, o.bottom() - i.bottom()),
            Rect::new(o.x, i.y, i.x - o.x, i.h),
            Rect::new(i.right(), i.y, o.right() - i.right(), i.h),
        ]
    }

    /// The narrowest side of the ring, in pixels.
    pub fn slim_px(&self) -> f32 {
        let (o, i) = (self.outer, self.inner);
        (i.y - o.y)
            .min(o.bottom() - i.bottom())
            .min(i.x - o.x)
            .min(o.right() - i.right())
    }

    /// Over how many pixels the inner edge fades into the window: most
    /// of a slim side, so the fade reads on every side.
    pub fn fade_px(&self) -> f32 {
        (self.slim_px() * 0.6).max(1.0)
    }
}

/// The edge of `rect` that faces the screen center, by the dominant
/// axis of the offset between the two centers. A window that already
/// sits at the center is the stage and does not use this; should it be
/// asked anyway, it gets `Top`.
pub fn facing_edge(cfg: &Config, rect: Rect) -> Edge {
    let (sx, sy) = (cfg.screen.px_w as f32 / 2.0, cfg.screen.px_h as f32 / 2.0);
    let (cx, cy) = rect.center();
    let (dx, dy) = (sx - cx, sy - cy);
    if dx.abs() <= 0.5 && dy.abs() <= 0.5 {
        return Edge::Top;
    }
    if dx.abs() > dy.abs() {
        if dx > 0.0 {
            Edge::Right
        } else {
            Edge::Left
        }
    } else if dy > 0.0 {
        Edge::Bottom
    } else {
        Edge::Top
    }
}

/// The frame for a window at `rect`: the stage's slim ring while it is
/// `staged` (on the focal stage), else slim on three sides and thick on
/// the side facing screen center, with the strip, the holes and the
/// notification box's place laid out on the thick side.
pub fn frame(cfg: &Config, rect: Rect, staged: bool) -> Frame {
    let ppi = cfg.screen.px_per_inch();
    let radius_in = RADIUS_IN_IN * ppi;
    let hole_r_full = HOLE_R_IN * ppi;
    if staged {
        let slim = cfg.stage_px();
        return Frame {
            inner: rect,
            outer: rect.expand(slim),
            thick: None,
            radius_out: radius_in + slim,
            radius_in,
            strip: None,
            holes: Vec::new(),
            hole_r: hole_r_full.min(slim / 2.5),
            note: None,
        };
    }
    let slim = cfg.frame_px();
    let thick = cfg.tab_px().max(slim);
    let edge = facing_edge(cfg, rect);
    let (l, t, r, b) = match edge {
        Edge::Left => (thick, slim, slim, slim),
        Edge::Right => (slim, slim, thick, slim),
        Edge::Top => (slim, thick, slim, slim),
        Edge::Bottom => (slim, slim, slim, thick),
    };
    // A corner window's glass extends on the band side to line up with
    // the neighbouring top/bottom window's thick frame (Ryan, 2026-09-04,
    // after the live run): down for a top corner, up for a bottom one.
    // That foot band is the §7 notification box's home.
    let vertical = matches!(edge, Edge::Left | Edge::Right);
    let (sh, band_h) = (cfg.screen.px_h as f32, cfg.band_frac * cfg.screen.px_h as f32);
    let top_corner = vertical && rect.bottom() <= band_h + 0.5;
    let bottom_corner = vertical && rect.y >= sh - band_h - 0.5;
    let (t, b) = (if bottom_corner { thick } else { t }, if top_corner { thick } else { b });
    let outer = Rect::new(rect.x - l, rect.y - t, rect.w + l + r, rect.h + t + b);
    let radius_out = radius_in + slim;
    // The thick band as a rectangle, flush against the window.
    let band = match edge {
        Edge::Left => Rect::new(outer.x, rect.y, thick, rect.h),
        Edge::Right => Rect::new(rect.right(), rect.y, thick, rect.h),
        Edge::Top => Rect::new(rect.x, outer.y, rect.w, thick),
        Edge::Bottom => Rect::new(rect.x, rect.bottom(), rect.w, thick),
    };
    let strip_t = (STRIP_IN * ppi).min(thick / 3.0);
    let hole_r = hole_r_full.min(thick / 2.5);
    let inset = HOLE_INSET_IN * ppi;
    let side_len = if vertical { rect.h } else { rect.w };
    // The strip never reaches the holes: leave a hole's width of air.
    let strip_max = (side_len - 2.0 * (inset + 2.0 * hole_r)).max(hole_r);
    let strip_len = cfg.tab_length_px().min(side_len * 0.6).min(strip_max);
    let note_h = (NOTE_H_IN * ppi).min(thick * 0.4);
    let (strip, holes, note) = if vertical {
        // Centred left-to-right in the band (rev 3, e); holes at the ends.
        let cx = band.x + band.w / 2.0;
        let strip = Rect::new(
            cx - strip_t / 2.0,
            rect.y + (rect.h - strip_len) / 2.0,
            strip_t,
            strip_len,
        );
        let holes = vec![(cx, rect.y + inset), (cx, rect.bottom() - inset)];
        // §7's box, inset past the corner arcs (rev 5): inside the foot
        // band for a corner window — under the window for a top corner,
        // above it for a bottom one — and under the frame for a side
        // window, whose glass has no foot yet.
        let note_y = if bottom_corner { rect.y - thick + slim } else { rect.bottom() + slim };
        let note = Rect::new(
            rect.x + radius_out,
            note_y,
            (rect.w - 2.0 * radius_out).max(0.0),
            note_h,
        );
        (strip, holes, note)
    } else {
        // Offset toward the window (rev 3, e): a fifth of the band in
        // from the window's edge; holes at the ends, centred in the band.
        let cy = band.y + band.h / 2.0;
        let off = thick * 0.2;
        let sy = match edge {
            Edge::Top => band.bottom() - off - strip_t,
            _ => band.y + off,
        };
        let sx = rect.x + (rect.w - strip_len) / 2.0;
        let strip = Rect::new(sx, sy, strip_len, strip_t);
        let holes = vec![(rect.x + inset, cy), (rect.right() - inset, cy)];
        // §7's box: under the strip, inside the band with margin (rev 4).
        let ny = match edge {
            Edge::Top => sy - off - note_h,
            _ => sy + strip_t + off,
        };
        let note = Rect::new(sx, ny, strip_len, note_h);
        (strip, holes, note)
    };
    Frame {
        inner: rect,
        outer,
        thick: Some(edge),
        radius_out,
        radius_in,
        strip: Some(strip),
        holes,
        hole_r,
        note: Some(note),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Fit;
    use crate::layout::{self, SlotId, FOCAL, SLOT_COUNT};

    /// True when two rectangles share any area. Touching edges do not,
    /// and neither does the sub-pixel slack that float inset-then-expand
    /// leaves between two rings meeting on a region boundary.
    fn overlaps(a: Rect, b: Rect) -> bool {
        let e = 0.01;
        a.x < b.right() - e && b.x < a.right() - e && a.y < b.bottom() - e && b.y < a.bottom() - e
    }

    /// True when `inner` lies within `outer`; edges may coincide.
    fn within(inner: Rect, outer: Rect) -> bool {
        let e = 0.01;
        inner.x >= outer.x - e
            && inner.y >= outer.y - e
            && inner.right() <= outer.right() + e
            && inner.bottom() <= outer.bottom() + e
    }

    /// A slot's region in pixels — the area that is the window's alone.
    fn region_px(cfg: &Config, s: SlotId) -> Rect {
        let r = layout::regions(cfg)[s.0 as usize].frac;
        let (w, h) = (cfg.screen.px_w as f32, cfg.screen.px_h as f32);
        Rect::new(r.x * w, r.y * h, r.w * w, r.h * h)
    }

    /// The slot named in a config file, unwrapped for the tests.
    fn slot(name: &str) -> SlotId {
        layout::slot_from_name(name).unwrap()
    }

    /// The frame of a slot's window at rest (not staged).
    fn frame_of(cfg: &Config, name: &str) -> Frame {
        frame(cfg, layout::window_rect(cfg, slot(name)), false)
    }

    /// The square around a hole's centre.
    fn hole_rect(f: &Frame, i: usize) -> Rect {
        let (x, y) = f.holes[i];
        Rect::new(x - f.hole_r, y - f.hole_r, 2.0 * f.hole_r, 2.0 * f.hole_r)
    }

    /// The thick band of a frame as a rectangle (the outer ring minus the
    /// three slim sides, on the thick side).
    fn thick_band(f: &Frame) -> Rect {
        let (o, i) = (f.outer, f.inner);
        match f.thick.unwrap() {
            Edge::Left => Rect::new(o.x, i.y, i.x - o.x, i.h),
            Edge::Right => Rect::new(i.right(), i.y, o.right() - i.right(), i.h),
            Edge::Top => Rect::new(i.x, o.y, i.w, i.y - o.y),
            Edge::Bottom => Rect::new(i.x, i.bottom(), i.w, o.bottom() - i.bottom()),
        }
    }

    #[test]
    fn frames_face_the_center() {
        let cfg = Config::default();
        let edge = |n: &str| frame_of(&cfg, n).thick.unwrap();
        assert_eq!(edge("left-top"), Edge::Right);
        assert_eq!(edge("left-bottom"), Edge::Right);
        assert_eq!(edge("right-top"), Edge::Left);
        assert_eq!(edge("right-bottom"), Edge::Left);
        assert_eq!(edge("top-1"), Edge::Bottom);
        assert_eq!(edge("top-2"), Edge::Bottom);
        assert_eq!(edge("bottom-1"), Edge::Top);
        assert_eq!(edge("bottom-2"), Edge::Top);
        // Corners are further from the center sideways than up or down
        // on a 16:9 panel, so they face inward along the band.
        assert_eq!(edge("corner-tl"), Edge::Right);
        assert_eq!(edge("corner-br"), Edge::Left);
    }

    #[test]
    fn a_frame_sits_in_its_windows_half_gutter() {
        let cfg = Config::default();
        for i in 1..SLOT_COUNT as u8 {
            let s = SlotId(i);
            let win = layout::window_rect(&cfg, s);
            let f = frame(&cfg, win, false);
            let name = layout::slot_name(s);
            assert_eq!(f.inner, win, "{name}: the hole is the window");
            assert!(within(f.outer, region_px(&cfg, s)), "{name}: frame leaves its region");
            assert!(within(win, f.outer), "{name}: frame does not surround its window");
        }
    }

    #[test]
    fn frames_never_collide() {
        let cfg = Config::default();
        let frames: Vec<Frame> = (0..SLOT_COUNT as u8)
            .map(|i| frame(&cfg, layout::window_rect(&cfg, SlotId(i)), i == 0))
            .collect();
        for a in 0..SLOT_COUNT {
            for b in 0..SLOT_COUNT {
                if a == b {
                    continue;
                }
                assert!(
                    !overlaps(frames[a].outer, frames[b].outer),
                    "frames of {} and {} overlap",
                    layout::slot_name(SlotId(a as u8)),
                    layout::slot_name(SlotId(b as u8))
                );
                assert!(
                    !overlaps(frames[a].outer, frames[b].inner),
                    "frame of {} covers window {}",
                    layout::slot_name(SlotId(a as u8)),
                    layout::slot_name(SlotId(b as u8))
                );
            }
        }
    }

    #[test]
    fn frame_sizes_are_as_asked() {
        let cfg = Config::default();
        let ppi = cfg.screen.px_per_inch();
        let f = frame_of(&cfg, "left-top");
        let (o, i) = (f.outer, f.inner);
        // Slim: 0.15" on the three sides away from the center; thick:
        // 0.75" toward it.
        assert!((i.x - o.x - 0.15 * ppi).abs() < 0.5, "left {}", i.x - o.x);
        assert!((i.y - o.y - 0.15 * ppi).abs() < 0.5);
        assert!((o.bottom() - i.bottom() - 0.15 * ppi).abs() < 0.5);
        assert!((o.right() - i.right() - 0.75 * ppi).abs() < 0.5, "thick {}", o.right() - i.right());
        assert!((f.slim_px() - 0.15 * ppi).abs() < 0.5);
        // Concentric corners: the ring keeps its width round the corner.
        assert!((f.radius_out - (f.radius_in + f.slim_px())).abs() < 0.01);
        // The knobs are honoured.
        let mut wide = Config::default();
        wide.frame_in = 0.3;
        wide.tab_in = 0.5;
        let f = frame_of(&wide, "left-top");
        assert!((f.slim_px() - 0.3 * ppi).abs() < 0.5);
        assert!((f.outer.right() - f.inner.right() - 0.5 * ppi).abs() < 0.5);
    }

    #[test]
    fn the_stage_frame_is_slim_all_round_with_no_furniture() {
        let cfg = Config::default();
        let stage = layout::window_rect(&cfg, FOCAL);
        let narrow = layout::focal_rect(&cfg, Some(Fit { w: 0.55, h: 1.0 }));
        for win in [stage, narrow] {
            let f = frame(&cfg, win, true);
            assert_eq!(f.thick, None);
            assert_eq!(f.strip, None);
            assert!(f.holes.is_empty());
            assert_eq!(f.note, None);
            assert_eq!(f.inner, win);
            assert!((f.outer.x - (win.x - cfg.stage_px())).abs() < 0.01);
            assert!((f.outer.bottom() - (win.bottom() + cfg.stage_px())).abs() < 0.01);
            assert!(within(f.outer, region_px(&cfg, FOCAL)), "the ring leaves the stage's region");
            assert!((f.slim_px() - cfg.stage_px()).abs() < 0.01);
        }
        // Slim means slimmer than a thick side, and the ring never meets
        // a neighbour's frame or window.
        assert!(cfg.stage_px() < cfg.tab_px());
        let ring = frame(&cfg, stage, true).outer;
        for i in 1..SLOT_COUNT as u8 {
            let s = SlotId(i);
            let other = frame(&cfg, layout::window_rect(&cfg, s), false);
            assert!(!overlaps(ring, other.outer), "the stage ring meets {}", layout::slot_name(s));
            assert!(!overlaps(ring, other.inner), "the stage ring covers {}", layout::slot_name(s));
        }
    }

    #[test]
    fn the_strip_and_holes_sit_in_the_thick_band_off_the_window() {
        let cfg = Config::default();
        let ppi = cfg.screen.px_per_inch();
        for i in 1..SLOT_COUNT as u8 {
            let s = SlotId(i);
            let name = layout::slot_name(s);
            let f = frame(&cfg, layout::window_rect(&cfg, s), false);
            let band = thick_band(&f);
            let strip = f.strip.expect("every side window has a strip");
            assert!(within(strip, band), "{name}: strip leaves the thick band");
            assert!(!overlaps(strip, f.inner), "{name}: strip over the window");
            assert_eq!(f.holes.len(), 2, "{name}: a hole at each end");
            for h in 0..2 {
                assert!(within(hole_rect(&f, h), band), "{name}: hole {h} leaves the band");
                assert!(!overlaps(hole_rect(&f, h), strip), "{name}: hole {h} meets the strip");
            }
            let vertical = matches!(f.thick, Some(Edge::Left | Edge::Right));
            if vertical {
                // Centred in the band, long and thin, 5" unless the side is short.
                assert!((strip.center().0 - band.center().0).abs() < 0.01, "{name}: strip off-centre");
                assert!(strip.w < strip.h / 10.0, "{name}: strip is not thin");
                // Long: about a third of the side or more, never longer than
                // asked, never more than 60% (the holes keep their air).
                assert!(strip.h <= cfg.tab_length_px() + 0.5, "{name}: strip longer than asked");
                assert!(strip.h <= band.h * 0.6 + 0.5, "{name}: strip crowds the holes");
                assert!(strip.h >= band.h * 0.3, "{name}: strip too short ({} of {})", strip.h, band.h);
                // Holes at the ends: the same distance from each end.
                assert!((f.holes[0].1 - f.inner.y - 0.9 * ppi).abs() < 0.5);
                assert!((f.inner.bottom() - f.holes[1].1 - 0.9 * ppi).abs() < 0.5);
            } else {
                // Offset toward the window: nearer the window's edge than
                // the band's far edge.
                let to_window = match f.thick.unwrap() {
                    Edge::Top => f.inner.y - strip.bottom(),
                    _ => strip.y - f.inner.bottom(),
                };
                let to_far = match f.thick.unwrap() {
                    Edge::Top => strip.y - f.outer.y,
                    _ => f.outer.bottom() - strip.bottom(),
                };
                assert!(to_window < to_far, "{name}: strip not offset toward the window");
                assert!(strip.h < strip.w / 10.0, "{name}: strip is not thin");
                assert!(strip.w <= cfg.tab_length_px() + 0.5, "{name}: strip longer than asked");
                assert!(strip.w >= band.w * 0.3, "{name}: strip too short ({} of {})", strip.w, band.w);
                assert!((f.holes[0].0 - f.inner.x - 0.9 * ppi).abs() < 0.5);
                assert!((f.inner.right() - f.holes[1].0 - 0.9 * ppi).abs() < 0.5);
            }
            assert!((f.hole_r - 0.2 * ppi).abs() < 0.5, "{name}: hole radius");
        }
    }

    #[test]
    fn bands_tile_the_ring() {
        let cfg = Config::default();
        for i in 0..SLOT_COUNT as u8 {
            let s = SlotId(i);
            let name = layout::slot_name(s);
            let f = frame(&cfg, layout::window_rect(&cfg, s), i == 0);
            let bands = f.bands();
            let area: f32 = bands.iter().map(|b| b.w * b.h).sum();
            let ring = f.outer.w * f.outer.h - f.inner.w * f.inner.h;
            assert!((area - ring).abs() < 1.0, "{name}: bands do not tile the ring ({area} vs {ring})");
            for (a, ba) in bands.iter().enumerate() {
                assert!(within(*ba, f.outer), "{name}: band {a} leaves the frame");
                assert!(!overlaps(*ba, f.inner), "{name}: band {a} covers the window");
                for (b, bb) in bands.iter().enumerate() {
                    assert!(a == b || !overlaps(*ba, *bb), "{name}: bands {a} and {b} overlap");
                }
            }
        }
    }

    #[test]
    fn the_note_has_a_place_and_is_clear_of_the_corners() {
        let cfg = Config::default();
        for i in 1..SLOT_COUNT as u8 {
            let s = SlotId(i);
            let name = layout::slot_name(s);
            let f = frame(&cfg, layout::window_rect(&cfg, s), false);
            let note = f.note.expect("every side window has a place for its box");
            let strip = f.strip.unwrap();
            assert!(!overlaps(note, f.inner), "{name}: box over the window");
            assert!(!overlaps(note, strip), "{name}: box over the strip");
            assert!(within(note, region_px(&cfg, s)), "{name}: box leaves the region");
            match f.thick.unwrap() {
                Edge::Left | Edge::Right => {
                    // Under the window (above it for a bottom corner), ending
                    // before the corner arcs begin; inside the glass where
                    // the frame has a foot band.
                    let bottom_corner = name.starts_with("corner-b");
                    if bottom_corner {
                        assert!(note.bottom() <= f.inner.y + 0.01, "{name}: box not above the window");
                        assert!(note.y >= f.outer.y - 0.01, "{name}: box leaves the foot band");
                    } else {
                        assert!(note.y >= f.inner.bottom() - 0.01, "{name}: box not under the window");
                    }
                    if name.starts_with("corner-") {
                        assert!(within(note, f.outer), "{name}: box outside the glass");
                    }
                    assert!(note.x >= f.inner.x + f.radius_out - 0.01, "{name}: box meets the left arc");
                    assert!(note.right() <= f.inner.right() - f.radius_out + 0.01, "{name}: box meets the right arc");
                }
                Edge::Top | Edge::Bottom => {
                    // Inside the thick band, under the strip, with margin.
                    let band = thick_band(&f);
                    assert!(within(note, band), "{name}: box leaves the band");
                    assert!(note.y > band.y + 0.01 && note.bottom() < band.bottom() - 0.01, "{name}: box touches the band's edge");
                }
            }
        }
    }

    #[test]
    fn corner_glass_lines_up_with_the_neighbours_frame() {
        let cfg = Config::default();
        let f = |n: &str| frame_of(&cfg, n);
        // Top corners: the foot reaches down to where the top-band
        // window's thick frame ends — the region boundary.
        for (corner, neighbour) in [("corner-tl", "top-1"), ("corner-tr", "top-2")] {
            let (c, n) = (f(corner), f(neighbour));
            assert!((c.outer.bottom() - n.outer.bottom()).abs() < 0.5, "{corner}: glass bottom {} vs {} {}", c.outer.bottom(), neighbour, n.outer.bottom());
            assert!((c.outer.bottom() - c.inner.bottom() - cfg.tab_px()).abs() < 0.5, "{corner}: the foot is a thick side deep");
            assert!((c.inner.y - c.outer.y - cfg.frame_px()).abs() < 0.5, "{corner}: the top stays slim");
            assert!(within(c.outer, region_px(&cfg, slot(corner))), "{corner}: foot leaves the region");
        }
        // Bottom corners: the same, upward, level with the bottom-band
        // window's thick top frame.
        for (corner, neighbour) in [("corner-bl", "bottom-1"), ("corner-br", "bottom-2")] {
            let (c, n) = (f(corner), f(neighbour));
            assert!((c.outer.y - n.outer.y).abs() < 0.5, "{corner}: glass top {} vs {} {}", c.outer.y, neighbour, n.outer.y);
            assert!((c.inner.y - c.outer.y - cfg.tab_px()).abs() < 0.5, "{corner}: the foot is a thick side deep");
            assert!((c.outer.bottom() - c.inner.bottom() - cfg.frame_px()).abs() < 0.5, "{corner}: the bottom stays slim");
            assert!(within(c.outer, region_px(&cfg, slot(corner))), "{corner}: foot leaves the region");
        }
        // Side windows have no foot: slim above and below.
        for side in ["left-top", "left-bottom", "right-top", "right-bottom"] {
            let s = f(side);
            assert!((s.outer.bottom() - s.inner.bottom() - cfg.frame_px()).abs() < 0.5, "{side}");
            assert!((s.inner.y - s.outer.y - cfg.frame_px()).abs() < 0.5, "{side}");
        }
        // And the foot never reaches the side window below/above it.
        assert!(!overlaps(f("corner-tl").outer, f("left-top").outer));
        assert!(!overlaps(f("corner-bl").outer, f("left-bottom").outer));
    }

    #[test]
    fn offset_moves_the_whole_frame() {
        let cfg = Config::default();
        let f = frame_of(&cfg, "top-1");
        let g = f.offset(100.0, -50.0);
        assert_eq!(g.outer, Rect::new(f.outer.x + 100.0, f.outer.y - 50.0, f.outer.w, f.outer.h));
        assert_eq!(g.inner.x, f.inner.x + 100.0);
        assert_eq!(g.strip.unwrap().y, f.strip.unwrap().y - 50.0);
        assert_eq!(g.holes[0], (f.holes[0].0 + 100.0, f.holes[0].1 - 50.0));
        assert_eq!(g.note.unwrap().x, f.note.unwrap().x + 100.0);
        assert_eq!(g.bounds(), g.outer);
    }
}
