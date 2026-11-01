//! Moving agents between cells without dropping them (SPEC §4).
//!
//! The conductor moves agents in memory and checkpoints each cell to its own
//! file, in a staggered order. When cell C is checkpointed, it upserts the
//! agents now in C, and it deletes a copy that C's file holds only if a newer
//! copy of that agent is already acknowledged in another cell
//! ([`superseded`]). Formal (`verus/src/moves.rs`): a delete guarded that way
//! never drops an agent, whatever order the checkpoints land in and whichever
//! of them fail. It can leave a duplicate, which [`crate::resolve`] clears.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::agent::AgentId;
use crate::geom::Cell;
use crate::headcount::Expect;

/// A row to delete: this id, at a tick strictly below `below`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delete {
    /// The agent.
    pub id: AgentId,
    /// Delete only rows older than this tick (SPEC 4.4).
    pub below: u64,
}

/// The deletes for a checkpoint of `cell` (SPEC 4.1).
///
/// Every id its file holds that is not in the cell now and whose last
/// acknowledged checkpoint is in another cell. `below` is that checkpoint's
/// tick, so a newer row is never deleted.
#[must_use]
pub fn superseded(
    cell: Cell,
    file: &BTreeSet<AgentId>,
    here: &BTreeSet<AgentId>,
    expect: &BTreeMap<AgentId, Expect>,
) -> Vec<Delete> {
    file.iter()
        .filter(|id| !here.contains(id))
        .filter_map(|id| {
            let e = expect.get(id)?;
            (e.cell != cell).then_some(Delete {
                id: *id,
                below: e.tick,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::CELLS;
    use proptest::prelude::*;

    fn c(i: u16) -> Cell {
        Cell::from_index(i).unwrap()
    }

    #[test]
    fn only_copies_superseded_elsewhere_are_deleted() {
        let file: BTreeSet<AgentId> = [AgentId(1), AgentId(2), AgentId(3), AgentId(4)].into();
        let here: BTreeSet<AgentId> = [AgentId(1)].into();
        let expect = BTreeMap::from([
            (
                AgentId(1),
                Expect {
                    cell: c(0),
                    tick: 9,
                    at_ms: 0,
                },
            ),
            (
                AgentId(2),
                Expect {
                    cell: c(5),
                    tick: 9,
                    at_ms: 0,
                },
            ),
            (
                AgentId(3),
                Expect {
                    cell: c(0),
                    tick: 8,
                    at_ms: 0,
                },
            ),
        ]);
        // 1 is here; 2 is acknowledged elsewhere; 3's last checkpoint is this
        // cell; 4 has none.
        assert_eq!(
            superseded(c(0), &file, &here, &expect),
            vec![Delete {
                id: AgentId(2),
                below: 9
            }]
        );
    }

    /// The files, the acknowledgements and memory, as the conductor keeps
    /// them, and the protocol run over random moves and failures.
    #[derive(Debug, Default)]
    struct Model {
        files: BTreeMap<Cell, BTreeMap<AgentId, u64>>,
        file_ids: BTreeMap<Cell, BTreeSet<AgentId>>,
        expect: BTreeMap<AgentId, Expect>,
        mem: BTreeMap<AgentId, Cell>,
    }

    impl Model {
        fn checkpoint(&mut self, cell: Cell, tick: u64, fails: bool) {
            let here: BTreeSet<AgentId> = self
                .mem
                .iter()
                .filter(|(_, c)| **c == cell)
                .map(|(id, _)| *id)
                .collect();
            let file = self.file_ids.get(&cell).cloned().unwrap_or_default();
            let dels = superseded(cell, &file, &here, &self.expect);
            if fails {
                return;
            }
            let rows = self.files.entry(cell).or_default();
            for id in &here {
                rows.insert(*id, tick);
            }
            for d in &dels {
                if rows.get(&d.id).is_some_and(|t| *t < d.below) {
                    rows.remove(&d.id);
                }
            }
            let f = self.file_ids.entry(cell).or_default();
            for d in &dels {
                f.remove(&d.id);
            }
            for id in &here {
                f.insert(*id);
                self.expect.insert(
                    *id,
                    Expect {
                        cell,
                        tick,
                        at_ms: 0,
                    },
                );
            }
        }

        fn in_some_file(&self, id: AgentId) -> bool {
            self.files.values().any(|r| r.contains_key(&id))
        }
    }

    proptest! {
        /// SPEC 4.2, run: agents move in memory, cells checkpoint in any
        /// order, any checkpoint can fail; an agent that was ever
        /// acknowledged is always in some file.
        #[test]
        fn checkpoints_never_drop_an_agent(
            start in proptest::collection::btree_map(1u64..40, 0u16..12, 1..40),
            steps in proptest::collection::vec((0u64..40, 0u16..12, 0u16..12, any::<bool>()), 1..200),
        ) {
            let mut m = Model::default();
            for (&id, &cell) in &start {
                m.mem.insert(AgentId(id), c(cell));
            }
            for cell in 0..12 {
                m.checkpoint(c(cell), 1, false);
            }
            for (tick, (who, to, ckpt, fails)) in steps.into_iter().enumerate() {
                let tick = tick as u64 + 2;
                if let Some(cell) = m.mem.get_mut(&AgentId(who)) {
                    *cell = c(to);
                }
                m.checkpoint(c(ckpt), tick, fails);
                for id in m.expect.keys() {
                    prop_assert!(m.in_some_file(*id), "agent {:?} is in no file", id);
                }
            }
            let _ = CELLS;
        }
    }
}
