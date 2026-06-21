# aw-notify-rs

A simplified Rust implementation of [aw-notify](https://github.com/ActivityWatch/aw-notify) that matches the Python version's behavior while providing Rust's performance and safety benefits.

## Overview

This is a Rust rewrite of the Python version, organized across multiple modules, while maintaining:

- ✅ **Identical behavior** to the Python implementation
- ✅ **Type safety** and memory safety of Rust
- ✅ **Zero runtime overhead** with native compilation
- ✅ **Simple architecture** that's easy to understand and maintain

## Features

- **Time Summaries**: Get daily and hourly summaries of your most-used categories
- **Threshold Alerts**: Receive notifications when you reach specific time thresholds
- **Server Monitoring**: Get notified if the ActivityWatch server goes down
- **New Day Greetings**: Start your day with a greeting showing the current date
- **Cross-platform**: Native desktop notifications on macOS, Linux, and Windows
- **Smart Caching**: 60-second TTL cache reduces server requests (matches Python's `@cache_ttl`)

## Installation

### Prerequisites

- Rust toolchain (1.70+)
- ActivityWatch server running (default: localhost:5600)

### Building from source

```bash
# Clone the repository
git clone https://github/0xbrayo/aw-notify-rs.git
cd aw-notify-rs

# Build the application
cargo build --release

# The binary will be available at target/release/aw-notify
```

## Usage

### Starting the notification service

```bash
# Start with default settings
./target/release/aw-notify start

# Start in testing mode (connects to port 5666)
./target/release/aw-notify --testing start

# Start with custom port
./target/release/aw-notify --port 5678 start

# Enable verbose logging
./target/release/aw-notify --verbose start
```

### Sending a one-time summary notification

```bash
# Send summary of today's activity (top-level categories only)
./target/release/aw-notify checkin

# Send detailed summary with all category levels (parent and leaf categories)
./target/release/aw-notify checkin-detailed

# Send summary in testing mode
./target/release/aw-notify --testing checkin
```

### Command-line options

```
ActivityWatch notification service

Usage: aw-notify [OPTIONS] [COMMAND]

Commands:
  start            Start the notification service
  checkin          Send a summary notification (top-level categories)
  checkin-detailed Send a detailed summary with all category levels
  help             Print this message or the help of the given subcommand(s)

Options:
  -v, --verbose      Verbose logging
      --testing      Testing mode (port 5666)
      --port <PORT>  Port to connect to ActivityWatch server
  -h, --help         Print help
  -V, --version      Print version
```

## Sending notifications from other modules

While the service is running it exposes a small local HTTP API so other
ActivityWatch modules (watchers, importers, scripts) can push their own desktop
notifications through aw-notify. Requests are queued and rate-limited centrally,
so multiple modules won't spam the user.

### Endpoint

```
POST http://127.0.0.1:<http_port>/notify        # http_port defaults to 5667
Content-Type: application/json
```

Request body:

| Field     | Type   | Required | Notes                                                            |
|-----------|--------|----------|------------------------------------------------------------------|
| `title`   | string | yes      | Notification title.                                              |
| `message` | string | yes      | Notification body.                                               |
| `sender`  | string | no       | Originating module; shown as `Title (sender)`. Alias: `watcher`. |

Responses:

| Status | Meaning                                                        |
|--------|---------------------------------------------------------------|
| `200`  | Queued for display.                                           |
| `400`  | Invalid or oversized JSON body.                              |
| `429`  | Notification queue is full — back off and retry later.       |
| `503`  | Service not ready to accept notifications.                  |
| `404`  | Wrong path or method.                                       |

The port is configurable via the `http_port` config key (see [Configuration Options](#configuration-options)).

### Quick test with curl

```bash
curl -s -X POST http://127.0.0.1:5667/notify \
  -H 'Content-Type: application/json' \
  -d '{"title":"Backup done","message":"Synced 1,234 events","watcher":"my-script"}'
```

### From Rust: the `aw-notify-client` crate

Rust modules can use the lightweight [`aw-notify-client`](crates/aw-notify-client)
crate (no TLS/hyper/reqwest pulled in — just `serde` and `ureq`):

```toml
# Cargo.toml
[dependencies]
aw-notify-client = { git = "https://github.com/ActivityWatch/aw-notify-rs", branch = "master" }
```

```rust
use aw_notify_client::NotificationClient;

let client = NotificationClient::new().with_sender("aw-watcher-foo");
match client.notify("Backup done", "Synced 1,234 events") {
    Ok(()) => {}
    Err(e) => eprintln!("notify failed: {e}"), // e.g. aw-notify not running, or queue full
}
```

A runnable example lives in [`examples/notify_client.rs`](examples/notify_client.rs):

```bash
cargo run --example notify_client "Build finished" "All tests passed ✅"
```

## Category Aggregation

aw-notify-rs supports three different category aggregation modes for analyzing your time:

### Aggregation Modes

1. **None** (`CategoryAggregation::None`)
   - Returns full category hierarchy as-is
   - Example: `"Work > Programming > Rust"` stays as `"Work > Programming > Rust"`
   - Use case: When you need detailed, granular category information

2. **TopLevelOnly** (`CategoryAggregation::TopLevelOnly`)
   - Aggregates all subcategories into their top-level parent
   - Example: `"Work > Programming > Rust"` becomes just `"Work"`
   - Use case: High-level overview of time spent across major categories
   - Command: `./target/release/aw-notify checkin`

3. **AllLevels** (`CategoryAggregation::AllLevels`)
   - Aggregates by all category levels (both parent and leaf categories)
   - Example: `"Work > Programming > Rust"` creates entries for:
     - `"Work"` (total time including all subcategories)
     - `"Work > Programming"` (total time including all its subcategories)
     - `"Work > Programming > Rust"` (leaf category time)
   - Use case: Comprehensive analysis showing both overview and details
   - Command: `./target/release/aw-notify checkin-detailed`

### Example Usage

The aggregation mode is controlled through the `CategoryAggregation` enum:

```rust
// Get detailed hierarchy
let cat_time = get_time(None, CategoryAggregation::None)?;

// Get top-level summary only
let cat_time = get_time(None, CategoryAggregation::TopLevelOnly)?;

// Get all levels (parent + leaf categories)
let cat_time = get_time(None, CategoryAggregation::AllLevels)?;
```

### Practical Example

Given these ActivityWatch categories with times:
- `"Work > Programming > Rust"`: 30 minutes
- `"Work > Programming > Python"`: 20 minutes
- `"Work > Meetings"`: 15 minutes
- `"Personal > Reading"`: 10 minutes

**None** returns:
```
Work > Programming > Rust: 30m
Work > Programming > Python: 20m
Work > Meetings: 15m
Personal > Reading: 10m
```

**TopLevelOnly** returns:
```
Work: 65m (30 + 20 + 15)
Personal: 10m
```

**AllLevels** returns:
```
Work: 65m (total)
Work > Programming: 50m (30 + 20)
Work > Programming > Rust: 30m
Work > Programming > Python: 20m
Work > Meetings: 15m
Personal: 10m
Personal > Reading: 10m
```

## Architecture

This implementation uses a simplified architecture that mirrors the Python version:

### Module Structure
- **`main.rs`** — core logic, notification handling, daemon threads, and CLI
- **`dirs.rs`** — platform-aware configuration directory resolution
- **`logging.rs`** — logging setup and configuration
- **Global state** using `once_cell::Lazy` (matches Python's globals)
- **Simple daemon threads** for background tasks (matches Python's threading)

### Core Components
- **CategoryAlert**: Tracks time thresholds (exact match to Python class)
- **Caching**: TTL cache with 60-second expiration (matches Python's `@cache_ttl`)
- **Notifications**: macOS terminal-notifier → notify-rust fallback (matches Python)
- **Query System**: Canonical events queries (identical to Python)

### Background Threads
- **Threshold monitoring**: Checks category time limits every 10 seconds
- **Hourly checkins**: Sends summaries at the top of each hour (if active)
- **New day notifications**: Greets user when they first become active each day
- **Server monitoring**: Alerts when ActivityWatch server goes up/down

## Configuration

aw-notify-rs supports flexible configuration through a TOML configuration file. By default, it looks for a configuration file at `~/.config/aw-notify/config.toml`.

### Configuration File Location

The default configuration file location varies by operating system:

- **Linux**: `~/.config/aw-notify/config.toml`
- **macOS**: `~/Library/Application Support/aw-notify/config.toml`
- **Windows**: `%APPDATA%\aw-notify\config.toml`

### Automatic Configuration Generation

When you first run aw-notify-rs, it will automatically create a default configuration file if one doesn't exist. The configuration file will be created with sensible defaults that match the original hardcoded behavior.

```bash
# First run will create the default config automatically
./target/release/aw-notify start
```

### Configuration Options

The configuration file supports the following options:

#### Feature Toggles
- `hourly_checkins`: Enable/disable hourly activity summaries (default: true)
- `new_day_greetings`: Enable/disable new day greeting notifications (default: true)
- `server_monitoring`: Enable/disable ActivityWatch server monitoring alerts (default: true)
- `http_port`: Port for the local HTTP API that other modules use to send notifications (default: 5667). See [Sending notifications from other modules](#sending-notifications-from-other-modules).

#### Category Alerts
Configure custom category alerts using the `[[alerts]]` sections:

```toml
[[alerts]]
category = "Programming"           # Category name (must match ActivityWatch)
label = "💻 Programming"          # Display label with optional emoji
thresholds_minutes = [30, 60, 120, 180, 240]  # Alert thresholds in minutes
positive = true                    # true = "Goal reached!", false = "Time spent" warning
```

### Practical Configuration Examples

#### Focus-Oriented Configuration with Nested Categories
For users who want to minimize distractions and track specific work activities:

```toml
hourly_checkins = false          # Disable hourly interruptions
new_day_greetings = true
server_monitoring = true

# Track overall programming time
[[alerts]]
category = "Work > Programming"
label = "💻 Programming"
thresholds_minutes = [60, 120, 180, 240]
positive = true                  # Celebrate coding achievements

# Track specific languages/projects
[[alerts]]
category = "Work > Programming > Rust"
label = "🦀 Rust Development"
thresholds_minutes = [30, 60, 90, 120]
positive = true

[[alerts]]
category = "Work > Programming > Python"
label = "🐍 Python Development"
thresholds_minutes = [30, 60, 90, 120]
positive = true

# Limit meeting time
[[alerts]]
category = "Work > Meetings"
label = "📅 Meetings"
thresholds_minutes = [30, 60, 120]
positive = false

# Limit distractions
[[alerts]]
category = "Social Media"
label = "📱 Social Media"
thresholds_minutes = [15, 30]    # Early warnings for social media
positive = false

[[alerts]]
category = "YouTube"
label = "📺 YouTube"
thresholds_minutes = [20, 45]    # Limit video consumption
positive = false
```

#### Simple Focus Configuration (Top-Level Only)
For users who prefer simpler, high-level tracking:

```toml
hourly_checkins = false          # Disable hourly interruptions
new_day_greetings = true
server_monitoring = true

[[alerts]]
category = "Programming"
label = "💻 Programming"
thresholds_minutes = [30, 60, 120, 180, 240]
positive = true                  # Celebrate coding achievements

[[alerts]]
category = "Social Media"
label = "📱 Social Media"
thresholds_minutes = [15, 30]    # Early warnings for social media
positive = false

[[alerts]]
category = "YouTube"
label = "📺 YouTube"
thresholds_minutes = [20, 45]    # Limit video consumption
positive = false
```

#### Balanced Lifestyle Configuration
For users who want gentle reminders without being too restrictive:

```toml
hourly_checkins = true
new_day_greetings = true
server_monitoring = true

[[alerts]]
category = "Work"
label = "💼 Work"
thresholds_minutes = [60, 120, 180, 240]
positive = true

[[alerts]]
category = "Reading"
label = "📚 Reading"
thresholds_minutes = [30, 60, 90]
positive = true

[[alerts]]
category = "All"
label = "Total Activity"
thresholds_minutes = [480]       # 8-hour daily reminder only
positive = false
```

#### Minimal Configuration
For users who only want essential notifications:

```toml
hourly_checkins = false
new_day_greetings = false
server_monitoring = true

[[alerts]]
category = "All"
label = "Daily Activity"
thresholds_minutes = [360, 480, 600]  # 6h, 8h, 10h warnings
positive = false
```

### Default Configuration

If no configuration file is found, the application uses these default alerts:

- **All activities**: 1h, 2h, 4h, 6h, 8h notifications
- **Twitter**: 15min, 30min, 1h warnings
- **YouTube**: 15min, 30min, 1h warnings
- **Work**: 15min, 30min, 1h, 2h, 4h achievements (shown as "Goal reached!")

## Documentation

- **[README.md](README.md)** - Main documentation (this file)
- **[config.example.toml](config.example.toml)** - Example configuration file

## Notification Types

1. **Threshold alerts**: "Time spent" or "Goal reached!" when limits hit
2. **Hourly summaries**: Top categories every hour (when active)
3. **Daily summaries**: "Time today" and "Time yesterday" reports
4. **New day greetings**: Welcome message with current date
5. **Server status**: Alerts when ActivityWatch server connectivity changes

### Tips for Configuration

- **Category Names**: Must match exactly what ActivityWatch reports. Check your ActivityWatch dashboard to see available categories.
- **Positive vs. Negative Alerts**: Use `positive = true` for activities you want to encourage (shows "Goal reached!"), and `positive = false` for activities you want to limit (shows "Time spent").
- **Threshold Strategy**: Start with longer thresholds and adjust based on your habits. Too many notifications can become counterproductive.
- **Emoji in Labels**: Use emoji in labels to make notifications more visually distinctive and easier to identify at a glance.
- **Testing Configuration**: Use `--output-only` flag to test your configuration without desktop notifications: `./target/release/aw-notify --output-only start`
- **Editing Configuration**: After the initial run, you can edit the generated configuration file to customize your alerts and preferences.

## Compatibility
- **100% behavioral compatibility** with Python version
- **Identical queries** and time calculations
- **Same notification logic** and message formatting
- **Matching cache behavior** and error handling


### Building

```bash
# Development build
cargo build

# Release build (optimized)
cargo build --release

# Check for errors
cargo check

# Run with logging
RUST_LOG=debug cargo run -- --verbose checkin
```

## Troubleshooting

### Server Connection Issues

If the service can't connect to ActivityWatch:

1. Ensure ActivityWatch server is running
2. Check the correct port (default: 5600, testing: 5666)
3. Verify server accessibility: `curl http://localhost:5600/api/0/info`


## License

This project is licensed under the Mozilla Public License 2.0 (MPL-2.0), the same as the ActivityWatch project.

## Acknowledgments

This is a simplified rewrite of the original [aw-notify](https://github.com/ActivityWatch/aw-notify) Python implementation by Erik Bjäreholt and the ActivityWatch team, designed to match its behavior exactly while providing Rust's performance and safety benefits.
