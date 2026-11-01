//! What the conductor and the hosts send each other.
//!
//! A tick is one read of every cell and up to two batches of writes, per host.

use mold::Agent;
use mold::moves::Delete;
use serde::{Deserialize, Serialize};

/// Read these cells.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadReq {
    /// Tenant ids (`c-CC-RR`).
    pub cells: Vec<String>,
}

/// One cell, as a host serves it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellRows {
    /// Tenant id.
    pub cell: String,
    /// The ownership epoch it is open under.
    pub epoch: u64,
    /// Its agents.
    pub agents: Vec<Agent>,
    /// Its trail: 64 bytes as 128 lowercase hex digits (empty before the
    /// first write).
    pub trail: String,
    /// The tick that wrote the trail (0 before the first write).
    #[serde(default)]
    pub trail_tick: u64,
}

/// A host's answer to a read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResp {
    /// The host.
    pub host: String,
    /// The cells it serves.
    pub cells: Vec<CellRows>,
    /// How many it does not own (`421` for each, inside the batch).
    pub refused: u32,
    /// Cells it owns but could not read, with why.
    pub failed: Vec<String>,
}

/// One cell's writes, applied in one transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellWrite {
    /// Tenant id.
    pub cell: String,
    /// Rows to insert or update (never over a newer row).
    #[serde(default)]
    pub upserts: Vec<Agent>,
    /// Rows to delete (only rows older than each `below`).
    #[serde(default)]
    pub deletes: Vec<Delete>,
    /// The cell's new trail, never over a newer one.
    #[serde(default)]
    pub trail: Option<TrailWrite>,
}

/// A cell's trail, as written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrailWrite {
    /// The tick that wrote it.
    pub tick: u64,
    /// 64 bytes as 128 lowercase hex digits.
    pub hex: String,
}

/// Bytes as lowercase hex.
#[must_use]
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Lowercase or uppercase hex to bytes; `None` for anything else.
#[must_use]
pub fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Write these cells.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteReq {
    /// The cells.
    pub cells: Vec<CellWrite>,
}

/// A cell written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Done {
    /// Tenant id.
    pub cell: String,
    /// The epoch it was written under.
    pub epoch: u64,
}

/// A host's answer to a write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteResp {
    /// The host.
    pub host: String,
    /// Cells committed.
    pub done: Vec<Done>,
    /// Cells it does not own.
    pub refused: Vec<String>,
    /// Cells it owns but could not write.
    pub failed: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_refuses_junk() {
        let b: Vec<u8> = (0..=255).collect();
        assert_eq!(from_hex(&to_hex(&b)), Some(b));
        assert_eq!(to_hex(&[0, 15, 255]), "000fff");
        assert_eq!(from_hex("0g"), None);
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("é1"), None);
    }
}
