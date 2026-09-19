//! Background capability lease/receipt record for aw-notify.
//!
//! Implements the "Background Capability Lease Protocol" schema (design v1):
//! a tiny, local-first, JSON-serializable contract that distinguishes
//! **enabled**, **running**, and **actually-delivered** for an opt-in
//! background capability. See the schema design in the Bob workspace:
//! `knowledge/technical/background-capability-lease-schema.md`.
//!
//! The lease is a single JSON object written to a well-known local path
//! (`~/.local/state/activitywatch/aw-notify/lease.json` on Linux). Any monitor
//! can read it and test liveness with no scheduler and no server round-trip.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::dirs;

/// Schema version. Bump on a breaking change; readers ignore unknown fields.
pub const SCHEMA_VERSION: u32 = 1;

/// Heartbeat staleness threshold in seconds. A lease whose heartbeat is older
/// than this is `stale` (hung/crashed) rather than `running`.
pub const HEARTBEAT_EXPIRY_SECONDS: i64 = 120;

/// The lease/receipt record for a single background capability.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Lease {
    pub capability: String,
    pub schema_version: u32,
    pub desired_state: String,
    pub effective_state: String,
    pub owner: Owner,
    pub heartbeat: Heartbeat,
    pub delivery_receipt: DeliveryReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Owner {
    pub process: String,
    pub pid: u32,
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Heartbeat {
    pub last: DateTime<Utc>,
    pub expiry_after_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeliveryReceipt {
    pub last_successful: Option<DateTime<Utc>>,
    pub last_payload: Option<serde_json::Value>,
}

impl Lease {
    /// Create a fresh lease for a capability that is enabled and running.
    pub fn new(capability: &str, process: &str, hostname: &str) -> Self {
        let now = Utc::now();
        Lease {
            capability: capability.to_string(),
            schema_version: SCHEMA_VERSION,
            desired_state: "enabled".to_string(),
            effective_state: "running".to_string(),
            owner: Owner {
                process: process.to_string(),
                pid: std::process::id(),
                hostname: hostname.to_string(),
            },
            heartbeat: Heartbeat {
                last: now,
                expiry_after_seconds: HEARTBEAT_EXPIRY_SECONDS,
            },
            delivery_receipt: DeliveryReceipt {
                last_successful: None,
                last_payload: None,
            },
        }
    }

    /// True if the heartbeat is fresh (now - last < expiry).
    pub fn is_running(&self) -> bool {
        let now = Utc::now();
        (now - self.heartbeat.last).num_seconds() < self.heartbeat.expiry_after_seconds
    }

    /// True if the lease is stale (heartbeat expired) — a hung/crashed process.
    pub fn is_stale(&self) -> bool {
        !self.is_running()
    }

    /// Refresh the heartbeat timestamp.
    pub fn touch(&mut self) {
        self.heartbeat.last = Utc::now();
    }

    /// Record a successful delivery. `last_successful` is only ever advanced
    /// by a success — a failed delivery never moves it (the edge case in the
    /// schema: last-successful, not last-attempted).
    pub fn record_delivery(&mut self, payload: serde_json::Value) {
        self.delivery_receipt.last_successful = Some(Utc::now());
        self.delivery_receipt.last_payload = Some(payload);
    }

    /// Mark the capability as cleanly stopped (removes the lease on write).
    pub fn mark_stopped(&mut self) {
        self.effective_state = "stopped".to_string();
    }
}

/// Path to the lease file for this capability.
pub fn lease_path() -> Result<PathBuf> {
    Ok(dirs::get_state_dir()?.join("lease.json"))
}

/// Write the lease to disk (atomic: write temp then rename).
pub fn write_lease(lease: &Lease) -> Result<()> {
    let path = lease_path()?;
    let json = serde_json::to_string_pretty(lease)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Read the lease from disk, if present.
pub fn read_lease() -> Result<Option<Lease>> {
    let path = lease_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let json = fs::read_to_string(&path)?;
    let lease: Lease = serde_json::from_str(&json)
        .map_err(|e| anyhow!("failed to parse lease at {}: {}", path.display(), e))?;
    Ok(Some(lease))
}

/// Remove the lease file (used on clean shutdown when the capability stops).
pub fn remove_lease() -> Result<()> {
    let path = lease_path()?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_lease() -> Lease {
        Lease::new("aw-notify", "aw-notify", "testhost")
    }

    #[test]
    fn fresh_lease_is_running() {
        let lease = test_lease();
        assert!(lease.is_running());
        assert!(!lease.is_stale());
        assert_eq!(lease.desired_state, "enabled");
        assert_eq!(lease.effective_state, "running");
    }

    #[test]
    fn stale_lease_detected() {
        let mut lease = test_lease();
        // Backdate the heartbeat beyond the expiry window.
        lease.heartbeat.last = Utc::now() - chrono::Duration::seconds(HEARTBEAT_EXPIRY_SECONDS + 1);
        assert!(lease.is_stale());
        assert!(!lease.is_running());
    }

    #[test]
    fn touch_refreshes_heartbeat() {
        let mut lease = test_lease();
        lease.heartbeat.last = Utc::now() - chrono::Duration::seconds(HEARTBEAT_EXPIRY_SECONDS + 1);
        assert!(lease.is_stale());
        lease.touch();
        assert!(lease.is_running());
    }

    #[test]
    fn delivery_receipt_is_last_successful() {
        let mut lease = test_lease();
        assert!(lease.delivery_receipt.last_successful.is_none());
        lease.record_delivery(serde_json::json!({"notification_type": "threshold"}));
        assert!(lease.delivery_receipt.last_successful.is_some());
        assert_eq!(
            lease.delivery_receipt.last_payload,
            Some(serde_json::json!({"notification_type": "threshold"}))
        );
    }

    #[test]
    fn roundtrip_serialization() {
        let lease = test_lease();
        let json = serde_json::to_string(&lease).unwrap();
        let parsed: Lease = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, lease);
    }
}
