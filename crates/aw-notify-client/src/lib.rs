//! Lightweight client for aw-notify's local notification HTTP API.
//!
//! aw-notify runs a small HTTP server (default `127.0.0.1:5667`) that accepts
//! `POST /notify` requests carrying a JSON [`NotificationRequest`]. This crate
//! lets other ActivityWatch modules (watchers, importers, scripts) push a
//! desktop notification through aw-notify without re-implementing the wire
//! format.
//!
//! ```no_run
//! use aw_notify_client::NotificationClient;
//!
//! let client = NotificationClient::new().with_sender("aw-watcher-foo");
//! // Ignoring the result here, but you can match on `NotifyError` to back off.
//! let _ = client.notify("Backup done", "Synced 1,234 events");
//! ```

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Default host aw-notify's HTTP server binds to.
pub const DEFAULT_HOST: &str = "127.0.0.1";
/// Default port aw-notify's HTTP server listens on.
pub const DEFAULT_PORT: u16 = 5667;

/// Notification payload sent to (and accepted by) aw-notify's `/notify` endpoint.
///
/// `title` and `message` are required; `sender` identifies the originating
/// module and is surfaced in the displayed notification (e.g. `Title (sender)`).
/// On the wire the field is `sender`, but incoming requests may also use the
/// alias `watcher`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRequest {
    pub title: String,
    pub message: String,
    #[serde(alias = "watcher", skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
}

/// Error returned when a notification could not be delivered.
///
/// Deliberately does not expose the underlying HTTP library types so the public
/// API stays stable across dependency bumps.
#[derive(Debug)]
pub enum NotifyError {
    /// The request never reached aw-notify (it isn't running, wrong port,
    /// connection refused/timed out, …). Contains a human-readable description.
    Transport(String),
    /// aw-notify received the request but rejected it with a non-2xx status.
    ///
    /// Notable codes: `400` invalid/oversized JSON, `429` queue full (back off
    /// and retry later), `503` service not ready, `404` wrong path.
    Rejected(u16),
}

impl fmt::Display for NotifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotifyError::Transport(msg) => {
                write!(f, "could not reach aw-notify: {msg}")
            }
            NotifyError::Rejected(code) => {
                write!(f, "aw-notify rejected the notification (HTTP {code})")
            }
        }
    }
}

impl std::error::Error for NotifyError {}

/// A reusable client targeting one aw-notify instance.
///
/// Cheap to construct and clone-free to reuse; the underlying connection agent
/// is shared across calls.
pub struct NotificationClient {
    base_url: String,
    sender: Option<String>,
    agent: ureq::Agent,
}

impl NotificationClient {
    /// Client targeting the default local aw-notify (`127.0.0.1:5667`).
    pub fn new() -> Self {
        Self::with_addr(DEFAULT_HOST, DEFAULT_PORT)
    }

    /// Client targeting a specific host and port.
    pub fn with_addr(host: &str, port: u16) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(2))
            .timeout_read(Duration::from_secs(3))
            .timeout_write(Duration::from_secs(3))
            .build();
        Self {
            base_url: format!("http://{host}:{port}"),
            sender: None,
            agent,
        }
    }

    /// Set the module name reported with every notification from this client.
    pub fn with_sender(mut self, sender: impl Into<String>) -> Self {
        self.sender = Some(sender.into());
        self
    }

    /// Send a notification. Returns once aw-notify has accepted (or rejected) it.
    pub fn notify(&self, title: &str, message: &str) -> Result<(), NotifyError> {
        let req = NotificationRequest {
            title: title.to_string(),
            message: message.to_string(),
            sender: self.sender.clone(),
        };
        // Serialize with serde_json so ureq's optional `json` feature isn't needed.
        let body = serde_json::to_string(&req)
            .map_err(|e| NotifyError::Transport(format!("failed to serialize request: {e}")))?;

        let url = format!("{}/notify", self.base_url);
        match self
            .agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body)
        {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, _)) => Err(NotifyError::Rejected(code)),
            Err(ureq::Error::Transport(t)) => Err(NotifyError::Transport(t.to_string())),
        }
    }
}

impl Default for NotificationClient {
    fn default() -> Self {
        Self::new()
    }
}

/// One-shot convenience: send a single notification to the default local
/// aw-notify. For repeated sends, construct a [`NotificationClient`] and reuse it.
pub fn send(title: &str, message: &str, sender: Option<&str>) -> Result<(), NotifyError> {
    let mut client = NotificationClient::new();
    if let Some(s) = sender {
        client = client.with_sender(s);
    }
    client.notify(title, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_sender_and_omits_none() {
        let with = NotificationRequest {
            title: "t".into(),
            message: "m".into(),
            sender: Some("watcher-x".into()),
        };
        assert_eq!(
            serde_json::to_string(&with).unwrap(),
            r#"{"title":"t","message":"m","sender":"watcher-x"}"#
        );

        let without = NotificationRequest {
            title: "t".into(),
            message: "m".into(),
            sender: None,
        };
        // `sender` is skipped entirely when absent.
        assert_eq!(
            serde_json::to_string(&without).unwrap(),
            r#"{"title":"t","message":"m"}"#
        );
    }

    #[test]
    fn deserializes_both_sender_and_watcher_alias() {
        // This is the exact type aw-notify deserializes incoming requests into,
        // so both spellings must map to `sender`.
        let from_sender: NotificationRequest =
            serde_json::from_str(r#"{"title":"t","message":"m","sender":"a"}"#).unwrap();
        assert_eq!(from_sender.sender.as_deref(), Some("a"));

        let from_watcher: NotificationRequest =
            serde_json::from_str(r#"{"title":"t","message":"m","watcher":"a"}"#).unwrap();
        assert_eq!(from_watcher.sender.as_deref(), Some("a"));

        let omitted: NotificationRequest =
            serde_json::from_str(r#"{"title":"t","message":"m"}"#).unwrap();
        assert_eq!(omitted.sender, None);
    }
}
