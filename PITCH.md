# Physarum

**A swarm that coordinates only through the database.** A slime mold grown in
384 SQLite files, as a test of a simple idea: give each agent a tenant's slice
of the fleet, give it local rules, and let the shared medium do the
coordinating.

*Physarum polycephalum* has no brain. Each bit of it senses the chemical trail
just ahead, turns toward the strongest, moves, and lays more trail; the trail
spreads and fades. Out of that comes a network that links food sources by
short paths, famously close to the Tokyo rail map. No agent ever reads
another. Everything one agent knows about the rest is in the trail, and here
the trail is in the cells' files.

## The moment

A dark dish with seven oat flakes. Within a few seconds amber threads start
to join them; within a minute the threads have thickened into veins along the
shortest routes and the stragglers have withered. Click to drop a new flake
and a branch reaches for it.

You press **kill -9** on host-a. A scatter of squares goes black across the
whole dish, and the agents inside them freeze as grey specks. The veins do not
stop. Where a vein ran through a dead cell, the mold routes around it, the way
the real organism routes around a cut. You press **Start** on host-c. Ten
seconds later the dark squares fill back in from the replica, a few seconds
stale: the old vein reappears, faded, and the network decides between the old
route and the detour.

## Where it could go

- Agents that are real: each agent an LLM worker, the trail the evidence of
  useful work (tests that went green, reviews approved), position an
  embedding of what it is working on. Separation and trail-following, with
  the fleet as the shared memory.
- Two hosts each growing their own cells, with no central conductor: the
  trail at a cell boundary is the only thing they share.
- Live range moves without a kill.
