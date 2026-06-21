//! Minimal example of sending a desktop notification through aw-notify.
//!
//! Start aw-notify first (`cargo run -- start`), then in another terminal:
//!
//! ```sh
//! cargo run --example notify_client
//! cargo run --example notify_client "Build finished" "All tests passed ✅"
//! ```

use aw_notify_client::{NotificationClient, NotifyError};

fn main() {
    // Optional title/message from the command line, with sample fallbacks.
    let mut args = std::env::args().skip(1);
    let title = args
        .next()
        .unwrap_or_else(|| "Hello from a watcher".to_string());
    let message = args
        .next()
        .unwrap_or_else(|| "This notification was sent over aw-notify's HTTP API.".to_string());

    // Reuse a single client per module; here we tag every notification with our name.
    let client = NotificationClient::new().with_sender("example-watcher");

    match client.notify(&title, &message) {
        // A 200 means aw-notify accepted/queued the request, not that the desktop
        // notification has actually been shown yet.
        Ok(()) => println!("Notification accepted by aw-notify: {title}"),
        Err(NotifyError::Transport(msg)) => {
            eprintln!("Could not reach aw-notify ({msg}). Is it running? Try: cargo run -- start");
            std::process::exit(1);
        }
        Err(NotifyError::Rejected(code)) => {
            eprintln!("aw-notify rejected the notification (HTTP {code}).");
            std::process::exit(1);
        }
    }
}
