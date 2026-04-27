//! Optional structured profiling: JSONL events with `ts_us` (microseconds
//! since `UNIX_EPOCH`), `thread`, and `event`, plus call-site `kvs`.
//! Disabled by default; [`mark`] is a single atomic-load no-op then.

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

/// Open `path` (truncating) and install the global profiler.
/// Returns `Err` if called twice.
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

/// Whether profiling is active. Gate expensive `kvs` payloads on this.
pub(crate) fn enabled() -> bool {
    PROFILE.get().is_some()
}

/// Emit one event. `kvs` should be a `Value::Object` (use `json!`); non-object
/// values are wrapped under a `data` key. IO/lock failures are swallowed.
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
        mark("ignored", json!({ "x": 1 }));
    }

    // `init` is global state; testing it would race with parallel tests.
}
