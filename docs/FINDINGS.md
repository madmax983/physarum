# Findings: building Physarum on autumn-web 0.9's fleet

Murmuration's findings stand (one replication thread per process, so a busy
fleet's lag passes `rpo_secs`; fencing holds; ownership follows the tenant's
slot). Physarum adds two, both about write volume.

1. **Writing every cell every tick filled the disk.** The first version wrote
   all 384 cells six times a second, as Murmuration does with its birds. The
   lab directory, almost all of it the replica, reached 1.4 GB in about three
   minutes, and the cloud box ran out of space. Every committed transaction
   on a replicated fleet database is shipped and kept for `retention_hours`
   (default 168). The guides say what replication costs offsite ("the size of
   your WAL writes"); worth a sentence that a fleet with many small, frequent
   transactions pays that per transaction, per database.

2. **Live memory with staggered checkpoints is the shape that fits.** Keeping
   the mold in the conductor's memory and checkpointing each cell every
   ~3 s cut the write volume about twentyfold (≈100 MB a minute), and still
   showed failover honestly: a cell that changes hands is reloaded from its
   file, so a takeover rewinds those cells to whatever the replica held. The
   cost is that the checkpoint interval adds to the RPO: a takeover rewound
   agents and trail by up to 4.2 s (release) and 10.8 s (debug), and lost 322
   of 4,000 agents (those whose last checkpoint was in host-a's cells and had
   not shipped). For an agent swarm on the fleet this is probably the right
   default: agents act in memory, and the fleet holds what must survive.

3. **A staggered checkpoint can starve an agent.** An agent crossing cell
   edges in step with the stagger can always be in a cell that is not due.
   The integration test found one: population stuck at 3,999 of 4,000. Any
   cell holding an agent whose checkpoint is more than two cycles old is now
   due too, and the bound is asserted on every tick.

4. **Fencing held again.** Zero stale epochs and zero double serves across
   the test and every drill; the thawed zombie served nothing.

5. **Small:** diesel's `FirstDsl::first` shadows the slice's `first()` on a
   `Vec<T>` of `QueryableByName` rows when `diesel_async::RunQueryDsl` is in
   scope; `<[T]>::first(&rows)` spells around it.
