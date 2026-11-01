//! The mold's agents: each senses the trail ahead of it, turns toward the
//! strongest, moves one texel, and deposits trail where it lands.
//!
//! This is Jeff Jones's 2010 model of *Physarum polycephalum*. No agent ever
//! reads another agent: everything they know about each other is in the
//! trail, and the trail is in the cells' files.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::geom::{Cell, HEIGHT, WIDTH, cell_of, wrap};
use crate::trail::{TEXEL, Trail, cell_of_texel, texel_of};

/// An agent's id. Never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub u64);

/// One agent, as a row of its cell's `agents` table. `(vx, vy)` is its
/// heading times its speed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    /// Its id.
    pub id: AgentId,
    /// Position, units.
    pub x: f32,
    /// Position, units.
    pub y: f32,
    /// Heading × speed, units per step.
    pub vx: f32,
    /// Heading × speed, units per step.
    pub vy: f32,
    /// The tick that wrote this row.
    pub tick: u64,
}

impl Agent {
    /// The cell it is in.
    #[must_use]
    pub fn cell(&self) -> Cell {
        cell_of(self.x, self.y)
    }

    /// Whether every field is a finite number (the host refuses others).
    #[must_use]
    pub const fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.vx.is_finite() && self.vy.is_finite()
    }
}

/// How far ahead an agent senses, in units.
pub const SENSE: f32 = 4.5 * TEXEL;
/// The angle between its sensors, radians.
pub const SENSE_ANGLE: f32 = 0.7;
/// How far it turns in one step, radians.
pub const TURN: f32 = 0.6;
/// How far it moves in one step: one texel.
pub const SPEED: f32 = TEXEL;
/// Trail it lays where it lands.
pub const DEPOSIT: f32 = 5.0;
/// The fraction of trail lost per step.
pub const DECAY: f32 = 0.08;
/// How far each step's spreading moves a texel toward its neighbourhood's
/// mean (1 would be a full blur; less keeps the veins thin).
pub const SPREAD: f32 = 0.5;
/// Trail a food source puts out per step, on each texel of its 3 × 3 block.
pub const FOOD: f32 = 60.0;
/// What a sensor reads inside a hole: worse than any trail.
pub const HOLE_SENSE: f32 = -1_000.0;

/// splitmix64: a small, good, deterministic hash.
#[must_use]
pub const fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A uniform number in `0..1` from `seed`.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "a 24-bit fraction")]
pub fn unit(seed: u64) -> f32 {
    (mix(seed) >> 40) as f32 / (1u64 << 24) as f32
}

fn sense(trail: &Trail, holes: &BTreeSet<Cell>, x: f32, y: f32, angle: f32) -> f32 {
    let (tx, ty) = texel_of(SENSE.mul_add(angle.cos(), x), SENSE.mul_add(angle.sin(), y));
    if holes.contains(&cell_of_texel(tx, ty)) {
        HOLE_SENSE
    } else {
        trail.get(tx, ty)
    }
}

/// One step for every agent, then food, then one pass of diffusion and decay.
///
/// Agents in holes are not in `agents` (nobody can read them). An agent never
/// moves into a hole (SPEC 3.1); it turns somewhere new instead.
pub fn step_once(
    agents: &mut [Agent],
    trail: &mut Trail,
    holes: &BTreeSet<Cell>,
    foods: &[(f32, f32)],
    salt: u64,
) {
    for a in agents.iter_mut() {
        let heading = a.vy.atan2(a.vx);
        let f = sense(trail, holes, a.x, a.y, heading);
        let l = sense(trail, holes, a.x, a.y, heading - SENSE_ANGLE);
        let r = sense(trail, holes, a.x, a.y, heading + SENSE_ANGLE);
        let coin = unit(a.id.0 ^ salt);
        let turned = if f >= l && f >= r {
            heading
        } else if f < l && f < r {
            if coin < 0.5 {
                heading - TURN
            } else {
                heading + TURN
            }
        } else if l > r {
            heading - TURN
        } else {
            heading + TURN
        };
        let (vx, vy) = (turned.cos() * SPEED, turned.sin() * SPEED);
        let (nx, ny) = (wrap(a.x + vx, WIDTH), wrap(a.y + vy, HEIGHT));
        if holes.contains(&cell_of(nx, ny)) {
            // Blocked: stay, and face a new way.
            let fresh = coin * std::f32::consts::TAU;
            (a.vx, a.vy) = (fresh.cos() * SPEED, fresh.sin() * SPEED);
            continue;
        }
        (a.x, a.y, a.vx, a.vy) = (nx, ny, vx, vy);
        let (tx, ty) = texel_of(nx, ny);
        trail.add(tx, ty, DEPOSIT);
    }
    for &(fx, fy) in foods {
        let (tx, ty) = texel_of(fx, fy);
        for dy in [crate::trail::TH - 1, 0, 1] {
            for dx in [crate::trail::TW - 1, 0, 1] {
                let (x, y) = ((tx + dx) % crate::trail::TW, (ty + dy) % crate::trail::TH);
                if !holes.contains(&cell_of_texel(x, y)) {
                    trail.add(x, y, FOOD);
                }
            }
        }
    }
    trail.diffuse(holes, SPREAD, DECAY);
}

/// A tick: `steps` steps, every agent stamped with `tick`. Deterministic.
#[must_use]
pub fn step(
    agents: &[Agent],
    trail: &mut Trail,
    holes: &BTreeSet<Cell>,
    foods: &[(f32, f32)],
    tick: u64,
    steps: u32,
) -> Vec<Agent> {
    let mut out: Vec<Agent> = agents.iter().map(|a| Agent { tick, ..*a }).collect();
    for s in 0..steps {
        step_once(
            &mut out,
            trail,
            holes,
            foods,
            mix(tick.wrapping_mul(64) + u64::from(s)),
        );
    }
    out
}

/// The first mold: `n` agents scattered over the whole sky, facing every way.
#[must_use]
pub fn seed(n: u64) -> Vec<Agent> {
    (1..=n)
        .map(|i| {
            let h = mix(i);
            let angle = unit(h ^ 5) * std::f32::consts::TAU;
            Agent {
                id: AgentId(i),
                x: unit(h) * WIDTH,
                y: unit(h ^ 3) * HEIGHT,
                vx: angle.cos() * SPEED,
                vy: angle.sin() * SPEED,
                tick: 0,
            }
        })
        .collect()
}

/// Where the food starts: seven oat flakes, roughly where the cities of a
/// small country might be.
#[must_use]
pub fn default_foods() -> Vec<(f32, f32)> {
    vec![
        (150.0, 130.0),
        (470.0, 95.0),
        (800.0, 170.0),
        (250.0, 420.0),
        (520.0, 330.0),
        (760.0, 500.0),
        (430.0, 560.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::CELLS;
    use proptest::prelude::*;

    #[test]
    fn deterministic() {
        let a = seed(300);
        let (mut t1, mut t2) = (Trail::new(), Trail::new());
        let foods = default_foods();
        assert_eq!(
            step(&a, &mut t1, &BTreeSet::new(), &foods, 4, 3),
            step(&a, &mut t2, &BTreeSet::new(), &foods, 4, 3)
        );
        assert_eq!(t1, t2);
    }

    #[test]
    fn an_agent_turns_toward_trail() {
        let mut t = Trail::new();
        // Strong trail ahead-left (up and right of an agent heading right).
        let a = Agent {
            id: AgentId(1),
            x: 100.0,
            y: 100.0,
            vx: SPEED,
            vy: 0.0,
            tick: 0,
        };
        let (tx, ty) = texel_of(
            SENSE.mul_add((-SENSE_ANGLE).cos(), 100.0),
            SENSE.mul_add((-SENSE_ANGLE).sin(), 100.0),
        );
        t.add(tx, ty, 200.0);
        let out = step(&[a], &mut t, &BTreeSet::new(), &[], 1, 1);
        assert!(out[0].vy < 0.0, "{:?}", out[0]);
    }

    #[test]
    fn food_grows_trail() {
        let mut t = Trail::new();
        let _ = step(&[], &mut t, &BTreeSet::new(), &default_foods(), 1, 2);
        let (tx, ty) = texel_of(150.0, 130.0);
        assert!(t.get(tx, ty) > 50.0);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]
        /// SPEC 3.1: no agent enters a hole; ids, count and tick preserved;
        /// positions stay in the sky.
        #[test]
        fn agents_stay_out_of_holes(
            n in 0u64..300,
            holes in proptest::collection::btree_set(0u16..CELLS, 0..150),
            tick in 0u64..10_000,
            steps in 1u32..5,
        ) {
            let holes: BTreeSet<Cell> = holes.into_iter().filter_map(Cell::from_index).collect();
            let start: Vec<Agent> = seed(n).into_iter().filter(|a| !holes.contains(&a.cell())).collect();
            let mut t = Trail::new();
            let out = step(&start, &mut t, &holes, &default_foods(), tick, steps);
            prop_assert_eq!(out.len(), start.len());
            for (a, b) in start.iter().zip(&out) {
                prop_assert_eq!(a.id, b.id);
                prop_assert_eq!(b.tick, tick);
                prop_assert!(b.is_finite());
                prop_assert!((0.0..WIDTH).contains(&b.x) && (0.0..HEIGHT).contains(&b.y));
                prop_assert!(!holes.contains(&b.cell()), "{:?} entered a hole", b);
            }
        }
    }
}
