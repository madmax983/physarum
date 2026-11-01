//! The conductor: grows the mold across the hosts, six ticks a second.
//!
//! The live mold is in the conductor's memory, as a swarm's working memory
//! would be; the fleet holds its durable state. Every tick:
//!
//! ```text
//!   read     every cell from every host ─► who serves each cell, holes
//!   reload   any cell that changed hands (host or epoch) from what it read
//!   drop     any cell that became a hole; its agents fall back to their last
//!            checkpoint elsewhere, or freeze in the hole
//!   resolve  one copy per agent ─► ghosts to delete
//!   count    against the last checkpoint of each agent ─► in a hole / missing / lost
//!   step     four steps: sense, turn, move, deposit; food; spread and fade
//!   save     a staggered 1/18 of the served cells: their agents, trail, and
//!            deletes of copies that a newer checkpoint elsewhere supersedes
//! ```
//!
//! Each cell is checkpointed about every three seconds. A copy of an agent is
//! deleted from a cell only once a newer copy is acknowledged in another cell
//! (`verus/src/moves.rs`), so no checkpoint order drops an agent.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use mold::agent::{default_foods, seed, step};
use mold::geom::{CELLS, HEIGHT, WIDTH};
use mold::headcount::{Expect, Status, classify};
use mold::moves::superseded;
use mold::resolve::resolve;
use mold::trail::Trail;
use mold::watermark::Watermarks;
use mold::{Agent, AgentId, Cell, HostSpec};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::wire::{
    CellWrite, ReadReq, ReadResp, TrailWrite, WriteReq, WriteResp, from_hex, to_hex,
};

/// How often the conductor ticks.
pub const TICK: Duration = Duration::from_millis(160);
/// Model steps per tick.
pub const STEPS: u32 = 4;
/// How long a host that answered last time gets.
pub const PATIENT: Duration = Duration::from_millis(1_500);
/// How long a host that did not answer last time gets, until it answers.
pub const PROBE: Duration = Duration::from_millis(400);
/// The mold's agents.
pub const SEED: u64 = 4_000;
/// Each cell is checkpointed once every this many ticks (about 3 s).
pub const CHECKPOINT: u64 = 18;
/// Most food at once.
pub const MAX_FOOD: usize = 24;
/// How long a mark stays, in ms.
pub const MARK_MS: u64 = 4_000;

/// A mark on the dish where something happened.
#[derive(Debug, Clone, Serialize)]
pub struct Mark {
    /// `lost`.
    pub kind: &'static str,
    /// Where.
    pub x: f32,
    /// Where.
    pub y: f32,
    /// When, ms.
    pub at_ms: u64,
}

/// An agent lost, with when its last write was acknowledged.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct LostRec {
    /// The agent.
    pub id: u64,
    /// When its last write was acknowledged.
    pub acked_ms: u64,
    /// When it was found in no cell with no holes left.
    pub found_ms: u64,
}

/// Something that came back older than its last acknowledged write: an
/// agent, or a cell's trail.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct RewindRec {
    /// `agent` or `trail`.
    pub kind: &'static str,
    /// How much older, ms: the replica was at least this far behind.
    pub by_ms: u64,
    /// When it was seen, ms.
    pub seen_ms: u64,
}

/// The conductor's counters.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Counts {
    /// Ticks run.
    pub ticks: u64,
    /// Agents lost.
    pub lost: u64,
    /// Agents that came back older.
    pub rewound: u64,
    /// Cells whose trail came back older.
    pub trails_rewound: u64,
    /// Ghost copies deleted.
    pub ghosts: u64,
    /// Answers refused by the epoch watermark.
    pub stale: u64,
    /// Cells two hosts served in one tick.
    pub double: u64,
    /// Cells whose write was not answered (it may have landed).
    pub unacked: u64,
    /// Agents of the first mold whose write was acknowledged.
    pub released: u64,
    /// Agents read that the conductor did not expect.
    pub found: u64,
    /// Cell writes a host refused or failed, with a reason.
    pub failed_writes: u64,
}

/// What the dish looks like after a tick.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Frame {
    /// The tick.
    pub tick: u64,
    /// How long it took, ms.
    pub tick_ms: u64,
    /// The trail, 192 × 128 bytes row by row, as hex. Holes are 0.
    pub trail: String,
    /// Agents, flattened: `x, y, state` per agent, whole units; state 0
    /// moving, 1 frozen in a hole.
    pub agents: Vec<u16>,
    /// For each cell, the lab index of the host that served it, or -1: a hole.
    pub cells: Vec<i8>,
    /// The food.
    pub foods: Vec<(f32, f32)>,
    /// Recent marks.
    pub marks: Vec<Mark>,
    /// Every agent expected: `released + found - lost`.
    pub population: usize,
    /// Moving.
    pub moving: usize,
    /// In holes or unaccounted for.
    pub frozen: usize,
    /// Holes now.
    pub holes: usize,
    /// The oldest last checkpoint among moving agents, in ticks, after this
    /// tick's saves (never more than about `2 × CHECKPOINT`).
    pub oldest_checkpoint: u64,
    /// Counters.
    pub counts: Counts,
    /// The most recent losses.
    pub lost: Vec<LostRec>,
    /// The most recent rewinds.
    pub rewinds: Vec<RewindRec>,
}

/// The mold's state between ticks.
#[derive(Debug, Default)]
pub struct Culture {
    hosts: Vec<HostSpec>,
    tick: u64,
    next_id: u64,
    expect: BTreeMap<AgentId, Expect>,
    trail_expect: BTreeMap<Cell, (u64, u64)>,
    last_seen: BTreeMap<AgentId, Agent>,
    water: Watermarks,
    marks: VecDeque<Mark>,
    lost: VecDeque<LostRec>,
    rewinds: VecDeque<RewindRec>,
    tick_ms: BTreeMap<u64, u64>,
    counts: Counts,
    slow: Vec<bool>,
    seeded: bool,
    foods: Vec<(f32, f32)>,
    /// The live trail, valid for the cells in `mem`.
    trail: Trail,
    /// The live agents of every cell the conductor holds.
    mem: BTreeMap<Cell, Vec<Agent>>,
    /// Who served each held cell last tick, and at what epoch.
    holder: BTreeMap<Cell, (usize, u64)>,
    /// The ids each cell's file holds, as far as the conductor knows.
    file_ids: BTreeMap<Cell, BTreeSet<AgentId>>,
    /// Each agent's last acknowledged checkpoint.
    acked: BTreeMap<AgentId, Agent>,
    /// Ghost copies to delete at each cell's next checkpoint.
    ghosts: BTreeMap<Cell, Vec<mold::moves::Delete>>,
}

/// Something worth a line on the timeline.
pub type Note = String;

async fn post<B: Serialize + Sync, R: DeserializeOwned>(
    client: &reqwest::Client,
    port: u16,
    path: &str,
    body: &B,
    timeout: Duration,
) -> Option<R> {
    let r = client
        .post(format!("http://127.0.0.1:{port}{path}"))
        .timeout(timeout)
        .json(body)
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.json().await.ok()
}

/// One tick's reads.
struct Read {
    /// Which host served each cell, and at what epoch.
    served: BTreeMap<Cell, (usize, u64)>,
    /// Every agent copy read.
    copies: Vec<(Cell, Agent)>,
    /// Each served cell's trail bytes and the tick that wrote them.
    trails: BTreeMap<Cell, (Vec<u8>, u64)>,
}

impl Culture {
    /// A dish that has not started, grown through `hosts`.
    #[must_use]
    pub fn new(hosts: Vec<HostSpec>) -> Self {
        Self {
            slow: vec![false; hosts.len()],
            hosts,
            next_id: SEED + 1,
            foods: default_foods(),
            ..Self::default()
        }
    }

    /// Drop food at `(x, y)` (units). The oldest goes past [`MAX_FOOD`].
    pub fn add_food(&mut self, x: f32, y: f32) {
        if x.is_finite() && y.is_finite() {
            self.foods
                .push((mold::geom::wrap(x, WIDTH), mold::geom::wrap(y, HEIGHT)));
            if self.foods.len() > MAX_FOOD {
                self.foods.remove(0);
            }
        }
    }

    /// Take every piece of food away.
    pub fn clear_food(&mut self) {
        self.foods.clear();
    }

    fn timeout(&self, host: usize) -> Duration {
        if self.slow[host] { PROBE } else { PATIENT }
    }

    async fn read(&mut self, client: &reqwest::Client, notes: &mut Vec<Note>) -> Read {
        let req = ReadReq {
            cells: Cell::all().map(Cell::key).collect(),
        };
        let hosts = self.hosts.clone();
        let mut set = tokio::task::JoinSet::new();
        for (i, h) in hosts.iter().enumerate() {
            let (client, req, port, t) = (client.clone(), req.clone(), h.port, self.timeout(i));
            set.spawn(async move {
                (
                    i,
                    post::<_, ReadResp>(&client, port, "/api/cells/read", &req, t).await,
                )
            });
        }
        let mut answers: Vec<(usize, ReadResp)> = Vec::new();
        while let Some(Ok((i, r))) = set.join_next().await {
            self.slow[i] = r.is_none();
            if let Some(r) = r {
                answers.push((i, r));
            }
        }
        answers.sort_by_key(|(i, _)| *i);
        let mut read = Read {
            served: BTreeMap::new(),
            copies: Vec::new(),
            trails: BTreeMap::new(),
        };
        let mut rows: BTreeMap<Cell, Vec<Agent>> = BTreeMap::new();
        for (i, r) in answers {
            for c in r.cells {
                let Some(cell) = Cell::from_key(&c.cell) else {
                    continue;
                };
                if let Err(e) = self.water.observe(&r.host, cell, c.epoch) {
                    self.counts.stale += 1;
                    notes.push(format!("refused {e}"));
                    continue;
                }
                if let Some(&(j, _)) = read.served.get(&cell) {
                    self.counts.double += 1;
                    notes.push(format!(
                        "{} and {} both served {cell}",
                        hosts[j].name, r.host
                    ));
                }
                read.served.insert(cell, (i, c.epoch));
                rows.insert(cell, c.agents);
                read.trails
                    .insert(cell, (from_hex(&c.trail).unwrap_or_default(), c.trail_tick));
            }
        }
        read.copies = rows
            .into_iter()
            .flat_map(|(cell, a)| a.into_iter().map(move |a| (cell, a)))
            .collect();
        read
    }

    /// Send each host its cells' writes; the cells acknowledged.
    async fn write(
        &mut self,
        client: &reqwest::Client,
        by_cell: BTreeMap<Cell, CellWrite>,
        served: &BTreeMap<Cell, (usize, u64)>,
        notes: &mut Vec<Note>,
    ) -> BTreeSet<Cell> {
        let hosts = self.hosts.clone();
        let mut per: BTreeMap<usize, Vec<CellWrite>> = BTreeMap::new();
        for (cell, w) in by_cell {
            if let Some(&(i, _)) = served.get(&cell) {
                per.entry(i).or_default().push(w);
            }
        }
        let mut set = tokio::task::JoinSet::new();
        for (i, cells) in per {
            let (client, port, t) = (client.clone(), hosts[i].port, self.timeout(i));
            set.spawn(async move {
                let req = WriteReq { cells };
                let n = req.cells.len();
                (
                    i,
                    n,
                    post::<_, WriteResp>(&client, port, "/api/cells/write", &req, t).await,
                )
            });
        }
        let mut acked = BTreeSet::new();
        while let Some(Ok((i, n, r))) = set.join_next().await {
            let Some(r) = r else {
                self.slow[i] = true;
                self.counts.unacked += u64::try_from(n).unwrap_or(0);
                continue;
            };
            if let Some(why) = r.failed.first() {
                self.counts.failed_writes += r.failed.len() as u64;
                notes.push(format!(
                    "{} could not write {} cells: {why}",
                    r.host,
                    r.failed.len()
                ));
            }
            for d in r.done {
                let Some(cell) = Cell::from_key(&d.cell) else {
                    continue;
                };
                if self.water.observe(&r.host, cell, d.epoch).is_ok() {
                    acked.insert(cell);
                } else {
                    self.counts.stale += 1;
                }
            }
        }
        acked
    }

    fn rewind(&mut self, kind: &'static str, read_tick: u64, expected_at: u64, seen: u64) {
        let by_ms = self
            .tick_ms
            .get(&read_tick)
            .map_or(0, |t0| expected_at.saturating_sub(*t0));
        self.rewinds.push_back(RewindRec {
            kind,
            by_ms,
            seen_ms: seen,
        });
    }

    /// Load `cell` from what was read: its agents and trail, replacing
    /// whatever the conductor held. Returns how many agents and whether the
    /// trail came back older than their last checkpoint.
    fn reload(
        &mut self,
        cell: Cell,
        agents: Vec<Agent>,
        trail: &(Vec<u8>, u64),
        seen: u64,
    ) -> (u64, bool) {
        self.trail.set_cell(cell, &trail.0);
        let mut back = 0;
        for a in &agents {
            if let Some(e) = self.expect.get(&a.id)
                && e.cell == cell
                && a.tick < e.tick
            {
                back += 1;
                self.rewind("agent", a.tick, e.at_ms, seen);
            }
        }
        let older = self
            .trail_expect
            .get(&cell)
            .copied()
            .filter(|&(want, _)| trail.1 < want);
        if let Some((_, at)) = older {
            self.rewind("trail", trail.1, at, seen);
        }
        let trail_back = older.is_some();
        self.file_ids
            .insert(cell, agents.iter().map(|a| a.id).collect());
        self.mem.insert(cell, agents);
        (back, trail_back)
    }

    /// Run one tick. `now` is ms on the console's clock.
    #[allow(
        clippy::too_many_lines,
        reason = "one tick, in the order of the module doc"
    )]
    pub async fn tick(
        &mut self,
        client: &reqwest::Client,
        now: impl Fn() -> u64,
    ) -> (Frame, Vec<Note>) {
        let started = Instant::now();
        let mut notes = Vec::new();
        self.tick += 1;
        let t = self.tick;

        // 1. Read.
        let mut read = self.read(client, &mut notes).await;
        let holes: BTreeSet<Cell> = Cell::all()
            .filter(|c| !read.served.contains_key(c))
            .collect();
        let seen = now();

        // 2. Reload what changed hands; drop what became a hole.
        let mut rows: BTreeMap<Cell, Vec<Agent>> = BTreeMap::new();
        for (cell, a) in std::mem::take(&mut read.copies) {
            rows.entry(cell).or_default().push(a);
        }
        let (mut agents_back, mut trails_back) = (0u64, 0u64);
        for (cell, by) in &read.served {
            if self.holder.get(cell) == Some(by) && self.mem.contains_key(cell) {
                continue;
            }
            let trail = read.trails.get(cell).cloned().unwrap_or_default();
            let (back, tb) =
                self.reload(*cell, rows.remove(cell).unwrap_or_default(), &trail, seen);
            agents_back += back;
            trails_back += u64::from(tb);
        }
        let dropped: Vec<Cell> = self
            .mem
            .keys()
            .filter(|c| holes.contains(c))
            .copied()
            .collect();
        let mut fell_back = 0u64;
        for cell in dropped {
            for a in self.mem.remove(&cell).unwrap_or_default() {
                // Its last checkpoint is in a cell still served: go back to it.
                if let (Some(e), Some(snap)) = (self.expect.get(&a.id), self.acked.get(&a.id))
                    && e.cell != cell
                    && !holes.contains(&e.cell)
                {
                    self.mem.entry(e.cell).or_default().push(*snap);
                    self.rewind("agent", snap.tick, seen, seen);
                    fell_back += 1;
                }
            }
            self.holder.remove(&cell);
        }
        self.holder = read.served.clone();
        if agents_back + fell_back > 0 {
            self.counts.rewound += agents_back + fell_back;
            notes.push(format!(
                "{} agents came back older than their last checkpoint ({fell_back} fell back to a checkpoint in another cell)",
                agents_back + fell_back
            ));
        }
        if trails_back > 0 {
            self.counts.trails_rewound += trails_back;
            notes.push(format!(
                "{trails_back} cells' trail came back older than its last checkpoint"
            ));
        }

        // 3. Resolve: one copy per agent across everything held.
        let copies: Vec<(Cell, Agent)> = self
            .mem
            .iter()
            .flat_map(|(c, v)| v.iter().map(move |a| (*c, *a)))
            .collect();
        let resolved = resolve(copies);
        let n_ghosts = resolved.ghost_count();
        if n_ghosts > 0 {
            self.counts.ghosts += n_ghosts as u64;
            for (cell, ds) in &resolved.ghosts {
                self.ghosts
                    .entry(*cell)
                    .or_default()
                    .extend(ds.iter().copied());
            }
        }

        // 4. Count, against each agent's last checkpoint.
        for (id, (cell, a)) in &resolved.alive {
            if !self.expect.contains_key(id)
                && self.file_ids.get(cell).is_some_and(|f| f.contains(id))
            {
                // Read from a file, and nobody expected it.
                self.expect.insert(
                    *id,
                    Expect {
                        cell: *cell,
                        tick: a.tick,
                        at_ms: seen,
                    },
                );
                self.acked.insert(*id, *a);
                self.counts.found += 1;
            }
        }
        let statuses = classify(&self.expect, &resolved.alive, &holes);
        let lost_now: Vec<AgentId> = statuses
            .iter()
            .filter(|(_, s)| **s == Status::Lost)
            .map(|(id, _)| *id)
            .collect();
        for id in &lost_now {
            if let Some(e) = self.expect.remove(id) {
                self.lost.push_back(LostRec {
                    id: id.0,
                    acked_ms: e.at_ms,
                    found_ms: seen,
                });
            }
            self.acked.remove(id);
            if let Some(a) = self.last_seen.remove(id) {
                self.marks.push_back(Mark {
                    kind: "lost",
                    x: a.x,
                    y: a.y,
                    at_ms: seen,
                });
            }
        }
        if !lost_now.is_empty() {
            self.counts.lost += lost_now.len() as u64;
            notes.push(format!(
                "{} agents lost: in no cell, and the dish has no holes",
                lost_now.len()
            ));
        }
        for id in resolved.alive.keys() {
            self.next_id = self.next_id.max(id.0 + 1);
        }

        // 5. Step, or inoculate an empty dish once every cell is served.
        let visible: Vec<Agent> = resolved.alive.values().map(|(_, a)| *a).collect();
        if !visible.is_empty() || !self.expect.is_empty() {
            self.seeded = true;
        }
        let next = if !self.seeded && holes.is_empty() {
            self.seeded = true;
            notes.push(format!("inoculated the dish with {SEED} agents"));
            seed(SEED)
                .into_iter()
                .map(|a| Agent { tick: t, ..a })
                .collect()
        } else {
            step(&visible, &mut self.trail, &holes, &self.foods, t, STEPS)
        };
        self.mem = read.served.keys().map(|c| (*c, Vec::new())).collect();
        for a in &next {
            self.mem.entry(a.cell()).or_default().push(*a);
            self.last_seen.insert(a.id, *a);
        }

        // 6. Save a staggered slice of the served cells, and any cell holding
        //    an agent whose last checkpoint is more than two cycles old (an
        //    agent can cross cell edges in step with the stagger and dodge it).
        let mut due: BTreeSet<Cell> = read
            .served
            .keys()
            .filter(|c| u64::from(c.index()) % CHECKPOINT == t % CHECKPOINT)
            .copied()
            .collect();
        for a in &next {
            let age = self
                .expect
                .get(&a.id)
                .map_or(u64::MAX, |e| t.saturating_sub(e.tick));
            if age > 2 * CHECKPOINT {
                due.insert(a.cell());
            }
        }
        let mut by_cell: BTreeMap<Cell, CellWrite> = BTreeMap::new();
        for cell in &due {
            let here: Vec<Agent> = self.mem.get(cell).cloned().unwrap_or_default();
            let ids: BTreeSet<AgentId> = here.iter().map(|a| a.id).collect();
            let mut dels = self.ghosts.get(cell).cloned().unwrap_or_default();
            // Copies whose newer checkpoint is acknowledged in another cell.
            let file = self.file_ids.get(cell).cloned().unwrap_or_default();
            dels.extend(superseded(*cell, &file, &ids, &self.expect));
            by_cell.insert(
                *cell,
                CellWrite {
                    cell: cell.key(),
                    upserts: here,
                    deletes: dels,
                    trail: Some(TrailWrite {
                        tick: t,
                        hex: to_hex(&self.trail.cell_bytes(*cell)),
                    }),
                },
            );
        }
        let sent: BTreeMap<Cell, CellWrite> = by_cell.clone();
        let acked = self.write(client, by_cell, &read.served, &mut notes).await;
        let at = now();
        for cell in &acked {
            let Some(w) = sent.get(cell) else { continue };
            self.trail_expect.insert(*cell, (t, at));
            let file = self.file_ids.entry(*cell).or_default();
            for d in &w.deletes {
                file.remove(&d.id);
            }
            for a in &w.upserts {
                file.insert(a.id);
                if !self.expect.contains_key(&a.id) {
                    self.counts.released += 1;
                }
                self.expect.insert(
                    a.id,
                    Expect {
                        cell: *cell,
                        tick: a.tick,
                        at_ms: at,
                    },
                );
                self.acked.insert(a.id, *a);
            }
            self.ghosts.remove(cell);
        }

        // The frame.
        let units = |v: f32| -> u16 {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to the dish"
            )]
            let u = v.floor().clamp(0.0, 959.0) as u16;
            u
        };
        let mut agents: Vec<u16> = Vec::with_capacity(next.len() * 3);
        for a in &next {
            agents.extend([units(a.x), units(a.y), 0]);
        }
        let moving = next.len();
        for (id, st) in &statuses {
            if matches!(st, Status::InHole | Status::Missing)
                && let Some(a) = self.last_seen.get(id)
            {
                agents.extend([units(a.x), units(a.y), 1]);
            }
        }
        let frozen = agents.len() / 3 - moving;
        let mut cells = vec![-1i8; usize::from(CELLS)];
        for (cell, (i, _)) in &read.served {
            cells[usize::from(cell.index())] = i8::try_from(*i).unwrap_or(-1);
        }
        let cutoff = at.saturating_sub(MARK_MS);
        while self.marks.front().is_some_and(|m| m.at_ms < cutoff) {
            self.marks.pop_front();
        }
        while self.lost.len() > 400 {
            self.lost.pop_front();
        }
        while self.rewinds.len() > 600 {
            self.rewinds.pop_front();
        }
        self.tick_ms.insert(t, at);
        while self.tick_ms.len() > 4_000 {
            self.tick_ms.pop_first();
        }
        self.counts.ticks += 1;
        let frame = Frame {
            tick: t,
            tick_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            trail: to_hex(&self.trail.bytes(&holes)),
            agents,
            cells,
            foods: self.foods.clone(),
            marks: self.marks.iter().cloned().collect(),
            population: self.expect.len(),
            moving,
            frozen,
            holes: holes.len(),
            oldest_checkpoint: next
                .iter()
                .filter_map(|a| self.expect.get(&a.id).map(|e| t.saturating_sub(e.tick)))
                .max()
                .unwrap_or(0),
            counts: self.counts,
            lost: self.lost.iter().copied().collect(),
            rewinds: self.rewinds.iter().copied().collect(),
        };
        (frame, notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn food_is_capped_and_wrapped() {
        let mut c = Culture::new(mold::lab());
        assert_eq!(c.foods.len(), default_foods().len());
        for i in 0..40 {
            c.add_food(1_000.0 + f32::from(u8::try_from(i).unwrap()), -5.0);
        }
        assert_eq!(c.foods.len(), MAX_FOOD);
        assert!(
            c.foods
                .iter()
                .all(|(x, y)| (0.0..WIDTH).contains(x) && (0.0..HEIGHT).contains(y))
        );
        c.add_food(f32::NAN, 1.0);
        assert_eq!(c.foods.len(), MAX_FOOD);
        c.clear_food();
        assert!(c.foods.is_empty());
    }

    #[test]
    fn a_host_that_timed_out_is_probed_briefly_until_it_answers() {
        let mut c = Culture::new(mold::lab());
        assert_eq!(c.timeout(0), PATIENT);
        c.slow[0] = true;
        assert_eq!(c.timeout(0), PROBE);
    }

    #[tokio::test]
    async fn with_no_hosts_every_cell_is_a_hole_and_nothing_is_seeded() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let hosts = mold::lab()
            .into_iter()
            .enumerate()
            .map(|(i, h)| HostSpec {
                port: 1 + u16::try_from(i).unwrap(),
                ..h
            })
            .collect();
        let mut c = Culture::new(hosts);
        let (frame, _) = c.tick(&client, || 0).await;
        assert_eq!(frame.holes, usize::from(CELLS));
        assert!(frame.agents.is_empty());
        assert_eq!(frame.population, 0);
        assert_eq!(frame.trail.len(), 2 * 192 * 128);
    }
}
