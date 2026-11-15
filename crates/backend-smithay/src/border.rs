//! Rounded border corners. The straight parts of a border are plain
//! rectangles; each corner of a rounded border is a small painted tile: a
//! quarter ring with smooth edges.

/// The outer radius of a rounded border, in pixels (the corner rounding of
/// Windows 11 windows).
pub const OUTER_RADIUS: i32 = 8;

/// Which corner of the window a tile is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

pub const CORNERS: [Corner; 4] = [
    Corner::TopLeft,
    Corner::TopRight,
    Corner::BottomLeft,
    Corner::BottomRight,
];

/// The side of a corner tile for a border of this width: the outer radius,
/// and never less than the border is wide.
pub fn tile_size(width: i32) -> i32 {
    OUTER_RADIUS.max(width)
}

/// Paint one corner: a `size` x `size` tile (`size` = `tile_size(width)`),
/// four bytes per pixel in the order blue, green, red, alpha, with the
/// colour already multiplied by the alpha (what the renderer expects).
/// The ring is `width` thick; its edges are smoothed over one pixel.
pub fn corner_tile(width: i32, color: (u8, u8, u8), alpha: u8, corner: Corner) -> Vec<u8> {
    let size = tile_size(width);
    let outer = size as f32;
    let inner = (size - width) as f32;
    // The centre of the circle is the tile's corner that points into the
    // window.
    let (cx, cy) = match corner {
        Corner::TopLeft => (outer, outer),
        Corner::TopRight => (0.0, outer),
        Corner::BottomLeft => (outer, 0.0),
        Corner::BottomRight => (0.0, 0.0),
    };
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let distance = (dx * dx + dy * dy).sqrt();
            // 1 inside the ring, 0 outside, a ramp across each edge.
            let coverage =
                (outer - distance + 0.5).clamp(0.0, 1.0) * (distance - inner + 0.5).clamp(0.0, 1.0);
            let a = coverage * f32::from(alpha) / 255.0;
            let at = ((y * size + x) * 4) as usize;
            pixels[at] = (f32::from(color.2) * a).round() as u8;
            pixels[at + 1] = (f32::from(color.1) * a).round() as u8;
            pixels[at + 2] = (f32::from(color.0) * a).round() as u8;
            pixels[at + 3] = (a * 255.0).round() as u8;
        }
    }
    pixels
}

/// A rectangle: x, y, width, height.
pub type Piece = (i32, i32, i32, i32);

/// Length of a dash and of the gap after it (the `Dashed` style).
pub const DASH: i32 = 12;
pub const GAP: i32 = 8;
/// Cut the straight pieces of a border into dashes along their long side.
pub fn dashed(pieces: &[Piece]) -> Vec<Piece> {
    let mut dashes = Vec::new();
    for &(x, y, w, h) in pieces {
        if w <= 0 || h <= 0 {
            continue;
        }
        let horizontal = w >= h;
        let length = if horizontal { w } else { h };
        let mut at = 0;
        while at < length {
            let run = DASH.min(length - at);
            dashes.push(if horizontal {
                (x + at, y, run, h)
            } else {
                (x, y + at, w, run)
            });
            at += DASH + GAP;
        }
    }
    dashes
}

/// The `Double` style: two thin lines with a gap between them, together as
/// wide as the border. `outer` is the border's outer rectangle.
pub fn double(outer: Piece, thickness: i32) -> Vec<Piece> {
    let (x, y, w, h) = outer;
    // A third each for the outer line, the gap and the inner line.
    let line = (thickness / 3).max(1);
    let inset = thickness - line;
    let ring = |rx: i32, ry: i32, rw: i32, rh: i32| {
        [
            (rx, ry, rw, line),                               // top
            (rx, ry + rh - line, rw, line),                   // bottom
            (rx, ry + line, line, rh - 2 * line),             // left
            (rx + rw - line, ry + line, line, rh - 2 * line), // right
        ]
    };
    let mut pieces = ring(x, y, w, h).to_vec();
    if inset > line {
        pieces.extend(ring(x + inset, y + inset, w - 2 * inset, h - 2 * inset));
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dashed_border_is_dashes_with_gaps_along_each_side() {
        // A top piece 100 long and 4 thick: dashes of 12 every 20.
        let dashes = dashed(&[(10, 5, 100, 4)]);
        assert_eq!(dashes.len(), 5);
        assert_eq!(dashes[0], (10, 5, 12, 4));
        assert_eq!(dashes[1], (30, 5, 12, 4));
        assert_eq!(dashes[4], (90, 5, 12, 4));
        // A side piece is cut the other way; the last dash stops at the end.
        let side = dashed(&[(0, 0, 4, 30)]);
        assert_eq!(side, [(0, 0, 4, 12), (0, 20, 4, 10)]);
        assert!(dashed(&[(0, 0, 0, 10)]).is_empty());
    }

    #[test]
    fn a_double_border_is_two_lines_with_a_gap() {
        // 6 wide: an outer line of 2, a gap of 2, an inner line of 2.
        let pieces = double((100, 100, 200, 100), 6);
        assert_eq!(pieces.len(), 8);
        assert_eq!(pieces[0], (100, 100, 200, 2), "outer top");
        assert_eq!(pieces[4], (104, 104, 192, 2), "inner top");
        // Across the left side at mid height: line, gap, line.
        let covered = |x: i32| {
            pieces
                .iter()
                .any(|(px, py, pw, ph)| x >= *px && x < px + pw && 150 >= *py && 150 < py + ph)
        };
        let across: Vec<bool> = (100..106).map(covered).collect();
        assert_eq!(across, [true, true, false, false, true, true]);
        // A border too thin for two lines is one line.
        assert_eq!(double((0, 0, 50, 50), 2).len(), 4);
    }

    fn pixel(tile: &[u8], size: i32, x: i32, y: i32) -> (u8, u8, u8, u8) {
        let at = ((y * size + x) * 4) as usize;
        (tile[at + 2], tile[at + 1], tile[at], tile[at + 3])
    }

    #[test]
    fn a_corner_is_a_quarter_ring() {
        let (width, color) = (4, (0, 170, 255));
        let size = tile_size(width);
        assert_eq!(size, 8);
        let tile = corner_tile(width, color, 255, Corner::TopLeft);
        // The outermost corner of the tile is outside the ring: clear.
        assert_eq!(pixel(&tile, size, 0, 0).3, 0);
        // Where the ring meets the straight edges it is the full colour:
        // the middle of the band, on the tile's far column and far row.
        assert_eq!(pixel(&tile, size, size - 1, 1), (0, 170, 255, 255));
        assert_eq!(pixel(&tile, size, 1, size - 1), (0, 170, 255, 255));
        // Inside the ring (towards the window) it is clear again.
        assert_eq!(pixel(&tile, size, size - 1, size - 1).3, 0);
        // On the diagonal, the ring is crossed once: clear, colour, clear.
        let along: Vec<u8> = (0..size).map(|i| pixel(&tile, size, i, i).3).collect();
        let opaque = along.iter().filter(|alpha| **alpha == 255).count();
        assert!(
            opaque >= 2 && along[0] == 0 && along[size as usize - 1] == 0,
            "{along:?}"
        );
    }

    #[test]
    fn the_four_corners_mirror_each_other() {
        let size = tile_size(4);
        let tiles: Vec<Vec<u8>> = CORNERS
            .iter()
            .map(|corner| corner_tile(4, (200, 40, 40), 255, *corner))
            .collect();
        for y in 0..size {
            for x in 0..size {
                let top_left = pixel(&tiles[0], size, x, y);
                assert_eq!(top_left, pixel(&tiles[1], size, size - 1 - x, y));
                assert_eq!(top_left, pixel(&tiles[2], size, x, size - 1 - y));
                assert_eq!(top_left, pixel(&tiles[3], size, size - 1 - x, size - 1 - y));
            }
        }
    }

    #[test]
    fn the_colour_is_multiplied_by_the_alpha() {
        let size = tile_size(4);
        let tile = corner_tile(4, (200, 100, 50), 128, Corner::TopLeft);
        let (r, g, b, a) = pixel(&tile, size, size - 1, 1);
        assert_eq!(a, 128);
        assert_eq!((r, g, b), (100, 50, 25));
        // A border wider than the radius still gets a tile it fits in.
        assert_eq!(tile_size(12), 12);
        assert_eq!(
            corner_tile(12, (1, 2, 3), 255, Corner::TopLeft).len(),
            12 * 12 * 4
        );
    }
}
