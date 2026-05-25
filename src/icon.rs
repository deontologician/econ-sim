//! Procedural thematic icons for the elements. Each icon is a short recipe of
//! signed-distance primitives composited back-to-front and rasterized to a small
//! RGBA texture, so the *same* art serves as a map sprite and a HUD image with no
//! asset files and no extra crates. Coverage is antialiased against the pixel size.
//!
//! Coordinate space is `[-1, 1]` with y up; shapes stay inside ~`±0.85` for margin.

use std::f32::consts::{FRAC_PI_4, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::elements::ELEMENTS;

/// Texture resolution. Small (icons are drawn ~18px in the HUD / sub-hex on the
/// map); generation is a one-off at setup so cost is irrelevant.
pub const ICON_PX: u32 = 64;

type Rgba = [f32; 4];
type Shape = (Prim, Rgba, Op);

#[derive(Clone, Copy)]
enum Op {
    Over,
    Cut,
}

#[derive(Clone, Copy)]
enum Prim {
    Circle { c: Vec2, r: f32 },
    Ring { c: Vec2, r: f32, th: f32 },
    Seg { a: Vec2, b: Vec2, th: f32 },
    Rect { c: Vec2, half: Vec2, rot: f32 },
    Tri { a: Vec2, b: Vec2, c: Vec2 },
    Hex { c: Vec2, r: f32, rot: f32 },
}

fn sd_circle(p: Vec2, c: Vec2, r: f32) -> f32 {
    (p - c).length() - r
}

fn sd_segment(p: Vec2, a: Vec2, b: Vec2, th: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = (pa.dot(ba) / ba.dot(ba)).clamp(0.0, 1.0);
    (pa - ba * h).length() - th
}

fn sd_box(p: Vec2, half: Vec2) -> f32 {
    let d = p.abs() - half;
    d.max(Vec2::ZERO).length() + d.x.max(d.y).min(0.0)
}

// iquilezles' exact triangle SDF (negative inside).
fn sd_triangle(p: Vec2, p0: Vec2, p1: Vec2, p2: Vec2) -> f32 {
    let (e0, e1, e2) = (p1 - p0, p2 - p1, p0 - p2);
    let (v0, v1, v2) = (p - p0, p - p1, p - p2);
    let pq0 = v0 - e0 * (v0.dot(e0) / e0.dot(e0)).clamp(0.0, 1.0);
    let pq1 = v1 - e1 * (v1.dot(e1) / e1.dot(e1)).clamp(0.0, 1.0);
    let pq2 = v2 - e2 * (v2.dot(e2) / e2.dot(e2)).clamp(0.0, 1.0);
    let s = (e0.x * e2.y - e0.y * e2.x).signum();
    let dx = pq0
        .dot(pq0)
        .min(pq1.dot(pq1))
        .min(pq2.dot(pq2));
    let dy = (s * (v0.x * e0.y - v0.y * e0.x))
        .min(s * (v1.x * e1.y - v1.y * e1.x))
        .min(s * (v2.x * e2.y - v2.y * e2.x));
    -dx.sqrt() * dy.signum()
}

// iquilezles' flat-top hexagon SDF (negative inside).
fn sd_hexagon(p0: Vec2, r: f32) -> f32 {
    let kxy = Vec2::new(-0.866_025_4, 0.5);
    let kz = 0.577_350_3;
    let mut p = p0.abs();
    p -= kxy * (2.0 * kxy.dot(p).min(0.0));
    p -= Vec2::new(p.x.clamp(-kz * r, kz * r), r);
    p.length() * p.y.signum()
}

fn sd(prim: &Prim, p: Vec2) -> f32 {
    match *prim {
        Prim::Circle { c, r } => sd_circle(p, c, r),
        Prim::Ring { c, r, th } => sd_circle(p, c, r).abs() - th,
        Prim::Seg { a, b, th } => sd_segment(p, a, b, th),
        Prim::Rect { c, half, rot } => sd_box(Vec2::from_angle(-rot).rotate(p - c), half),
        Prim::Tri { a, b, c } => sd_triangle(p, a, b, c),
        Prim::Hex { c, r, rot } => sd_hexagon(Vec2::from_angle(-rot).rotate(p - c), r),
    }
}

/// Rasterize an element's recipe into an RGBA8 (sRGB) image.
pub fn render_icon(id: usize) -> Image {
    let shapes = recipe(id);
    let s = ICON_PX as usize;
    let inv = 1.0 / s as f32;
    let px = 2.0 * inv; // one pixel, in the normalized coordinate space
    let mut data = vec![0u8; s * s * 4];
    for y in 0..s {
        for x in 0..s {
            let p = Vec2::new(
                (x as f32 + 0.5) * inv * 2.0 - 1.0,
                1.0 - (y as f32 + 0.5) * inv * 2.0,
            );
            let mut acc = [0.0f32; 4];
            for (prim, color, op) in &shapes {
                let cov = (0.5 - sd(prim, p) / px).clamp(0.0, 1.0) * color[3];
                match op {
                    Op::Over => {
                        for k in 0..3 {
                            acc[k] = color[k] * cov + acc[k] * (1.0 - cov);
                        }
                        acc[3] = cov + acc[3] * (1.0 - cov);
                    }
                    Op::Cut => acc[3] *= 1.0 - cov,
                }
            }
            let o = (y * s + x) * 4;
            for k in 0..4 {
                data[o + k] = (acc[k].clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
    }
    Image::new(
        Extent3d {
            width: ICON_PX,
            height: ICON_PX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

const DARK: Rgba = [0.12, 0.12, 0.14, 1.0];

/// The per-element shape recipe. `base` is the element's own colour; accents are
/// literal. Indices match `ELEMENTS`.
fn recipe(id: usize) -> Vec<Shape> {
    let c = ELEMENTS[id].color;
    let base: Rgba = [c.0, c.1, c.2, 1.0];
    use Op::{Cut, Over};
    use Prim::*;
    match id {
        // Lightning — a jagged bolt.
        0 => vec![
            (Seg { a: v(0.2, 0.9), b: v(-0.25, 0.1), th: 0.14 }, base, Over),
            (Seg { a: v(-0.25, 0.1), b: v(0.15, 0.05), th: 0.14 }, base, Over),
            (Seg { a: v(0.15, 0.05), b: v(-0.2, -0.9), th: 0.14 }, base, Over),
        ],
        // Fire — a flame: red teardrop with a brighter inner flame.
        1 => vec![
            (Circle { c: v(0.0, -0.3), r: 0.5 }, base, Over),
            (Tri { a: v(-0.45, -0.15), b: v(0.45, -0.15), c: v(0.0, 0.85) }, base, Over),
            (Circle { c: v(0.0, -0.3), r: 0.28 }, [0.99, 0.8, 0.3, 1.0], Over),
            (Tri { a: v(-0.24, -0.15), b: v(0.24, -0.15), c: v(0.0, 0.45) }, [0.99, 0.8, 0.3, 1.0], Over),
        ],
        // Slime — a googly blob.
        2 => vec![
            (Circle { c: v(0.0, -0.15), r: 0.62 }, base, Over),
            (Circle { c: v(-0.4, 0.2), r: 0.18 }, base, Over),
            (Circle { c: v(0.4, 0.18), r: 0.16 }, base, Over),
            (Circle { c: v(-0.2, 0.0), r: 0.09 }, DARK, Over),
            (Circle { c: v(0.2, 0.0), r: 0.09 }, DARK, Over),
        ],
        // Acid — droplet with bubbles.
        3 => vec![
            (Circle { c: v(0.0, -0.2), r: 0.46 }, base, Over),
            (Tri { a: v(-0.33, -0.05), b: v(0.33, -0.05), c: v(0.0, 0.72) }, base, Over),
            (Circle { c: v(-0.12, -0.28), r: 0.08 }, [1.0, 1.0, 1.0, 0.7], Over),
            (Circle { c: v(0.15, -0.1), r: 0.06 }, [1.0, 1.0, 1.0, 0.7], Over),
        ],
        // Water — droplet with a highlight.
        4 => vec![
            (Circle { c: v(0.0, -0.2), r: 0.46 }, base, Over),
            (Tri { a: v(-0.33, -0.05), b: v(0.33, -0.05), c: v(0.0, 0.75) }, base, Over),
            (Circle { c: v(-0.13, -0.27), r: 0.1 }, [1.0, 1.0, 1.0, 0.8], Over),
        ],
        // Sugar — a cube with a lighter highlight.
        5 => vec![
            (Rect { c: v(0.0, 0.0), half: v(0.5, 0.5), rot: 0.0 }, base, Over),
            (Rect { c: v(-0.12, 0.12), half: v(0.18, 0.18), rot: 0.0 }, [1.0, 1.0, 1.0, 0.6], Over),
        ],
        // Wood — a log with end-grain rings.
        6 => vec![
            (Rect { c: v(0.0, 0.0), half: v(0.7, 0.3), rot: 0.0 }, base, Over),
            (Ring { c: v(-0.5, 0.0), r: 0.16, th: 0.045 }, [0.4, 0.27, 0.14, 1.0], Over),
            (Ring { c: v(-0.5, 0.0), r: 0.07, th: 0.04 }, [0.4, 0.27, 0.14, 1.0], Over),
        ],
        // Ice — a diamond.
        7 => vec![
            (Rect { c: v(0.0, 0.0), half: v(0.45, 0.45), rot: FRAC_PI_4 }, base, Over),
            (Rect { c: v(0.0, 0.0), half: v(0.2, 0.2), rot: FRAC_PI_4 }, [1.0, 1.0, 1.0, 0.7], Over),
        ],
        // Stone — a lumpy rock with a flat base and a shaded facet.
        8 => vec![
            (Circle { c: v(-0.05, -0.1), r: 0.55 }, base, Over),
            (Rect { c: v(0.1, -0.45), half: v(0.55, 0.2), rot: 0.0 }, base, Over),
            (Tri { a: v(-0.5, 0.1), b: v(0.0, 0.35), c: v(0.1, -0.1) }, [0.4, 0.4, 0.43, 0.6], Over),
        ],
        // Sand — two dunes over a ground line.
        9 => vec![
            (Rect { c: v(0.0, -0.7), half: v(0.9, 0.25), rot: 0.0 }, base, Over),
            (Circle { c: v(-0.3, -0.3), r: 0.42 }, base, Over),
            (Circle { c: v(0.32, -0.36), r: 0.46 }, base, Over),
        ],
        // Oil — a dark droplet with a purple sheen.
        10 => vec![
            (Circle { c: v(0.0, -0.2), r: 0.46 }, base, Over),
            (Tri { a: v(-0.33, -0.05), b: v(0.33, -0.05), c: v(0.0, 0.75) }, base, Over),
            (Circle { c: v(-0.13, -0.25), r: 0.12 }, [0.55, 0.45, 0.65, 0.8], Over),
        ],
        // Gold — an ingot (trapezoid) with a bright top face.
        11 => vec![
            (Tri { a: v(-0.6, -0.28), b: v(0.6, -0.28), c: v(0.45, 0.15) }, base, Over),
            (Tri { a: v(-0.6, -0.28), b: v(0.45, 0.15), c: v(-0.45, 0.15) }, base, Over),
            (Rect { c: v(0.0, 0.16), half: v(0.45, 0.07), rot: 0.0 }, [1.0, 0.9, 0.5, 1.0], Over),
        ],
        // Iron — a hexagonal nugget with a highlight.
        12 => vec![
            (Hex { c: v(0.0, 0.0), r: 0.62, rot: 0.0 }, base, Over),
            (Hex { c: v(0.0, 0.0), r: 0.32, rot: 0.0 }, [0.78, 0.8, 0.84, 0.6], Over),
        ],
        // Salt — scattered crystals.
        13 => vec![
            (Rect { c: v(-0.25, -0.1), half: v(0.2, 0.2), rot: FRAC_PI_4 }, base, Over),
            (Rect { c: v(0.28, 0.05), half: v(0.16, 0.16), rot: FRAC_PI_4 }, base, Over),
            (Rect { c: v(0.05, -0.45), half: v(0.14, 0.14), rot: FRAC_PI_4 }, base, Over),
            (Rect { c: v(-0.05, 0.4), half: v(0.12, 0.12), rot: FRAC_PI_4 }, base, Over),
        ],
        // Steam — a puffy cloud.
        14 => vec![
            (Rect { c: v(0.0, -0.28), half: v(0.7, 0.18), rot: 0.0 }, base, Over),
            (Circle { c: v(-0.35, -0.05), r: 0.32 }, base, Over),
            (Circle { c: v(0.05, 0.12), r: 0.4 }, base, Over),
            (Circle { c: v(0.42, -0.05), r: 0.3 }, base, Over),
        ],
        // Ash — scattered specks.
        15 => vec![
            (Circle { c: v(-0.3, 0.2), r: 0.16 }, base, Over),
            (Circle { c: v(0.25, 0.3), r: 0.12 }, base, Over),
            (Circle { c: v(0.1, -0.1), r: 0.2 }, base, Over),
            (Circle { c: v(-0.15, -0.35), r: 0.13 }, base, Over),
            (Circle { c: v(0.4, -0.25), r: 0.1 }, base, Over),
        ],
        // Crystal — a cut gem with facet lines.
        16 => vec![
            (Tri { a: v(-0.45, 0.3), b: v(0.45, 0.3), c: v(0.3, 0.0) }, base, Over),
            (Tri { a: v(-0.45, 0.3), b: v(0.3, 0.0), c: v(-0.3, 0.0) }, base, Over),
            (Tri { a: v(-0.3, 0.0), b: v(0.3, 0.0), c: v(0.0, -0.7) }, base, Over),
            (Seg { a: v(-0.45, 0.3), b: v(0.0, -0.7), th: 0.02 }, [1.0, 1.0, 1.0, 0.6], Over),
            (Seg { a: v(0.45, 0.3), b: v(0.0, -0.7), th: 0.02 }, [1.0, 1.0, 1.0, 0.6], Over),
        ],
        // Mud — a blob with darker bubbles.
        17 => vec![
            (Circle { c: v(0.0, -0.1), r: 0.55 }, base, Over),
            (Circle { c: v(-0.35, 0.2), r: 0.2 }, base, Over),
            (Circle { c: v(0.35, 0.18), r: 0.18 }, base, Over),
            (Circle { c: v(-0.1, -0.05), r: 0.1 }, [0.3, 0.22, 0.15, 1.0], Over),
            (Circle { c: v(0.2, -0.2), r: 0.08 }, [0.3, 0.22, 0.15, 1.0], Over),
        ],
        // Smoke — rising puffs.
        18 => vec![
            (Circle { c: v(-0.2, -0.25), r: 0.3 }, base, Over),
            (Circle { c: v(0.1, 0.05), r: 0.34 }, base, Over),
            (Circle { c: v(0.35, 0.32), r: 0.22 }, base, Over),
        ],
        // Wind — gusts with a curl.
        19 => vec![
            (Seg { a: v(-0.6, 0.3), b: v(0.4, 0.3), th: 0.1 }, base, Over),
            (Ring { c: v(0.4, 0.48), r: 0.18, th: 0.1 }, base, Over),
            (Seg { a: v(-0.6, -0.05), b: v(0.25, -0.05), th: 0.1 }, base, Over),
            (Seg { a: v(-0.6, -0.4), b: v(0.05, -0.4), th: 0.1 }, base, Over),
        ],
        // Light — a sun with rays.
        20 => {
            let mut s = vec![(Circle { c: v(0.0, 0.0), r: 0.34 }, base, Over)];
            for i in 0..8 {
                let dir = Vec2::from_angle(i as f32 * TAU / 8.0);
                s.push((Seg { a: dir * 0.42, b: dir * 0.75, th: 0.07 }, base, Over));
            }
            s
        }
        // Shadow — a crescent (a disc with a bite cut out).
        21 => vec![
            (Circle { c: v(0.0, 0.0), r: 0.58 }, base, Over),
            (Circle { c: v(0.3, 0.18), r: 0.52 }, base, Cut),
        ],
        // Copper — a wire coil with a tail.
        22 => vec![
            (Ring { c: v(0.0, 0.1), r: 0.4, th: 0.1 }, base, Over),
            (Seg { a: v(0.0, -0.3), b: v(0.0, -0.7), th: 0.08 }, base, Over),
            (Seg { a: v(0.0, -0.7), b: v(0.3, -0.7), th: 0.08 }, base, Over),
        ],
        // Sulfur — a hazard triangle with a mark.
        23 => vec![
            (Tri { a: v(-0.62, -0.5), b: v(0.62, -0.5), c: v(0.0, 0.62) }, base, Over),
            (Rect { c: v(0.0, -0.02), half: v(0.06, 0.22), rot: 0.0 }, DARK, Over),
            (Circle { c: v(0.0, -0.36), r: 0.08 }, DARK, Over),
        ],
        // Honey — a comb cell with a drip.
        24 => vec![
            (Hex { c: v(0.0, 0.2), r: 0.5, rot: 0.0 }, base, Over),
            (Hex { c: v(0.0, 0.2), r: 0.28, rot: 0.0 }, base, Cut),
            (Circle { c: v(0.0, -0.5), r: 0.16 }, base, Over),
            (Tri { a: v(-0.12, -0.4), b: v(0.12, -0.4), c: v(0.0, -0.05) }, base, Over),
        ],
        // Fallback: a plain disc in the element's colour.
        _ => vec![(Circle { c: v(0.0, 0.0), r: 0.6 }, base, Over)],
    }
}
