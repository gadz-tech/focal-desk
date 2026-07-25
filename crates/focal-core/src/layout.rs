//! The slot layout: 13 regions that tile the screen exactly, windows
//! inset from their region by half the structural gutter.
//!
//! ```text
//! +----+--------+--------+----+
//! | c  |  t1    |  t2    | c  |   band (band_frac high)
//! +----+--------+--------+----+
//! | L1 |                 | R1 |
//! +----+      FOCAL      +----+   middle
//! | L2 |                 | R2 |
//! +----+--------+--------+----+
//! | c  |  b1    |  b2    | c  |   band
//! +----+--------+--------+----+
//! ```
//! Region edges align, so the gutters form straight, continuous
//! channels — which is what makes wire routing tractable.

use crate::config::{Config, Fit};
use crate::geometry::Rect;

/// One of the 13 fixed slots. Slot 0 is the focal stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotId(pub u8);

pub const FOCAL: SlotId = SlotId(0);
pub const SLOT_COUNT: usize = 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Focal,
    Side,
    Small,
}

#[derive(Clone, Copy, Debug)]
pub struct Region {
    /// Position and size as fractions of the screen.
    pub frac: Rect,
    pub tier: Tier,
}

/// The 13 regions. Pure function of config, so a future alternative
/// layout (scrolling strip, twin focal, ultrawide variant) is just a
/// different implementation behind the same slot ids.
pub fn regions(cfg: &Config) -> [Region; SLOT_COUNT] {
    let fc = cfg.focal_frac;
    let sc = (1.0 - fc) / 2.0;
    let tb = cfg.band_frac;
    let mid = 1.0 - 2.0 * tb;
    let r = |x: f32, y: f32, w: f32, h: f32, tier: Tier| Region {
        frac: Rect::new(x, y, w, h),
        tier,
    };
    [
        r(sc, tb, fc, mid, Tier::Focal),
        r(0.0, tb, sc, mid / 2.0, Tier::Side),
        r(0.0, tb + mid / 2.0, sc, mid / 2.0, Tier::Side),
        r(sc + fc, tb, sc, mid / 2.0, Tier::Side),
        r(sc + fc, tb + mid / 2.0, sc, mid / 2.0, Tier::Side),
        r(sc, 0.0, fc / 2.0, tb, Tier::Small),
        r(sc + fc / 2.0, 0.0, fc / 2.0, tb, Tier::Small),
        r(sc, tb + mid, fc / 2.0, tb, Tier::Small),
        r(sc + fc / 2.0, tb + mid, fc / 2.0, tb, Tier::Small),
        r(0.0, 0.0, sc, tb, Tier::Small),
        r(sc + fc, 0.0, sc, tb, Tier::Small),
        r(0.0, tb + mid, sc, tb, Tier::Small),
        r(sc + fc, tb + mid, sc, tb, Tier::Small),
    ]
}

pub fn slot_name(s: SlotId) -> &'static str {
    [
        "focal",
        "left-top",
        "left-bottom",
        "right-top",
        "right-bottom",
        "top-1",
        "top-2",
        "bottom-1",
        "bottom-2",
        "corner-tl",
        "corner-tr",
        "corner-bl",
        "corner-br",
    ][s.0 as usize]
}

/// Home assignment order for windows without an explicit slot:
/// center-out — sides first, then bands, corners last.
pub fn home_priority() -> [SlotId; 12] {
    [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12].map(SlotId)
}

/// Window rect for a slot: region in pixels, inset by half the gutter.
/// Because neighbouring regions share an edge, two adjacent windows end
/// up exactly one gutter apart — the structural-gutter invariant.
pub fn window_rect(cfg: &Config, slot: SlotId) -> Rect {
    let reg = regions(cfg)[slot.0 as usize];
    let (w, h) = (cfg.screen.px_w as f32, cfg.screen.px_h as f32);
    Rect::new(
        reg.frac.x * w,
        reg.frac.y * h,
        reg.frac.w * w,
        reg.frac.h * h,
    )
    .inset(cfg.gutter_px() / 2.0)
}

/// Where a promoted window sits: the focal stage shrunk by the app's
/// fit hint and centered. The stage is a place, not a size.
pub fn focal_rect(cfg: &Config, fit: Option<Fit>) -> Rect {
    let stage = window_rect(cfg, FOCAL);
    let f = fit.unwrap_or_default();
    stage.centered_sub(stage.w * f.w, stage.h * f.h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_tile_exactly() {
        let cfg = Config::default();
        let sum: f32 = regions(&cfg).iter().map(|r| r.frac.w * r.frac.h).sum();
        assert!((sum - 1.0).abs() < 1e-5, "regions must tile the screen, got {sum}");
    }

    #[test]
    fn gutter_is_structural() {
        let cfg = Config::default();
        let g = cfg.gutter_px();
        let focal = window_rect(&cfg, FOCAL);
        let l1 = window_rect(&cfg, SlotId(1));
        let t1 = window_rect(&cfg, SlotId(5));
        // horizontal neighbour: left column to focal
        assert!(((focal.x - l1.right()) - g).abs() < 0.5);
        // vertical neighbour: top band to focal
        assert!(((focal.y - t1.bottom()) - g).abs() < 0.5);
        // screen edge: half a gutter
        assert!((l1.x - g / 2.0).abs() < 0.5);
    }

    #[test]
    fn focal_fit_is_centered() {
        let cfg = Config::default();
        let stage = window_rect(&cfg, FOCAL);
        let r = focal_rect(&cfg, Some(Fit { w: 0.55, h: 1.0 }));
        assert!((r.w - stage.w * 0.55).abs() < 0.5);
        assert!((r.h - stage.h).abs() < 0.5);
        let (sc, rc) = (stage.center(), r.center());
        assert!((sc.0 - rc.0).abs() < 0.5 && (sc.1 - rc.1).abs() < 0.5);
    }
}
