//! Pointy-top hex grid math using "odd-r" offset coordinates (odd rows pushed
//! right by half a hex). Storage stays a simple `row * cols + col` grid; this
//! module only handles pixel placement and neighbour lookup.

pub const SQRT3: f32 = 1.732_050_8;

/// Pixel centre of the hex at offset coordinate `(col, row)`.
///
/// y grows downward in offset space, so we negate it to put row 0 at the top
/// in Bevy's y-up world space.
pub fn hex_center(col: i32, row: i32, size: f32) -> (f32, f32) {
    let shift = 0.5 * (row.rem_euclid(2)) as f32;
    let x = size * SQRT3 * (col as f32 + shift);
    let y = -size * 1.5 * row as f32;
    (x, y)
}

// odd-r neighbour deltas (dcol, drow), differing by row parity.
const EVEN_ROW: [(i32, i32); 6] = [(1, 0), (0, -1), (-1, -1), (-1, 0), (-1, 1), (0, 1)];
const ODD_ROW: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (0, 1), (1, 1)];

/// The six neighbour offset coordinates of `(col, row)`. Callers must bounds-check.
pub fn neighbors(col: i32, row: i32) -> [(i32, i32); 6] {
    let deltas = if row.rem_euclid(2) == 0 { EVEN_ROW } else { ODD_ROW };
    let mut out = [(0, 0); 6];
    for (i, (dc, dr)) in deltas.iter().enumerate() {
        out[i] = (col + dc, row + dr);
    }
    out
}
