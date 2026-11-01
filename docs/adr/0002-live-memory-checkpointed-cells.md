# ADR 0002: The mold lives in memory; cells are checkpointed

Status: accepted, 2026-11-01

## Context

Murmuration treats the files as the only truth: it reads every cell and
writes every cell every tick. With 4,000 agents and a trail in all 384 cells,
that is 384 replicated transactions six times a second. The first version of
Physarum did it, and the replica grew by about half a gigabyte a minute until
the disk filled.

## Decision

The conductor keeps the live mold (agents and trail) in memory for the cells
it holds, and checkpoints each cell to its file about every 3 s, staggered so
that an eighteenth of the cells are written each tick. It still reads every
cell every tick, to see holes and changes of hand:

- a cell whose (host, epoch) changed is reloaded from what was read;
- a cell that became a hole is dropped: its agents go back to their last
  checkpoint in a served cell, or freeze in the hole;
- a checkpoint deletes a copy only if a newer copy is acknowledged in another
  cell (`mold::moves::superseded`, `verus/src/moves.rs`);
- a cell holding an agent whose last checkpoint is more than two cycles old
  is due at once.

## Consequences

- About 100 MB a minute of replica instead of 500.
- The RPO the mold sees is the checkpoint age plus the replication lag.
- No order of checkpoints drops an agent; only a rewind can.
- It is the shape an agent swarm on the fleet would use: work in memory,
  persist what must survive.
