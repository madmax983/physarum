//! The trail: the chemoattractant the mold lays down and follows.
//!
//! The map is 192 × 128 texels, 5 units square: 8 × 8 texels per cell of the
//! sky. Each cell's 64 texels are stored in that cell's file as 64 bytes, so
//! the trail is as partitioned as the agents are. A texel in a hole cell has
//! no value: nobody can read or write it.
//!
//! Formal (`verus/src/trail.rs`): a diffusion-and-decay pass never raises the
//! highest texel, and storage clamps every texel into a byte.

use std::collections::BTreeSet;

use crate::geom::{CELL, COLS, Cell, HEIGHT, ROWS, WIDTH, wrap};

/// Texels per cell side.
pub const TEX: u16 = 8;
/// Texels across the map.
pub const TW: u16 = COLS * TEX;
/// Texels down the map.
pub const TH: u16 = ROWS * TEX;
/// A texel's side, in units.
pub const TEXEL: f32 = CELL / TEX as f32;
/// Bytes a cell stores.
pub const CELL_BYTES: usize = (TEX as usize) * (TEX as usize);
/// The strongest a texel can be.
pub const MAX: f32 = 255.0;

/// The whole map, as the conductor works on it during a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Trail {
    data: Vec<f32>,
}

impl Default for Trail {
    fn default() -> Self {
        Self::new()
    }
}

/// The texel a point is in, after wrapping it into the sky.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "wrapped and floored into range"
)]
pub fn texel_of(x: f32, y: f32) -> (u16, u16) {
    let tx = (wrap(x, WIDTH) / TEXEL).floor() as u16;
    let ty = (wrap(y, HEIGHT) / TEXEL).floor() as u16;
    (tx.min(TW - 1), ty.min(TH - 1))
}

/// The cell a texel belongs to.
#[must_use]
pub fn cell_of_texel(tx: u16, ty: u16) -> Cell {
    Cell::new(tx / TEX, ty / TEX)
        .unwrap_or_else(|| Cell::new(0, 0).unwrap_or_else(|| unreachable!()))
}

impl Trail {
    /// An empty map.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: vec![0.0; usize::from(TW) * usize::from(TH)],
        }
    }

    fn at(tx: u16, ty: u16) -> usize {
        usize::from(ty) * usize::from(TW) + usize::from(tx)
    }

    /// The value at a texel.
    #[must_use]
    pub fn get(&self, tx: u16, ty: u16) -> f32 {
        self.data[Self::at(tx % TW, ty % TH)]
    }

    /// Add to a texel, up to [`MAX`].
    pub fn add(&mut self, tx: u16, ty: u16, v: f32) {
        let i = Self::at(tx % TW, ty % TH);
        self.data[i] = (self.data[i] + v).clamp(0.0, MAX);
    }

    /// The highest texel.
    #[must_use]
    pub fn max(&self) -> f32 {
        self.data.iter().copied().fold(0.0, f32::max)
    }

    /// Load a cell's 64 stored bytes. Short input fills with zeros.
    pub fn set_cell(&mut self, cell: Cell, bytes: &[u8]) {
        for k in 0..CELL_BYTES {
            let (dx, dy) = (
                u16::try_from(k % 8).unwrap_or(0),
                u16::try_from(k / 8).unwrap_or(0),
            );
            let i = Self::at(cell.col() * TEX + dx, cell.row() * TEX + dy);
            self.data[i] = f32::from(bytes.get(k).copied().unwrap_or(0));
        }
    }

    /// A cell's 64 bytes for storage: each texel rounded and clamped into a
    /// byte (SPEC 3.3).
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=255 first"
    )]
    pub fn cell_bytes(&self, cell: Cell) -> Vec<u8> {
        (0..CELL_BYTES)
            .map(|k| {
                let (dx, dy) = (
                    u16::try_from(k % 8).unwrap_or(0),
                    u16::try_from(k / 8).unwrap_or(0),
                );
                let v = self.data[Self::at(cell.col() * TEX + dx, cell.row() * TEX + dy)];
                v.round().clamp(0.0, MAX) as u8
            })
            .collect()
    }

    /// The whole map as bytes, row by row (for the canvas).
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=255 first"
    )]
    pub fn bytes(&self, holes: &BTreeSet<Cell>) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len());
        for ty in 0..TH {
            for tx in 0..TW {
                let v = if holes.contains(&cell_of_texel(tx, ty)) {
                    0.0
                } else {
                    self.get(tx, ty)
                };
                out.push(v.round().clamp(0.0, MAX) as u8);
            }
        }
        out
    }

    /// One pass of spreading and fading (SPEC 3.2): every texel outside the
    /// holes moves `spread` of the way toward the mean of itself and its
    /// served neighbours, then loses `decay` of itself. Holes are neither read
    /// nor written.
    #[allow(clippy::cast_precision_loss, reason = "at most nine neighbours")]
    pub fn diffuse(&mut self, holes: &BTreeSet<Cell>, spread: f32, decay: f32) {
        let mut blocked = vec![false; usize::from(crate::geom::CELLS)];
        for c in holes {
            blocked[usize::from(c.index())] = true;
        }
        let (w, h) = (usize::from(TW), usize::from(TH));
        let tex = usize::from(TEX);
        let cols = usize::from(COLS);
        let hole = |x: usize, y: usize| blocked[(y / tex) * cols + x / tex];
        let mut next = self.data.clone();
        for y in 0..h {
            let rows = [(y + h - 1) % h, y, (y + 1) % h];
            for x in 0..w {
                if hole(x, y) {
                    continue;
                }
                let cols3 = [(x + w - 1) % w, x, (x + 1) % w];
                let (mut sum, mut n) = (0.0f32, 0u32);
                for &ny in &rows {
                    for &nx in &cols3 {
                        if !hole(nx, ny) {
                            sum += self.data[ny * w + nx];
                            n += 1;
                        }
                    }
                }
                let here = self.data[y * w + x];
                next[y * w + x] = spread.mul_add(sum / n as f32 - here, here) * (1.0 - decay);
            }
        }
        self.data = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::CELLS;
    use proptest::prelude::*;

    #[test]
    fn a_cell_round_trips_through_its_bytes() {
        let mut t = Trail::new();
        let c = Cell::new(5, 3).unwrap();
        let bytes: Vec<u8> = (0..64u8).map(|i| i * 3).collect();
        t.set_cell(c, &bytes);
        assert_eq!(t.cell_bytes(c), bytes);
        // Other cells untouched.
        assert!(
            t.cell_bytes(Cell::new(6, 3).unwrap())
                .iter()
                .all(|b| *b == 0)
        );
        // A cell's texels are where the map says they are.
        assert!((t.get(5 * TEX + 1, 3 * TEX) - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn texels_and_cells_agree() {
        assert_eq!(texel_of(0.0, 0.0), (0, 0));
        assert_eq!(texel_of(959.9, 639.9), (TW - 1, TH - 1));
        assert_eq!(texel_of(-0.1, -0.1), (TW - 1, TH - 1));
        for c in Cell::all() {
            let (x, y) = c.centre();
            let (tx, ty) = texel_of(x, y);
            assert_eq!(cell_of_texel(tx, ty), c);
        }
    }

    #[test]
    fn diffusion_spreads_fades_and_leaves_holes_alone() {
        let mut t = Trail::new();
        t.add(40, 40, 200.0);
        let hole = Cell::new(0, 0).unwrap();
        let mut b = vec![0u8; CELL_BYTES];
        b[0] = 99;
        t.set_cell(hole, &b);
        let holes: BTreeSet<Cell> = [hole].into();
        t.diffuse(&holes, 1.0, 0.1);
        assert!(t.get(40, 40) < 200.0 && t.get(41, 40) > 0.0 && t.get(39, 41) > 0.0);
        assert!(
            (t.get(0, 0) - 99.0).abs() < f32::EPSILON,
            "a hole was written"
        );
        assert!(t.get(8, 0).abs() < f32::EPSILON || t.get(8, 0) > 0.0);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]
        /// SPEC 3.2 and 3.3: never above the highest texel, never negative,
        /// every stored byte within range.
        #[test]
        fn diffusion_never_raises_the_peak(
            spots in proptest::collection::vec((0u16..192, 0u16..128, 0f32..255.0), 0..60),
            holes in proptest::collection::btree_set(0u16..CELLS, 0..40),
            decay in 0f32..0.5,
            spread in 0f32..=1.0,
        ) {
            let mut t = Trail::new();
            for (x, y, v) in spots { t.add(x, y, v); }
            let holes: BTreeSet<Cell> = holes.into_iter().filter_map(Cell::from_index).collect();
            let before = t.max();
            t.diffuse(&holes, spread, decay);
            prop_assert!(t.max() <= before + 1e-3);
            prop_assert!(t.data.iter().all(|v| *v >= 0.0));
            for c in Cell::all() { prop_assert_eq!(t.cell_bytes(c).len(), CELL_BYTES); }
        }
    }
}
