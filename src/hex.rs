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

/// The six neighbour offset coordinates of `(col, row)` on a **torus**: the map wraps
/// in both axes, so every tile has six in-bounds neighbours and there are no edges.
/// `cols` and `rows` should be even so odd-r parity stays consistent across the seam.
pub fn neighbors(col: i32, row: i32, cols: i32, rows: i32) -> [(i32, i32); 6] {
    let deltas = if row.rem_euclid(2) == 0 { EVEN_ROW } else { ODD_ROW };
    let mut out = [(0, 0); 6];
    for (i, (dc, dr)) in deltas.iter().enumerate() {
        out[i] = ((col + dc).rem_euclid(cols), (row + dr).rem_euclid(rows));
    }
    out
}

/// odd-r offset coordinate → cube coordinate.
fn to_cube(col: i32, row: i32) -> (i32, i32, i32) {
    let x = col - (row - (row & 1)) / 2;
    let z = row;
    (x, -x - z, z)
}

/// Hex distance (number of steps) between two odd-r offset coords, ignoring wrap.
fn cube_distance(ac: i32, ar: i32, bc: i32, br: i32) -> i32 {
    let (ax, ay, az) = to_cube(ac, ar);
    let (bx, by, bz) = to_cube(bc, br);
    ((ax - bx).abs() + (ay - by).abs() + (az - bz).abs()) / 2
}

/// Shortest hex distance on the **torus**: the minimum over the nine wrapped images of
/// `b`. `cols`/`rows` must be even (so the wrapped images keep odd-r parity), which the
/// map already guarantees.
pub fn torus_distance(ac: i32, ar: i32, bc: i32, br: i32, cols: i32, rows: i32) -> i32 {
    let mut best = i32::MAX;
    for i in -1..=1 {
        for j in -1..=1 {
            best = best.min(cube_distance(ac, ar, bc + i * cols, br + j * rows));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torus_distance_is_symmetric_and_zero_on_self() {
        assert_eq!(torus_distance(3, 4, 3, 4, 30, 22), 0);
        assert_eq!(
            torus_distance(2, 1, 9, 7, 30, 22),
            torus_distance(9, 7, 2, 1, 30, 22)
        );
    }

    #[test]
    fn torus_distance_wraps_around_the_seam() {
        // Columns 0 and 29 are neighbours across the wrap on a 30-wide torus, so the
        // distance is far less than the 29 it would be without wrapping.
        assert!(torus_distance(0, 0, 29, 0, 30, 22) < 5);
    }

    #[test]
    fn each_neighbour_is_one_step_away() {
        for (nc, nr) in neighbors(5, 5, 30, 22) {
            assert_eq!(torus_distance(5, 5, nc, nr, 30, 22), 1);
        }
    }
}
