//! Consumer-side latency probe for the gobcam loopback.
//!
//! Reads `/dev/video10` (or wherever the daemon writes), watches a
//! patch in the bottom-right where reactions render, and emits one
//! JSONL line per relevant event:
//!
//! - `baseline_ready`  — the first N frames have been collected and the
//!   patch's mean RGB is the reference.
//! - `frame`           — every captured frame, with its delta vs the
//!   baseline. Useful for plotting; spammy.
//! - `novelty`         — the first frame after `baseline_ready` whose
//!   delta exceeds the threshold. The harness exits immediately after.
//!
//! Timestamps are `SystemTime::now()` microseconds since `UNIX_EPOCH`,
//! identical to the daemon's `--profile-log` events; you can paste both
//! files into `jq -s 'add'` and align by `ts_us`.
//!
//! Run via `just perf-capture` or
//! `cargo run -p gobcam-pipeline --example perf_capture --release -- --help`.

use std::fs::File;
use std::io::{LineWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{self as gst_app, AppSink};
use serde_json::json;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Loopback device the daemon writes to.
    #[arg(long, default_value = "/dev/video10")]
    device: String,
    /// JSONL output path.
    #[arg(long, default_value = "/tmp/gobcam-perf-capture.jsonl")]
    output: PathBuf,
    /// Frame count for the patch baseline.
    #[arg(long, default_value_t = 30)]
    baseline_frames: u32,
    /// Patch top-left X (within the 1280×720 frame).
    #[arg(long, default_value_t = 992)]
    patch_x: u32,
    /// Patch top-left Y.
    #[arg(long, default_value_t = 432)]
    patch_y: u32,
    /// Patch side length in pixels (square).
    #[arg(long, default_value_t = 256)]
    patch_size: u32,
    /// Trigger novelty event when channel-mean L2 distance (0–255)
    /// exceeds this value.
    #[arg(long, default_value_t = 18.0)]
    threshold: f64,
    /// Emit a `frame` event for every captured frame too. Spammy but
    /// useful for plotting α(t) against the daemon's events.
    #[arg(long)]
    log_all_frames: bool,
    /// Stop on first novelty event (default). Pass `--watch` to keep
    /// running and re-baseline after each detection.
    #[arg(long)]
    watch: bool,
}

const FRAME_W: u32 = 1280;
const FRAME_H: u32 = 720;

struct Logger {
    out: Mutex<LineWriter<File>>,
}

impl Logger {
    fn new(path: &PathBuf) -> Result<Self> {
        let f = File::create(path).with_context(|| format!("creating {}", path.display()))?;
        Ok(Self {
            out: Mutex::new(LineWriter::new(f)),
        })
    }

    fn log(&self, payload: serde_json::Value) {
        let ts_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        let mut obj = match payload {
            serde_json::Value::Object(o) => o,
            other => {
                let mut o = serde_json::Map::new();
                o.insert("data".into(), other);
                o
            }
        };
        obj.insert("ts_us".into(), json!(ts_us));
        let line = serde_json::to_string(&serde_json::Value::Object(obj)).unwrap_or_default();
        if let Ok(mut g) = self.out.lock() {
            let _ = writeln!(g, "{line}");
        }
    }
}

#[derive(Default)]
struct Baseline {
    sum: [f64; 3],
    n: u32,
}

impl Baseline {
    fn add(&mut self, m: [f64; 3]) {
        for i in 0..3 {
            self.sum[i] += m[i];
        }
        self.n += 1;
    }
    fn mean(&self) -> Option<[f64; 3]> {
        if self.n == 0 {
            return None;
        }
        let n = f64::from(self.n);
        Some([self.sum[0] / n, self.sum[1] / n, self.sum[2] / n])
    }
}

fn patch_mean(pixels: &[u8], stride: u32, patch: (u32, u32, u32)) -> Option<[f64; 3]> {
    let (px, py, sz) = patch;
    if px + sz > FRAME_W || py + sz > FRAME_H {
        return None;
    }
    let mut sum = [0u64; 3];
    let mut count = 0u64;
    for y in py..py + sz {
        let row = y as usize * stride as usize;
        for x in px..px + sz {
            let p = row + x as usize * 3;
            if p + 2 >= pixels.len() {
                return None;
            }
            sum[0] += u64::from(pixels[p]);
            sum[1] += u64::from(pixels[p + 1]);
            sum[2] += u64::from(pixels[p + 2]);
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let c = count as f64;
    Some([sum[0] as f64 / c, sum[1] as f64 / c, sum[2] as f64 / c])
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) / 3.0).sqrt()
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    gst::init()?;

    let logger = Arc::new(Logger::new(&cli.output)?);
    logger.log(json!({"event": "start", "patch": [cli.patch_x, cli.patch_y, cli.patch_size]}));

    let pipeline_str = format!(
        "v4l2src device={dev} ! videoconvert ! \
         video/x-raw,format=RGB,width={w},height={h} ! \
         appsink name=sink sync=false max-buffers=2 drop=false",
        dev = cli.device,
        w = FRAME_W,
        h = FRAME_H,
    );
    let pipeline = gst::parse::launch(&pipeline_str)
        .context("parsing capture pipeline")?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow::anyhow!("not a Pipeline"))?;
    let sink = pipeline
        .by_name("sink")
        .context("no appsink named 'sink'")?
        .dynamic_cast::<AppSink>()
        .map_err(|_| anyhow::anyhow!("'sink' is not an appsink"))?;

    let baseline = Arc::new(Mutex::new(Baseline::default()));
    let baseline_ref: Arc<Mutex<Option<[f64; 3]>>> = Arc::new(Mutex::new(None));
    let frame_n = Arc::new(AtomicU32::new(0));
    let done = Arc::new(AtomicBool::new(false));

    let logger_cb = Arc::clone(&logger);
    let baseline_cb = Arc::clone(&baseline);
    let baseline_ref_cb = Arc::clone(&baseline_ref);
    let frame_n_cb = Arc::clone(&frame_n);
    let done_cb = Arc::clone(&done);
    let log_all = cli.log_all_frames;
    let watch_mode = cli.watch;
    let baseline_target = cli.baseline_frames;
    let threshold = cli.threshold;
    let patch = (cli.patch_x, cli.patch_y, cli.patch_size);

    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |s| {
                let sample = s.pull_sample().map_err(|_| gst::FlowError::Error)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let stride = FRAME_W * 3;
                let mean =
                    patch_mean(map.as_slice(), stride, patch).ok_or(gst::FlowError::Error)?;
                let n = frame_n_cb.fetch_add(1, Ordering::AcqRel);

                let ref_now = *baseline_ref_cb.lock().expect("ref poisoned");
                if let Some(ref_mean) = ref_now {
                    let delta = distance(mean, ref_mean);
                    if log_all {
                        logger_cb.log(json!({
                            "event": "frame",
                            "frame_n": n,
                            "delta": delta,
                            "mean": mean,
                        }));
                    }
                    if delta > threshold {
                        logger_cb.log(json!({
                            "event": "novelty",
                            "frame_n": n,
                            "delta": delta,
                            "mean": mean,
                        }));
                        if watch_mode {
                            // Re-baseline so the next event captures the
                            // next change.
                            *baseline_ref_cb.lock().expect("ref poisoned") = Some(mean);
                        } else {
                            done_cb.store(true, Ordering::Release);
                        }
                    }
                } else {
                    let mut b = baseline_cb.lock().expect("baseline poisoned");
                    b.add(mean);
                    if b.n >= baseline_target {
                        let avg = b.mean().expect("baseline mean");
                        *baseline_ref_cb.lock().expect("ref poisoned") = Some(avg);
                        logger_cb.log(json!({
                            "event": "baseline_ready",
                            "n_frames": b.n,
                            "mean": avg,
                        }));
                    }
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    pipeline
        .set_state(gst::State::Playing)
        .context("starting capture pipeline")?;

    while !done.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(10));
    }
    pipeline.set_state(gst::State::Null).ok();
    Ok(())
}
