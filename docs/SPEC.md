# Physarum — specification

A slime mold grown in a SQLite fleet split between hosts. The mold's agents
and the trail they follow live in 384 tenant files; the conductor grows it in
memory and checkpoints each cell to its file. Kill a host and its cells go
dark; the veins route around the holes until a standby restores them.

## 1. The dish

- A torus of `24 × 16` **cells**, each `40 × 40` units, `960 × 640` in all.
- Every cell is one tenant of a `[database.fleet] mode = "tenant"` fleet,
  tenant id `c-CC-RR`: one SQLite file with an `agents` table and a one-row
  `trail` table.
- Ownership is by the slot of the tenant id: host-a serves a scatter of 152
  cells, host-b the other 232.

Geometry is Murmuration's, unchanged (`crates/mold/src/geom.rs`,
`verus/src/geom.rs`): every point lands in a cell (1.1), a cell's key and
index round-trip (1.2), the torus neighbourhood is reflexive and symmetric
(1.3).

## 2. A tick (six a second)

The live mold is in the conductor's memory; the files are its durable state.

1. **Read** every cell from every host. A cell no host answers is a **hole**.
2. **Reload** any cell that changed hands (another host, or a new epoch)
   from what was read: its agents and its trail. **Drop** any cell that
   became a hole: its agents go back to their last checkpoint if that is in
   a served cell, or freeze in the hole.
3. **Resolve** one copy per agent; older copies are ghosts, deleted at their
   cell's next checkpoint.
4. **Count** against each agent's last acknowledged checkpoint (§5).
5. **Step** four times: every agent senses, turns, moves one texel and
   deposits; food deposits; the trail spreads and fades.
6. **Save** a staggered eighteenth of the served cells, plus any cell holding
   an agent whose last checkpoint is more than two cycles old (§4.5).

## 3. The mold (`crates/mold/src/agent.rs`, `trail.rs`)

Jeff Jones's 2010 agent model of *Physarum polycephalum*: each agent senses
the trail 4.5 texels ahead at −0.7, 0 and +0.7 rad, turns 0.6 rad toward the
strongest (randomly when the middle is weakest), moves one texel, and
deposits 5. Food deposits 60 on its 3 × 3 block every step. The trail is 192 ×
128 texels, 8 × 8 per cell, one byte each when stored.

- **3.1** No agent moves into a hole: a step that would is refused and the
  agent faces a new way. Counts, ids and ticks preserved; positions stay in
  the dish. Property test.
- **3.2** One pass of spreading and fading never raises the highest texel:
  each texel moves `spread` (0.5) of the way toward the mean of its served
  neighbourhood, then loses `decay` (0.08). Holes are neither read nor
  written — formal (`verus/src/trail.rs`, integer model), property test (f32).
- **3.3** A stored texel is always a byte: rounded, clamped to 0..=255 —
  formal (`store`), test.
- **3.4** Deterministic: the same agents, trail, holes, food and tick give the
  same result. Test.

## 4. Checkpoints (`crates/mold/src/moves.rs`, `verus/src/moves.rs`)

- **4.1** A checkpoint of cell C upserts the agents now in C, writes C's
  trail, and deletes a copy C's file holds only if a newer copy of that
  agent is acknowledged in another cell (`superseded`).
- **4.2** **Checkpoints never drop an agent.** Over any order of checkpoints,
  any of which may fail, and any moves in memory, an agent that was ever
  acknowledged is in some file — formal (`theorem_moves_never_drop`) and
  property test (`checkpoints_never_drop_an_agent`).
- **4.3** They can duplicate; resolution keeps the newest copy (§5.1).
- **4.4** An upsert never replaces a newer row and a delete only removes rows
  older than its `below`; the trail never replaces a newer trail. Host SQL,
  unit test.
- **4.5** No agent's last checkpoint is more than about two cycles old (36
  ticks): an agent crossing cell edges in step with the stagger forces its
  cell due. Asserted on every tick of `tests/dish.rs` once the dish is full.
- **4.6** **Loss needs a rewind.** By 4.2, an agent vanishes only when a cell
  goes back in time: a reload from a replica that had not received its last
  checkpoint.

## 5. Resolution and the headcount

Murmuration's, unchanged: the newest copy wins, ties to the lower cell
(`verus/src/resolve.rs`, 5.1); each expected agent is present, rewound, in a
hole, missing or lost (5.2). A rewind is reported with how far back it went,
for agents and for each cell's trail (5.3). **The headcount adds up** (5.4):
`population == released + found - lost` on every tick.

## 6. Fencing

A per-cell epoch watermark on every read and write answer; two hosts
answering one cell is a double serve. Both stay at zero.

## 7. The lab (`tests/dish.rs`, `tools/drill.py`)

host-a (`0-8191`, `:3301`), host-b (`8192-16383`, `:3302`), host-c (standby
for `0-8191`, empty volume, `:3303`); the test uses `:4301–4303`. Shared
control database and replica; `lease_secs = 6`, `rpo_secs = 2`,
`sync_interval_secs = 1`.

| # | Claim |
|---|---|
| 7.1 | The cells each primary serves are exactly those whose tenant id's slot is in its range, and every agent reaches a checkpoint. |
| 7.2 | `kill -9` of host-a makes exactly its cells holes; agents whose checkpoint is there freeze; no moving agent is ever in a hole. |
| 7.3 | The standby fills every hole from the replica. |
| 7.4 | Fail-back returns exactly host-a's cells; the thawed standby serves none and waits. |
| 7.5 | Throughout: no stale epoch, no double serve, 4.5 and 5.4. |

Observed (2-CPU cloud box, release): 40–110 ms a tick with 4,000 agents; the
lab directory grows about 100 MB a minute (Reset wipes it); kill to every hole
closed 9.9 s; fail-back 3.3 s; a takeover lost 322 of 4,000 agents and
rewound 869 agents and 83 cells' trail by up to 4.2 s; zero stale, zero
double.

## 8. Scoreboard — all four files verify

Verus 0.2026.08.23.fbbbbcf, run by `./verify.sh` on 2026-11-01.

| Obligation | File | Result | Notes |
|---|---|---|---|
| 4.2 | `moves.rs` | verified | Passed first time. One invariant over a map of sets; the guard does the work. |
| 3.2, 3.3 | `trail.rs` | verified (14) | Needed one pass: a `nonlinear_arith` block sees only its own `requires`, so the bound on the blend is split into `q·here <= q·m`, `p·mean <= p·m` and `q·m + p·m == 1000·m`. |
| 5.1 | `resolve.rs` | verified | Passed first time. |
| 1.1, 1.2, 1.3 | `geom.rs` | verified (20) | Needed one pass: `int` constants must be `spec const`; `lemma_wrap_neg` now proves `(-v) % n` from `v == k·n + r` with vstd's `lemma_fundamental_div_mod_converse`. |
