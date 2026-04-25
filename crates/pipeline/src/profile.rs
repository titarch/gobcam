//! Optional structured profiling for trigger-path latency analysis.
//!
//! Enabled by passing `--profile-log <path>` (or `GOBCAM_PROFILE_LOG=...`)
//! to the daemon. Each call to [`mark`] writes one JSONL line; when the
//! profiler isn't initialized the call is a single atomic-load no-op.
//!
//! Per-event fields, always present:
//! - `ts_us` — wall-clock microseconds since `UNIX_EPOCH`. Comparable
//!   across processes on the same machine, so the consumer-side
//!   `perf_capture` harness (Stage 2) can align with these events.
//! - `thread` — thread name (best-effort).
//! - `event` — event name passed to [`mark`].
//!
//! Plus whatever the call site passes through `kvs`. Events are
//! line-buffered so a `tail -f` shows progress in real time.

use std::fs::OpenOptions;
use std::io::{LineWriter, Write};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{Value, json};

struct Profiler {
    out: Mutex<LineWriter<std::fs::File>>,
}

static PROFILE: OnceLock<Profiler> = OnceLock::new();

/// Open `path` (truncating) and install the global profiler. Idempotent
/// failure: a second call returns `Err`.
pub(crate) fn init(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening profile log {}", path.display()))?;
    let p = Profiler {
        out: Mutex::new(LineWriter::new(file)),
    };
    PROFILE
        .set(p)
        .map_err(|_| anyhow::anyhow!("profile already initialized"))?;
    mark("profile.init", json!({}));
    Ok(())
}

/// Whether profiling is active. Use to skip building expensive `kvs`
/// payloads at the call site.
pub(crate) fn enabled() -> bool {
    PROFILE.get().is_some()
}

/// Emit one event. `kvs` should be a `Value::Object` (use the
/// `serde_json::json!` macro). Non-object values are wrapped under a
/// `data` key. Failures (lock poisoning, IO errors) are swallowed —
/// tracing must never bring the daemon down.
pub(crate) fn mark(event: &str, kvs: Value) {
    let Some(p) = PROFILE.get() else {
        return;
    };
    let ts_us = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros())
            .unwrap_or(0),
    )
    .unwrap_or(u64::MAX);
    let thread = std::thread::current()
        .name()
        .map_or_else(|| String::from("?"), String::from);

    let mut obj = match kvs {
        Value::Object(o) => o,
        other => {
            let mut o = serde_json::Map::new();
            o.insert("data".into(), other);
            o
        }
    };
    obj.insert("ts_us".into(), json!(ts_us));
    obj.insert("thread".into(), json!(thread));
    obj.insert("event".into(), json!(event));

    let Ok(line) = serde_json::to_string(&Value::Object(obj)) else {
        return;
    };
    if let Ok(mut g) = p.out.lock() {
        let _ = writeln!(g, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_without_init_is_noop() {
        // No init() call; mark must not panic and must write nothing.
        mark("ignored", json!({ "x": 1 }));
    }

    // Note: `init` is global state, so tests that initialize would
    // collide with each other and with anything running in parallel.
    // The init path is exercised through the smoke test in Stage 1.
}
