//! CPU-rasterized line charts for the in-app Graphs overlay. Like `icon.rs`, each chart
//! is drawn straight into an RGBA texture (no plotting crate, no asset files) and shown
//! through a UI `ImageNode`; the textures are re-rasterized in place a few times a
//! second from a rolling history buffer.
//!
//! Two chart kinds: a **sparkline** (one series, min–max auto-scaled to fill the box)
//! per stat, and an **overlay** chart that draws several selected series together, each
//! normalized independently to [0,1] so their *shapes* line up for spotting correlations
//! regardless of magnitude.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// The time series tracked, in sample order. Index into a `[f32; N_SERIES]` sample and
/// into `SERIES` (label + plot colour). "Plus average age" lives at index 13.
pub const N_SERIES: usize = 15;

/// `(label, rgb)` for each series. Colours are picked to stay distinct when several are
/// overlaid on the correlation chart.
pub const SERIES: [(&str, [u8; 3]); N_SERIES] = [
    ("prod/t", [90, 200, 120]),
    ("cons/t", [230, 140, 70]),
    ("margin/t", [240, 210, 80]),
    ("utility/t", [120, 200, 230]),
    ("trades", [180, 140, 230]),
    ("avg bucks", [120, 230, 180]),
    ("appetite", [230, 110, 110]),
    ("starving", [230, 70, 90]),
    ("claimed", [150, 190, 90]),
    ("hunger", [220, 120, 60]),
    ("deaths/t", [200, 60, 70]),
    ("income/t", [110, 180, 120]),
    ("infl%", [200, 200, 120]),
    ("avg age", [130, 170, 240]),
    ("clump", [200, 150, 220]),
];

const BG: [u8; 4] = [18, 18, 24, 255];
const OVERLAY_BG: [u8; 4] = [12, 12, 16, 255];
const GRID: [u8; 4] = [44, 44, 54, 255];

/// A blank RGBA8 texture, kept in MAIN+RENDER world so its pixels can be mutated and
/// re-uploaded each refresh (the icons, by contrast, are static and render-world only).
pub fn blank_image(w: u32, h: u32) -> Image {
    Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; (w * h * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

fn dims(img: &Image) -> (usize, usize) {
    let s = img.texture_descriptor.size;
    (s.width as usize, s.height as usize)
}

fn fill(d: &mut [u8], c: [u8; 4]) {
    for px in d.chunks_exact_mut(4) {
        px.copy_from_slice(&c);
    }
}

fn put(d: &mut [u8], w: usize, h: usize, x: i32, y: i32, c: [u8; 4]) {
    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
        let o = ((y as usize) * w + x as usize) * 4;
        d[o..o + 4].copy_from_slice(&c);
    }
}

/// Bresenham line. A raster primitive naturally takes the buffer, its dimensions, two
/// endpoints and a colour — splitting that up would only obscure it.
#[allow(clippy::too_many_arguments)]
fn line(d: &mut [u8], w: usize, h: usize, mut x0: i32, mut y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        put(d, w, h, x0, y0, c);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// Map sample `i` of `n` (auto-scaled by `lo..hi`) to pixel coordinates in `w×h`, with a
/// 1px top/bottom margin so peaks aren't clipped.
fn point(i: usize, v: f32, n: usize, lo: f32, hi: f32, w: usize, h: usize) -> (i32, i32) {
    let span = (hi - lo).max(1e-9);
    let x = if n > 1 {
        (i as f32 / (n - 1) as f32 * (w as f32 - 1.0)) as i32
    } else {
        0
    };
    let norm = ((v - lo) / span).clamp(0.0, 1.0);
    let y = ((1.0 - norm) * (h as f32 - 3.0)) as i32 + 1;
    (x, y)
}

fn min_max(samples: &[f32]) -> (f32, f32) {
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    for &v in samples {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if lo > hi {
        (0.0, 1.0)
    } else if (hi - lo).abs() < 1e-9 {
        (lo - 0.5, hi + 0.5) // flat series → centre the line
    } else {
        (lo, hi)
    }
}

/// Rasterize one auto-scaled series filling the box, with a faint mid-line baseline.
pub fn render_sparkline(img: &mut Image, samples: &[f32], color: [u8; 3]) {
    let (w, h) = dims(img);
    let Some(d) = img.data.as_mut() else {
        return;
    };
    fill(d, BG);
    let mid = (h / 2) as i32;
    for x in 0..w as i32 {
        put(d, w, h, x, mid, GRID);
    }
    if samples.len() < 2 {
        return;
    }
    let (lo, hi) = min_max(samples);
    let c = [color[0], color[1], color[2], 255];
    for i in 1..samples.len() {
        let (x0, y0) = point(i - 1, samples[i - 1], samples.len(), lo, hi, w, h);
        let (x1, y1) = point(i, samples[i], samples.len(), lo, hi, w, h);
        line(d, w, h, x0, y0, x1, y1, c);
    }
}

/// Rasterize several series together, each independently normalized to fill the height,
/// so their shapes can be compared for correlation. Draws quarter grid-lines behind.
pub fn render_overlay(img: &mut Image, series: &[(&[f32], [u8; 3])]) {
    let (w, h) = dims(img);
    let Some(d) = img.data.as_mut() else {
        return;
    };
    fill(d, OVERLAY_BG);
    for q in 1..4 {
        let y = (q * h / 4) as i32;
        for x in 0..w as i32 {
            put(d, w, h, x, y, GRID);
        }
    }
    for (samples, color) in series {
        if samples.len() < 2 {
            continue;
        }
        let (lo, hi) = min_max(samples);
        let c = [color[0], color[1], color[2], 255];
        for i in 1..samples.len() {
            let (x0, y0) = point(i - 1, samples[i - 1], samples.len(), lo, hi, w, h);
            let (x1, y1) = point(i, samples[i], samples.len(), lo, hi, w, h);
            line(d, w, h, x0, y0, x1, y1, c);
            // A second offset row thickens the trace so overlaid lines read clearly.
            line(d, w, h, x0, y0 + 1, x1, y1 + 1, c);
        }
    }
}
