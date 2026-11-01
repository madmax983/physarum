//! A host: a tenant fleet, one tenant per cell of the sky, with ownership
//! and replication; and two batch endpoints the conductor calls every tick.
//!
//! Every cell goes through `DatabaseFleet::open`, which checks that this host
//! owns the cell's slot at its current epoch before it touches the disk. A
//! cell this host does not own is refused inside the batch (the batch's own
//! `421`), so the conductor can ask every host for every cell.

use autumn_web::AppState;
use autumn_web::db::fleet::{DatabaseFleet, FleetDatabase, FleetError};
use autumn_web::prelude::*;
use autumn_web::reexports::axum::response::Response;
use autumn_web::reexports::{diesel, diesel_async};
use diesel_async::{RunQueryDsl as _, SimpleAsyncConnection as _};
use mold::moves::Delete;
use mold::{Agent, AgentId, Cell};

use crate::wire::{
    CellRows, CellWrite, Done, ReadReq, ReadResp, TrailWrite, WriteReq, WriteResp, from_hex, to_hex,
};

/// Cells one request works on at once.
///
/// A standby taking over a range restores every cell from the replica on its
/// first read; unbounded, that is a few hundred restores at once, enough to
/// starve the other hosts.
pub const PARALLEL: usize = 16;

fn owner(state: &AppState) -> String {
    state
        .config()
        .database
        .fleet
        .as_ref()
        .and_then(|f| f.ownership.as_ref())
        .map_or_else(|| "unowned".to_owned(), |o| o.owner.clone())
}

fn fleet(state: &AppState) -> Option<DatabaseFleet> {
    state.shards().and_then(|s| s.fleet()).cloned()
}

fn epoch(db: &FleetDatabase) -> u64 {
    db.epoch().map_or(0, autumn_web::lock::FencingToken::get)
}

/// Why one cell could not be served.
enum Miss {
    /// Not this host's slot.
    NotOwner,
    /// Anything else, with why.
    Failed(String),
}

async fn open(fleet: &DatabaseFleet, cell: &str) -> Result<FleetDatabase, Miss> {
    if Cell::from_key(cell).is_none() {
        return Err(Miss::Failed(format!("{cell}: not a cell")));
    }
    let key = fleet
        .key_for(cell)
        .map_err(|e| Miss::Failed(format!("{cell}: {e}")))?;
    match fleet.open(&key).await {
        Ok(db) => Ok(db),
        Err(FleetError::NotOwner { .. }) => Err(Miss::NotOwner),
        Err(e) => Err(Miss::Failed(format!("{cell}: {e}"))),
    }
}

#[derive(diesel::QueryableByName)]
struct Row {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    x: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    y: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    vx: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    vy: f64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    tick: i64,
}

#[derive(diesel::QueryableByName)]
struct Blob {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    tick: i64,
    #[diesel(sql_type = diesel::sql_types::Binary)]
    data: Vec<u8>,
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the rows were written from f32"
)]
fn to_agent(r: &Row) -> Option<Agent> {
    Some(Agent {
        id: AgentId(u64::try_from(r.id).ok()?),
        x: r.x as f32,
        y: r.y as f32,
        vx: r.vx as f32,
        vy: r.vy as f32,
        tick: u64::try_from(r.tick).ok()?,
    })
}

async fn read_cell(fleet: &DatabaseFleet, cell: &str) -> Result<CellRows, Miss> {
    let db = open(fleet, cell).await?;
    let _lease = db.lease();
    let mut conn = db
        .pool()
        .get()
        .await
        .map_err(|e| Miss::Failed(format!("{cell}: {e}")))?;
    let rows: Vec<Row> = diesel::sql_query("SELECT id, x, y, vx, vy, tick FROM agents")
        .load(&mut *conn)
        .await
        .map_err(|e| Miss::Failed(format!("{cell}: {e}")))?;
    let trail: Vec<Blob> = diesel::sql_query("SELECT tick, data FROM trail WHERE id = 1")
        .load(&mut *conn)
        .await
        .map_err(|e| Miss::Failed(format!("{cell}: {e}")))?;
    // Spelled out: a bare `.first()` here resolves to diesel's `FirstDsl`.
    let row = <[Blob]>::first(&trail);
    Ok(CellRows {
        cell: cell.to_owned(),
        epoch: epoch(&db),
        agents: rows.iter().filter_map(to_agent).collect(),
        trail: row.map(|b| to_hex(&b.data)).unwrap_or_default(),
        trail_tick: row.and_then(|b| u64::try_from(b.tick).ok()).unwrap_or(0),
    })
}

/// Read cells. Answers the ones this host serves; counts the rest.
#[post("/api/cells/read")]
#[public]
pub async fn read(State(state): State<AppState>, Json(req): Json<ReadReq>) -> Response {
    let host = owner(&state);
    let Some(fleet) = fleet(&state) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no fleet").into_response();
    };
    // Each cell in its own task: they run together, and a read that is still
    // restoring a cell from the replica finishes even if the conductor stops
    // waiting (a dropped JoinHandle does not abort its task).
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(PARALLEL));
    let tasks: Vec<_> = req
        .cells
        .into_iter()
        .map(|cell| {
            let (fleet, gate) = (fleet.clone(), std::sync::Arc::clone(&gate));
            tokio::spawn(async move {
                let _permit = gate.acquire_owned().await;
                read_cell(&fleet, &cell).await
            })
        })
        .collect();
    let mut resp = ReadResp {
        host,
        cells: Vec::new(),
        refused: 0,
        failed: Vec::new(),
    };
    for t in tasks {
        match t.await {
            Ok(Ok(rows)) => resp.cells.push(rows),
            Ok(Err(Miss::NotOwner)) => resp.refused += 1,
            Ok(Err(Miss::Failed(why))) => resp.failed.push(why),
            Err(e) => resp.failed.push(e.to_string()),
        }
    }
    Json(resp).into_response()
}

/// The SQL for one cell's writes, as one transaction.
///
/// Every value is a number the server parsed from JSON and checked finite,
/// or hex it decoded to bytes and re-encoded, so formatting it into the
/// statement cannot inject anything.
///
/// # Errors
///
/// A non-finite number, an id or tick past `i64`, or a trail that is not 64
/// bytes of hex.
pub fn cell_sql(
    upserts: &[Agent],
    deletes: &[Delete],
    trail: Option<&TrailWrite>,
) -> Result<String, String> {
    use std::fmt::Write as _;
    let mut sql = String::from("BEGIN IMMEDIATE;\n");
    if !upserts.is_empty() {
        sql.push_str("INSERT INTO agents (id, x, y, vx, vy, tick) VALUES ");
        for (i, b) in upserts.iter().enumerate() {
            if !b.is_finite() {
                return Err(format!("agent {} is not finite", b.id.0));
            }
            let id = i64::try_from(b.id.0).map_err(|e| e.to_string())?;
            let tick = i64::try_from(b.tick).map_err(|e| e.to_string())?;
            if i > 0 {
                sql.push(',');
            }
            let _ = write!(sql, "({id},{},{},{},{},{tick})", b.x, b.y, b.vx, b.vy);
        }
        // Never over a newer row (SPEC 4.4).
        sql.push_str(
            " ON CONFLICT(id) DO UPDATE SET x = excluded.x, y = excluded.y, vx = excluded.vx, \
             vy = excluded.vy, tick = excluded.tick WHERE excluded.tick >= agents.tick;\n",
        );
    }
    for d in deletes {
        let id = i64::try_from(d.id.0).map_err(|e| e.to_string())?;
        let below = i64::try_from(d.below).map_err(|e| e.to_string())?;
        // Only rows older than the tick that moved or superseded it (SPEC 4.4).
        let _ = writeln!(
            sql,
            "DELETE FROM agents WHERE id = {id} AND tick < {below};"
        );
    }
    if let Some(t) = trail {
        let bytes = from_hex(&t.hex)
            .filter(|b| b.len() == mold::trail::CELL_BYTES)
            .ok_or_else(|| "a trail is 64 bytes of hex".to_owned())?;
        let tick = i64::try_from(t.tick).map_err(|e| e.to_string())?;
        // Never over a newer trail.
        let _ = writeln!(
            sql,
            "INSERT INTO trail (id, tick, data) VALUES (1, {tick}, X'{}') ON CONFLICT(id) DO UPDATE SET \
             tick = excluded.tick, data = excluded.data WHERE excluded.tick >= trail.tick;",
            to_hex(&bytes)
        );
    }
    sql.push_str("COMMIT;");
    Ok(sql)
}

async fn write_cell(fleet: &DatabaseFleet, w: &CellWrite) -> Result<Done, Miss> {
    let sql = cell_sql(&w.upserts, &w.deletes, w.trail.as_ref()).map_err(Miss::Failed)?;
    let db = open(fleet, &w.cell).await?;
    let _lease = db.lease();
    let mut conn = db
        .pool()
        .get()
        .await
        .map_err(|e| Miss::Failed(format!("{}: {e}", w.cell)))?;
    if let Err(e) = conn.batch_execute(&sql).await {
        let _ = conn.batch_execute("ROLLBACK;").await;
        return Err(Miss::Failed(format!("{}: {e}", w.cell)));
    }
    Ok(Done {
        cell: w.cell.clone(),
        epoch: epoch(&db),
    })
}

/// Write cells: each cell's upserts and deletes in one transaction.
#[post("/api/cells/write")]
#[public]
pub async fn write(State(state): State<AppState>, Json(req): Json<WriteReq>) -> Response {
    let host = owner(&state);
    let Some(fleet) = fleet(&state) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no fleet").into_response();
    };
    // As for reads: concurrent, and a write runs to its commit or rollback
    // even if the conductor has stopped waiting for the answer.
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(PARALLEL));
    let tasks: Vec<_> = req
        .cells
        .into_iter()
        .map(|w| {
            let (fleet, gate) = (fleet.clone(), std::sync::Arc::clone(&gate));
            tokio::spawn(async move {
                let _permit = gate.acquire_owned().await;
                let cell = w.cell.clone();
                (cell, write_cell(&fleet, &w).await)
            })
        })
        .collect();
    let mut resp = WriteResp {
        host,
        done: Vec::new(),
        refused: Vec::new(),
        failed: Vec::new(),
    };
    for t in tasks {
        match t.await {
            Ok((_, Ok(d))) => resp.done.push(d),
            Ok((cell, Err(Miss::NotOwner))) => resp.refused.push(cell),
            Ok((_, Err(Miss::Failed(why)))) => resp.failed.push(why),
            Err(e) => resp.failed.push(e.to_string()),
        }
    }
    Json(resp).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(id: u64, tick: u64) -> Agent {
        Agent {
            id: AgentId(id),
            x: 1.5,
            y: 2.0,
            vx: -0.25,
            vy: 3.0,
            tick,
        }
    }

    #[test]
    fn the_sql_is_one_transaction_and_guards_ticks() {
        let sql = cell_sql(
            &[b(1, 4), b(2, 4)],
            &[Delete {
                id: AgentId(9),
                below: 4,
            }],
            None,
        )
        .unwrap();
        assert!(sql.starts_with("BEGIN IMMEDIATE;") && sql.ends_with("COMMIT;"));
        assert!(sql.contains("(1,1.5,2,-0.25,3,4),(2,1.5,2,-0.25,3,4)"));
        assert!(sql.contains("WHERE excluded.tick >= agents.tick"));
        assert!(sql.contains("DELETE FROM agents WHERE id = 9 AND tick < 4;"));
        assert_eq!(
            cell_sql(&[], &[], None).unwrap(),
            "BEGIN IMMEDIATE;\nCOMMIT;"
        );
        let t = TrailWrite {
            tick: 7,
            hex: "ab".repeat(64),
        };
        let sql = cell_sql(&[], &[], Some(&t)).unwrap();
        assert!(sql.contains("VALUES (1, 7, X'abab"));
        assert!(sql.contains("WHERE excluded.tick >= trail.tick"));
        let short = TrailWrite {
            tick: 7,
            hex: "ab".into(),
        };
        assert!(cell_sql(&[], &[], Some(&short)).is_err());
        let junk = TrailWrite {
            tick: 7,
            hex: "zz".repeat(64),
        };
        assert!(cell_sql(&[], &[], Some(&junk)).is_err());
    }

    #[test]
    fn non_finite_and_huge_values_are_refused() {
        let mut nan = b(1, 1);
        nan.x = f32::NAN;
        assert!(cell_sql(&[nan], &[], None).is_err());
        assert!(cell_sql(&[b(u64::MAX, 1)], &[], None).is_err());
        assert!(
            cell_sql(
                &[],
                &[Delete {
                    id: AgentId(1),
                    below: u64::MAX
                }],
                None
            )
            .is_err()
        );
    }
}
