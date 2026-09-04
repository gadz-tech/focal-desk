//! A look at the painter without a Windows desktop: renders every frame
//! of the default layout at the mock's scale (a 1920×1080 view of the
//! 65" panel, so the inch-based sizes come out at the mock's pixel
//! sizes) over a dark ground with plain window rectangles, and writes
//! `target/frames-preview.ppm` (binary P6). The left-top window is drawn
//! as the active one.
//!
//! `cargo run -p focal-core --example frames_preview`, then any image
//! viewer; the wrap of 2026-09-04 converts it to PNG with a few lines of
//! Python's standard library.

use std::io::Write;

use focal_core::config::{Config, Screen};
use focal_core::geometry::Rect;
use focal_core::layout::{self, SlotId, FOCAL, SLOT_COUNT};
use focal_core::paint::{self, Pixels};
use focal_core::tab;

/// Fill `rect` of an RGB image (`w` wide) with a solid color.
fn fill(img: &mut [[u8; 3]], w: usize, h: usize, rect: Rect, color: [u8; 3]) {
    let (x0, y0) = (rect.x.round().max(0.0) as usize, rect.y.round().max(0.0) as usize);
    let (x1, y1) = (
        (rect.right().round() as usize).min(w),
        (rect.bottom().round() as usize).min(h),
    );
    for y in y0..y1 {
        for x in x0..x1 {
            img[y * w + x] = color;
        }
    }
}

/// Composite premultiplied pixels over the image at `at`.
fn blend(img: &mut [[u8; 3]], w: usize, h: usize, at: Rect, px: &Pixels) {
    let (ox, oy) = (at.x.round() as i64, at.y.round() as i64);
    for y in 0..px.h {
        for x in 0..px.w {
            let (ix, iy) = (ox + x as i64, oy + y as i64);
            if ix < 0 || iy < 0 || ix as usize >= w || iy as usize >= h {
                continue;
            }
            let p = px.data[y * px.w + x];
            let a = (p >> 24) as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let src = [((p >> 16) & 0xFF) as f32, ((p >> 8) & 0xFF) as f32, (p & 0xFF) as f32];
            let dst = &mut img[iy as usize * w + ix as usize];
            for c in 0..3 {
                dst[c] = (src[c] + dst[c] as f32 * (1.0 - a)).round().min(255.0) as u8;
            }
        }
    }
}

/// Render the scene and write the PPM.
fn main() {
    let mut cfg = Config::default();
    cfg.screen = Screen::from_px(1920, 1080, 65.0);
    let (w, h) = (1920usize, 1080usize);
    let mut img = vec![[0x14u8, 0x14, 0x0F]; w * h];
    for i in 0..SLOT_COUNT as u8 {
        fill(&mut img, w, h, layout::window_rect(&cfg, SlotId(i)), [0x23, 0x22, 0x1E]);
    }
    for i in 0..SLOT_COUNT as u8 {
        let s = SlotId(i);
        let frame = tab::frame(&cfg, layout::window_rect(&cfg, s), s == FOCAL);
        for band in frame.bands() {
            let px = paint::paint_band(&frame, band, s == SlotId(1));
            blend(&mut img, w, h, band, &px);
        }
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/frames-preview.ppm");
    let mut out = std::fs::File::create(&path).expect("create the preview file");
    write!(out, "P6\n{w} {h}\n255\n").expect("write the header");
    let bytes: Vec<u8> = img.iter().flat_map(|p| p.iter().copied()).collect();
    out.write_all(&bytes).expect("write the pixels");
    println!("wrote {}", path.display());
}
