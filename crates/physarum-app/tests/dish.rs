//! The lab's claims, against real host processes.
//!
//! Three `physarum-app host` processes on spare ports (4301–4303), one
//! shared control database and replica, driven by the console's own
//! supervisor and conductor. One test on purpose: the steps are a sequence.
//!
//! What it holds the framework and the conductor to:
//!   1. the primaries split the 384 cells exactly as the slot of each cell's
//!      tenant id says, and every agent of the mold reaches a checkpoint;
//!   2. after `kill -9` of host-a, exactly host-a's cells are holes, agents
//!      whose last checkpoint is in them freeze, and no moving agent is ever
//!      in a hole;
//!   3. the standby takes every hole back from the replica;
//!   4. fail-back to host-a and a thawed zombie host-c: the zombie serves
//!      nothing;
//!   5. throughout: no stale epoch, no cell served twice, and
//!      `population == released + found - lost`.

use std::path::Path;
use std::time::{Duration, Instant};

use mold::{Cell, HostSpec, SlotRange};
use physarum_app::console::conductor::{CHECKPOINT, Culture, Frame, SEED};
use physarum_app::console::supervisor::HostProc;
use physarum_app::wire::{ReadReq, ReadResp};

/// Generous: a primary claims and creates its cells in seconds.
const CLAIM: Duration = Duration::from_mins(1);
/// A standby restoring every cell from the replica on a loaded machine.
const TAKEOVER: Duration = Duration::from_mins(2);
const SECRET: &str = "murmuration-test-secret-not-for-production-use-0123456789";

fn spec(name: &'static str, a: u16, b: u16, port: u16) -> HostSpec {
    HostSpec {
        name,
        slots: SlotRange::new(a, b).unwrap(),
        port,
        role: "test",
    }
}

struct Lab {
    hosts: Vec<HostProc>,
    root: std::path::PathBuf,
    client: reqwest::Client,
    culture: Culture,
    started: Instant,
}

impl Lab {
    fn specs() -> Vec<HostSpec> {
        vec![
            spec("host-a", 0, 8_191, 4301),
            spec("host-b", 8_192, 16_383, 4302),
            spec("host-c", 0, 8_191, 4303),
        ]
    }

    fn new(root: &Path) -> Self {
        Self {
            hosts: Self::specs().into_iter().map(HostProc::new).collect(),
            root: root.to_path_buf(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            culture: Culture::new(Self::specs()),
            started: Instant::now(),
        }
    }

    fn start(&mut self, i: usize) {
        let exe = Path::new(env!("CARGO_BIN_EXE_physarum-app"));
        let root = self.root.clone();
        self.hosts[i].start(exe, &root, SECRET).unwrap();
    }

    async fn tick(&mut self) -> Frame {
        let t0 = self.started;
        let now = move || u64::try_from(t0.elapsed().as_millis()).unwrap();
        let (f, _) = self.culture.tick(&self.client, now).await;
        check(&f);
        f
    }

    /// Tick until `done`, or panic after `within`.
    async fn until(
        &mut self,
        within: Duration,
        what: &str,
        done: impl Fn(&Frame) -> bool,
    ) -> Frame {
        let t = Instant::now();
        loop {
            let f = self.tick().await;
            if done(&f) {
                return f;
            }
            assert!(
                t.elapsed() < within,
                "never: {what} (holes {}, population {})",
                f.holes,
                f.population
            );
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    }

    async fn served_by(&self, i: usize) -> usize {
        let req = ReadReq {
            cells: Cell::all().map(Cell::key).collect(),
        };
        let r: ReadResp = self
            .client
            .post(format!(
                "http://127.0.0.1:{}/api/cells/read",
                self.hosts[i].spec.port
            ))
            .json(&req)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        r.cells.len()
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for h in &mut self.hosts {
            let _ = h.kill();
        }
    }
}

/// The invariants every frame keeps (5).
fn check(f: &Frame) {
    let c = f.counts;
    assert_eq!(c.stale, 0, "an answer below the epoch watermark");
    assert_eq!(c.double, 0, "a cell served by two hosts in one tick");
    assert_eq!(
        f.population as u64,
        c.released + c.found - c.lost,
        "population does not add up: {c:?}"
    );
    // Once inoculated and with no holes, no moving agent's checkpoint is
    // more than about two cycles old.
    if f.holes == 0 && f.population as u64 == SEED {
        assert!(
            f.oldest_checkpoint <= 2 * CHECKPOINT + 1,
            "a checkpoint {} ticks old",
            f.oldest_checkpoint
        );
    }
    // Moving agents are never in a hole.
    for a in f.agents.chunks(3).filter(|a| a[2] == 0) {
        let cell = mold::geom::cell_of(f32::from(a[0]), f32::from(a[1]));
        assert!(
            f.cells[usize::from(cell.index())] >= 0,
            "an agent moves in hole {cell}"
        );
    }
}

fn owner_of(cell: Cell) -> i8 {
    let slot = autumn_web::sharding::slot_for_key(cell.key().as_str().into()).0;
    i8::from(slot >= 8_192)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(
    clippy::too_many_lines,
    reason = "one drill, each step depends on the last"
)]
async fn holes_open_and_close_and_the_count_adds_up() {
    let tmp = tempfile::tempdir().unwrap();
    let mut lab = Lab::new(tmp.path());
    lab.start(0);
    lab.start(1);

    // 1. The split, and every agent checkpointed.
    let f = lab
        .until(CLAIM, "the primaries serve every cell", |f| f.holes == 0)
        .await;
    for cell in Cell::all() {
        assert_eq!(f.cells[usize::from(cell.index())], owner_of(cell), "{cell}");
    }
    let a_cells = Cell::all().filter(|c| owner_of(*c) == 0).count();
    let f = lab
        .until(CLAIM, "every agent reaches a checkpoint", |f| {
            f.population as u64 == SEED
        })
        .await;
    assert_eq!(f.counts.released, SEED);
    for _ in 0..30 {
        lab.tick().await;
    }

    // 2. kill -9 host-a: exactly its cells go dark.
    lab.hosts[0].kill().unwrap();
    let f = lab
        .until(Duration::from_secs(30), "host-a's cells are holes", |f| {
            f.holes > 0
        })
        .await;
    assert_eq!(f.holes, a_cells);
    for cell in Cell::all() {
        assert_eq!(
            f.cells[usize::from(cell.index())] == -1,
            owner_of(cell) == 0,
            "{cell}"
        );
    }
    assert!(f.frozen > 0, "no agent's checkpoint was in host-a's cells");
    for _ in 0..12 {
        lab.tick().await;
    }

    // 3. The standby fills every hole from the replica.
    lab.start(2);
    let f = lab
        .until(TAKEOVER, "host-c takes the holes back", |f| f.holes == 0)
        .await;
    assert_eq!(f.cells.iter().filter(|o| **o == 2).count(), a_cells);
    assert_eq!(f.frozen, 0);
    eprintln!(
        "takeover: {} lost, {} agents and {} cells' trail came back older (up to {} ms), {} ghosts",
        f.counts.lost,
        f.counts.rewound,
        f.counts.trails_rewound,
        f.rewinds.iter().map(|r| r.by_ms).max().unwrap_or(0),
        f.counts.ghosts
    );
    for _ in 0..24 {
        lab.tick().await;
    }

    // 4. Freeze the standby, fail back to host-a, thaw the zombie.
    lab.hosts[2].freeze().unwrap();
    tokio::time::sleep(Duration::from_secs(8)).await;
    lab.start(0);
    let f = lab
        .until(TAKEOVER, "host-a takes its cells back", |f| {
            f.holes == 0 && f.cells.iter().filter(|o| **o == 0).count() == a_cells
        })
        .await;
    assert_eq!(f.frozen, 0);
    lab.hosts[2].thaw().unwrap();
    for _ in 0..24 {
        lab.tick().await;
    }
    assert_eq!(
        lab.served_by(2).await,
        0,
        "the thawed zombie still serves cells"
    );
    let f = lab.tick().await;
    assert_eq!(f.cells.iter().filter(|o| **o == 2).count(), 0);
}
