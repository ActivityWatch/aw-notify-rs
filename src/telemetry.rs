//! Process-lifetime delivery counters and watcher-style liveness.
use anyhow::Result;
use aw_client_rust::blocking::AwClient;
use aw_models::Event;
use chrono::{DateTime, Duration, Utc};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::thread;
use std::time;

pub const INTERVAL: time::Duration = time::Duration::from_secs(5);
const PULSETIME: f64 = 10.0;

#[derive(Clone, Copy, Debug)]
pub enum AlertType {
    Threshold,
    Checkin,
    ProductivityScore,
    NewDay,
    ServerStatus,
    External,
}

impl AlertType {
    fn key(self) -> &'static str {
        match self {
            Self::Threshold => "threshold",
            Self::Checkin => "checkin",
            Self::ProductivityScore => "productivity_score",
            Self::NewDay => "new_day",
            Self::ServerStatus => "server_status",
            Self::External => "external",
        }
    }
}

#[derive(Default, Serialize)]
struct Counts {
    shown: u64,
    forwarded: u64,
    // No backend currently observes user dismissal. Unknown must not become zero.
    dismissed: Option<u64>,
}

pub struct Telemetry {
    session_started: DateTime<Utc>,
    counts: Mutex<BTreeMap<&'static str, Counts>>,
}

impl Telemetry {
    pub fn new() -> Self {
        Self {
            session_started: Utc::now(),
            counts: Mutex::new(
                [
                    AlertType::Threshold,
                    AlertType::Checkin,
                    AlertType::ProductivityScore,
                    AlertType::NewDay,
                    AlertType::ServerStatus,
                    AlertType::External,
                ]
                .into_iter()
                .map(|kind| (kind.key(), Counts::default()))
                .collect(),
            ),
        }
    }

    pub fn record(&self, kind: AlertType, output_only: bool, count: u64) {
        let mut counts = self.counts.lock().unwrap();
        let counters = counts.get_mut(kind.key()).unwrap();
        if output_only {
            counters.forwarded += count;
        } else {
            counters.shown += count;
        }
    }

    /// Count only successful delivery, never enqueue attempts or failed output.
    pub fn deliver(
        &self,
        enabled: bool,
        output_only: bool,
        kind: AlertType,
        send: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        if !enabled {
            return Ok(());
        }
        send()?;
        self.record(kind, output_only, 1);
        Ok(())
    }

    fn event(&self, output_only: bool) -> Event {
        let data = serde_json::json!({
            "schema_version": 1,
            "enabled": true,
            "session_started": self.session_started,
            "output_only": output_only,
            "counts": *self.counts.lock().unwrap(),
        });
        Event::new(
            Utc::now(),
            Duration::zero(),
            data.as_object().unwrap().clone(),
        )
    }
}

fn bucket_id(hostname: &str) -> String {
    format!("aw-notify_{hostname}")
}

// The pinned aw-client does not surface HTTP error statuses for writes. Bucket
// creation is idempotent, so ensure it exists on every pulse (also after deletion
// or a server restart) rather than caching an unverified creation response.
fn send_heartbeat(client: &AwClient, bucket: &str, event: &Event) -> Result<()> {
    client.create_bucket_simple(bucket, "app.aw-notify.status")?;
    client.heartbeat(bucket, event, PULSETIME)?;
    Ok(())
}

pub fn start(
    client: &'static AwClient,
    hostname: &str,
    telemetry: &'static Telemetry,
    output_only: bool,
    shutdown: Receiver<()>,
) {
    let bucket = bucket_id(hostname);
    thread::spawn(move || {
        loop {
            // Disconnection is also shutdown; never emit after the owner exits.
            if !matches!(
                shutdown.try_recv(),
                Err(crossbeam_channel::TryRecvError::Empty)
            ) {
                break;
            }
            if let Err(error) = send_heartbeat(client, &bucket, &telemetry.event(output_only)) {
                log::warn!("Failed to send liveness heartbeat: {}", error);
            }
            if !matches!(
                shutdown.recv_timeout(INTERVAL),
                Err(RecvTimeoutError::Timeout)
            ) {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_delivery_counts_by_type_without_inventing_dismissals() {
        let stats = Telemetry::new();
        stats
            .deliver(true, false, AlertType::Threshold, || Ok(()))
            .unwrap();
        stats
            .deliver(true, true, AlertType::External, || Ok(()))
            .unwrap();
        assert!(stats
            .deliver(true, false, AlertType::Threshold, || anyhow::bail!(
                "backend failed"
            ))
            .is_err());
        stats
            .deliver(false, false, AlertType::Checkin, || {
                panic!("disabled must not deliver")
            })
            .unwrap();
        let event = stats.event(false);
        assert_eq!(event.data["counts"]["threshold"]["shown"], 1);
        assert_eq!(event.data["counts"]["external"]["shown"], 0);
        assert_eq!(event.data["counts"]["external"]["forwarded"], 1);
        assert_eq!(event.data["counts"]["checkin"]["shown"], 0);
        for counts in event.data["counts"].as_object().unwrap().values() {
            assert!(counts["dismissed"].is_null());
        }
        assert_eq!(
            event.data["session_started"],
            stats.event(false).data["session_started"]
        );
        assert_eq!(event.duration, Duration::zero());
    }

    #[test]
    fn watcher_contract() {
        assert_eq!(bucket_id("my-host"), "aw-notify_my-host");
        assert_eq!(INTERVAL, time::Duration::from_secs(5));
        assert_eq!(PULSETIME, 2.0 * INTERVAL.as_secs_f64());
    }
}
