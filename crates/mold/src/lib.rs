//! Physarum's engine: a slime mold on a sky of 384 cells, each one file of a
//! `SQLite` fleet. No IO, no async, no clock.
//!
//! - [`geom`] — 24 × 16 cells on a torus; each cell is one tenant file
//!   (`verus/src/geom.rs`).
//! - [`trail`] — the chemoattractant, 8 × 8 texels a cell; spreads and fades
//!   (`verus/src/trail.rs`).
//! - [`agent`] — sense, turn, move, deposit (Jones 2010); food.
//! - [`moves`] — insert-then-delete between cells; never drops an agent
//!   (`verus/src/moves.rs`).
//! - [`resolve`] — one copy per agent, the newest (`verus/src/resolve.rs`).
//! - [`headcount`] — present, rewound, in a hole, missing, lost.
//! - [`watermark`] — per-cell epoch fencing on the conductor's side.
//! - [`ranges`] — slot ranges and the lab's three hosts.
#![forbid(unsafe_code)]

pub mod agent;
pub mod geom;
pub mod headcount;
pub mod moves;
pub mod ranges;
pub mod resolve;
pub mod trail;
pub mod watermark;

pub use agent::{Agent, AgentId};
pub use geom::Cell;
pub use ranges::{HostSpec, SLOTS, SlotRange, lab};
