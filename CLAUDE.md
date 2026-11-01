# CLAUDE.md — Physarum

A slime mold whose agents and trail live in a SQLite tenant fleet split
between hosts, with ownership and replication. Read `PITCH.md`, then
`docs/SPEC.md`. Built from Murmuration; the fleet, host and headcount parts
are the same.

- `crates/mold` — pure (no IO, async or clock): `geom`, `trail` (8 × 8 bytes
  a cell, spread and fade), `agent` (Jones 2010: sense, turn, move, deposit;
  food), `moves` (`superseded`: the checkpoint delete rule), `resolve`,
  `headcount`, `watermark`, `ranges`.
- `crates/physarum-app/src/host.rs` — batch read and write; each cell's
  agents, deletes and trail in one `BEGIN IMMEDIATE … COMMIT`.
- `crates/physarum-app/src/console/conductor.rs` — the live mold in memory;
  reload on change of hands, drop on hole, staggered checkpoints.
- `verus/src/` — `moves.rs`, `trail.rs`, `resolve.rs`, `geom.rs`. Change the
  runtime and the model together.

Rules that matter here:

- Delete a copy from a cell's file only through `moves::superseded`: a newer
  copy must already be acknowledged elsewhere. That is what makes
  checkpoints unable to drop an agent.
- A cell is reloaded from its file whenever its (host, epoch) changes. Do not
  "optimise" that away: it is how a takeover shows what the replica held.
- Every cell is due at least every two cycles for the agents in it (SPEC 4.5);
  the stagger alone can starve an agent.
- `population == released + found - lost` is asserted every tick; a new way
  for an agent to appear or vanish must be counted.
- The host formats only numbers it parsed and checked finite, and hex it
  decoded, into SQL.
- Write load matters: every checkpoint is replicated. Keep checkpoints
  staggered; the lab grows about 100 MB a minute as it is.
- Ports: console 3000, hosts 3301–3303; `tests/dish.rs` uses 4301–4303.

Gates: `./verify.sh` (authoritative, on the Mac). In the cloud:
`cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test --workspace`.
