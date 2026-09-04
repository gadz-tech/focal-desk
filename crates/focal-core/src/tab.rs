//! Tap-target geometry: where the tab (or border) beside a managed
//! window goes. Pure, like everything in this crate — the adapter draws
//! whatever this returns and reads the mouse on it.
//!
//! Ryan, 2026-08-28: "a border around each window which can be a manual
//! focus activator — all the way around, or maybe a tab that is always
//! facing the center of the screen. It has to be pretty big so I can tap
//! it easily." Tap → `Event::Promoted`; drag → `Event::MoveTo`. Which
//! shape is `tap_style` in the config; how big is `tab_in` and
//! `tab_length_in`, in inches like the gutter.
//!
//! One rule makes the geometry safe: **a target lives in its own
//! window's half of the gutter.** Every window is its region inset by
//! half a gutter, so that margin belongs to the window alone; a target no
//! deeper than the margin (`Config::tab_px` clamps it) never touches
//! another window, another target, or the far side of the channel.
//! `targets_never_collide` pins it for all thirteen slots.
//!
//! 2026-09-04 (REQUESTS §6): **the stage gets a slim ring, no tab.** The
//! window on the focal stage already has focus; its frame is only a drag
//! handle and, later, the landing strip for wires, so it is `stage_in`
//! wide all round and never grows a title tab. The adapter now draws
//! every target *behind* its window (§4) — geometry here is unchanged by
//! that; only who is on top is.

use crate::config::{Config, TapStyle};
use crate::geometry::Rect;

/// Which edge of a window a tab hangs off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// What the adapter should put beside a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TapTarget {
    /// One tab, this rectangle, flush against the center-facing edge.
    Tab(Rect),
    /// A ring: everything inside `outer` but outside `inner` (the
    /// window itself, which the ring must not cover).
    Border { outer: Rect, inner: Rect },
}

impl TapTarget {
    /// The same target shifted by `(dx, dy)` — the adapter adds the
    /// managed monitor's virtual-desktop origin this way.
    pub fn offset(self, dx: f32, dy: f32) -> Self {
        let shift = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        match self {
            TapTarget::Tab(r) => TapTarget::Tab(shift(r)),
            TapTarget::Border { outer, inner } => TapTarget::Border {
                outer: shift(outer),
                inner: shift(inner),
            },
        }
    }

    /// The rectangle a window drawing this target must cover.
    pub fn bounds(self) -> Rect {
        match self {
            TapTarget::Tab(r) => r,
            TapTarget::Border { outer, .. } => outer,
        }
    }
}

/// The edge of `rect` that faces the screen center, by the dominant
/// axis of the offset between the two centers. A window that already
/// sits at the center — the focal stage — gets its tab on top, where
/// the eye expects a title.
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

/// The tab for a window at `rect`: `tab_px` deep, up to `tab_length_px`
/// long (never longer than the edge), centered on the facing edge and
/// flush against it, so it sits in the window's own half-gutter.
pub fn tab_rect(cfg: &Config, rect: Rect) -> Rect {
    let depth = cfg.tab_px();
    match facing_edge(cfg, rect) {
        Edge::Right => {
            let len = cfg.tab_length_px().min(rect.h);
            Rect::new(rect.right(), rect.y + (rect.h - len) / 2.0, depth, len)
        }
        Edge::Left => {
            let len = cfg.tab_length_px().min(rect.h);
            Rect::new(rect.x - depth, rect.y + (rect.h - len) / 2.0, depth, len)
        }
        Edge::Bottom => {
            let len = cfg.tab_length_px().min(rect.w);
            Rect::new(rect.x + (rect.w - len) / 2.0, rect.bottom(), len, depth)
        }
        Edge::Top => {
            let len = cfg.tab_length_px().min(rect.w);
            Rect::new(rect.x + (rect.w - len) / 2.0, rect.y - depth, len, depth)
        }
    }
}

/// The ring around a window at `rect`, `tab_px` wide on every side.
pub fn border_ring(cfg: &Config, rect: Rect) -> TapTarget {
    TapTarget::Border {
        outer: rect.expand(cfg.tab_px()),
        inner: rect,
    }
}

/// The stage's frame: a slim ring all the way around, `stage_px` wide,
/// with no thick side and no title tab (REQUESTS-2026-09-04 §6). Retires
/// the 2026-09-03 "the stage gets a title tab on top" rule.
pub fn stage_frame(cfg: &Config, rect: Rect) -> TapTarget {
    TapTarget::Border { outer: rect.expand(cfg.stage_px()), inner: rect }
}

/// The tap target for a window at `rect`: the stage's slim ring while it
/// is `staged` (on the focal stage), else the configured style.
pub fn tap_target(cfg: &Config, rect: Rect, staged: bool) -> TapTarget {
    if staged {
        return stage_frame(cfg, rect);
    }
    match cfg.tap_style {
        TapStyle::Tab => TapTarget::Tab(tab_rect(cfg, rect)),
        TapStyle::Border => border_ring(cfg, rect),
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

    /// The facing edge of a slot's window at rest.
    fn edge_of(cfg: &Config, name: &str) -> Edge {
        facing_edge(cfg, layout::window_rect(cfg, slot(name)))
    }

    #[test]
    fn tabs_face_the_center() {
        let cfg = Config::default();
        assert_eq!(edge_of(&cfg, "left-top"), Edge::Right);
        assert_eq!(edge_of(&cfg, "left-bottom"), Edge::Right);
        assert_eq!(edge_of(&cfg, "right-top"), Edge::Left);
        assert_eq!(edge_of(&cfg, "right-bottom"), Edge::Left);
        assert_eq!(edge_of(&cfg, "top-1"), Edge::Bottom);
        assert_eq!(edge_of(&cfg, "top-2"), Edge::Bottom);
        assert_eq!(edge_of(&cfg, "bottom-1"), Edge::Top);
        assert_eq!(edge_of(&cfg, "bottom-2"), Edge::Top);
        // Corners are further from the center sideways than up or down
        // on a 16:9 panel, so they face inward along the band.
        assert_eq!(edge_of(&cfg, "corner-tl"), Edge::Right);
        assert_eq!(edge_of(&cfg, "corner-br"), Edge::Left);
        // The stage is the center; its tab is a title.
        assert_eq!(edge_of(&cfg, "focal"), Edge::Top);
    }

    #[test]
    fn a_tab_sits_flush_in_its_windows_half_gutter() {
        let cfg = Config::default();
        for i in 0..SLOT_COUNT as u8 {
            let s = SlotId(i);
            let win = layout::window_rect(&cfg, s);
            let tab = tab_rect(&cfg, win);
            let name = layout::slot_name(s);
            assert!(within(tab, region_px(&cfg, s)), "{name}: tab leaves its region");
            assert!(!overlaps(tab, win), "{name}: tab covers its window");
            // Flush: the tab shares an edge with the window.
            let touching = (tab.x - win.right()).abs() < 0.01
                || (tab.right() - win.x).abs() < 0.01
                || (tab.y - win.bottom()).abs() < 0.01
                || (tab.bottom() - win.y).abs() < 0.01;
            assert!(touching, "{name}: tab floats away from its window");
        }
    }

    #[test]
    fn targets_never_collide() {
        for style in [TapStyle::Tab, TapStyle::Border] {
            let mut cfg = Config::default();
            cfg.tap_style = style;
            let bounds: Vec<Rect> = (0..SLOT_COUNT as u8)
                .map(|i| tap_target(&cfg, layout::window_rect(&cfg, SlotId(i)), false).bounds())
                .collect();
            let wins: Vec<Rect> = (0..SLOT_COUNT as u8)
                .map(|i| layout::window_rect(&cfg, SlotId(i)))
                .collect();
            for a in 0..SLOT_COUNT {
                for b in 0..SLOT_COUNT {
                    if a != b {
                        assert!(
                            !overlaps(bounds[a], bounds[b]),
                            "{style:?}: targets of {} and {} overlap",
                            layout::slot_name(SlotId(a as u8)),
                            layout::slot_name(SlotId(b as u8))
                        );
                    }
                    if !(style == TapStyle::Border && a == b) {
                        assert!(
                            !overlaps(bounds[a], wins[b]),
                            "{style:?}: target of {} covers window {}",
                            layout::slot_name(SlotId(a as u8)),
                            layout::slot_name(SlotId(b as u8))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_tab_is_as_big_as_asked_and_no_bigger_than_the_edge() {
        let cfg = Config::default();
        let ppi = cfg.screen.px_per_inch();
        let tab = tab_rect(&cfg, layout::window_rect(&cfg, slot("left-top")));
        // 0.75" deep, 5" long at ~135.6 ppi.
        assert!((tab.w - 0.75 * ppi).abs() < 0.5, "depth {}", tab.w);
        assert!((tab.h - 5.0 * ppi).abs() < 0.5, "length {}", tab.h);
        // Ask for a tab longer than any edge: it is clamped to the edge.
        let mut long = Config::default();
        long.tab_length_in = 100.0;
        let win = layout::window_rect(&long, slot("left-top"));
        let tab = tab_rect(&long, win);
        assert!((tab.h - win.h).abs() < 0.01);
        assert!((tab.y - win.y).abs() < 0.01);
    }

    #[test]
    fn the_border_is_a_ring_in_the_half_gutter() {
        let mut cfg = Config::default();
        cfg.tap_style = TapStyle::Border;
        let s = slot("right-top");
        let win = layout::window_rect(&cfg, s);
        match tap_target(&cfg, win, false) {
            TapTarget::Border { outer, inner } => {
                assert_eq!(inner, win);
                assert!((outer.x - (win.x - cfg.tab_px())).abs() < 0.01);
                assert!((outer.w - (win.w + 2.0 * cfg.tab_px())).abs() < 0.01);
                assert!(within(outer, region_px(&cfg, s)));
            }
            other => panic!("expected a border, got {other:?}"),
        }
    }

    #[test]
    fn the_focused_windows_tab_follows_its_fit() {
        let cfg = Config::default();
        let stage = layout::window_rect(&cfg, FOCAL);
        let narrow = layout::focal_rect(&cfg, Some(Fit { w: 0.55, h: 1.0 }));
        let tab = tab_rect(&cfg, narrow);
        // Still a title tab, centered on the narrower window, inside the
        // stage's region.
        assert_eq!(facing_edge(&cfg, narrow), Edge::Top);
        assert!((tab.center().0 - narrow.center().0).abs() < 0.01);
        assert!((tab.bottom() - narrow.y).abs() < 0.01);
        assert!(within(tab, region_px(&cfg, FOCAL)));
        assert!(!overlaps(tab, stage.expand(-1.0)) || tab.bottom() <= stage.y + 0.01);
    }

    #[test]
    fn the_stage_gets_a_slim_ring_and_no_tab() {
        for style in [TapStyle::Tab, TapStyle::Border] {
            let mut cfg = Config::default();
            cfg.tap_style = style;
            let stage = layout::window_rect(&cfg, FOCAL);
            let narrow = layout::focal_rect(&cfg, Some(Fit { w: 0.55, h: 1.0 }));
            for win in [stage, narrow] {
                match tap_target(&cfg, win, true) {
                    TapTarget::Border { outer, inner } => {
                        assert_eq!(inner, win);
                        assert!((outer.x - (win.x - cfg.stage_px())).abs() < 0.01);
                        assert!((outer.y - (win.y - cfg.stage_px())).abs() < 0.01);
                        assert!((outer.right() - (win.right() + cfg.stage_px())).abs() < 0.01);
                        assert!(
                            within(outer, region_px(&cfg, FOCAL)),
                            "{style:?}: the ring leaves the stage's region"
                        );
                    }
                    other => panic!("{style:?}: the stage got {other:?}, not a slim ring"),
                }
            }
            // Slim means slimmer than a tab, and the ring never meets a
            // neighbour's target or window.
            assert!(cfg.stage_px() < cfg.tab_px());
            let ring = tap_target(&cfg, stage, true).bounds();
            for i in 1..SLOT_COUNT as u8 {
                let s = SlotId(i);
                let other = tap_target(&cfg, layout::window_rect(&cfg, s), false).bounds();
                assert!(!overlaps(ring, other), "{style:?}: the stage ring meets {}", layout::slot_name(s));
                assert!(!overlaps(ring, layout::window_rect(&cfg, s)), "{style:?}: ring over {}", layout::slot_name(s));
            }
        }
    }

    #[test]
    fn offset_moves_the_whole_target() {
        let cfg = Config::default();
        let win = layout::window_rect(&cfg, slot("top-1"));
        let t = tap_target(&cfg, win, false).offset(100.0, -50.0);
        let plain = tap_target(&cfg, win, false).bounds();
        assert_eq!(t.bounds(), Rect::new(plain.x + 100.0, plain.y - 50.0, plain.w, plain.h));
        let ring = border_ring(&cfg, win).offset(10.0, 10.0);
        match ring {
            TapTarget::Border { inner, .. } => assert_eq!(inner.x, win.x + 10.0),
            _ => unreachable!(),
        }
    }
}
