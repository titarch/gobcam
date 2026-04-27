//! Playground: lazy pumps. Each appsrc slot pushes ONE buffer at init
//! and then sleeps on a condvar. The compositor's
//! `max-last-buffer-repeat=∞` (its default) means it keeps reusing that
//! buffer for every output frame with zero further pulls.
//!
//! On activation, the driver wakes a slot and the pump pushes a burst
//! of fresh frames at 30 fps with monotonic PTS. After the burst it
//! pushes one final "back to base" buffer and sleeps again.
//!
//! What we want to see:
//!   * slots' buffer-push counters stay flat between bursts (proving
//!     idle pumps don't push, the compositor doesn't pull, no churn);
//!   * the compositor output still shows every slot colored even
//!     while pumps sleep (proving max-last-buffer-repeat works);
//!   * activation latency stays sub-frame (the new color appears on
//!     the next output tick after wake-up).
//!
//! Env knobs:
//!   * `PURE_IDLE=1` — never trigger, just measure idle cost.
//!   * `ALWAYS_PUMP=1` — push at 30 fps continuously regardless of
//!     trigger state. Same topology as the lazy mode for fair
//!     head-to-head CPU comparison.
//!   * `GST_SINK=autovideosink` — show a window instead of fakesink.
//!
//! Run with `cargo run -p gobcam-pipeline --example repro_idle_lazy_pumps`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{self as gst_app, AppSrc};

const N_SLOTS: usize = 48;
const W: i32 = 256;
const H: i32 = 256;
const BURST_FRAMES: u32 = 30;

struct SlotPump {
    state: Arc<Mutex<PumpState>>,
    wake: Arc<Condvar>,
    pushed: Arc<AtomicU64>,
}

struct PumpState {
    base_color: [u8; 4],
    active_color: Option<[u8; 4]>,
    burst_remaining: u32,
    next_pts: gst::ClockTime,
}

fn main() -> Result<()> {
    gst::init()?;

    // Default sink is fakesink for headless CPU measurement.
    // Pass `GST_SINK=autovideosink` to get a window instead.
    let sink = std::env::var("GST_SINK").unwrap_or_else(|_| "fakesink sync=false".into());
    let pipeline_str = format!(
        "videotestsrc is-live=true pattern=ball ! \
         video/x-raw,width=640,height=480,framerate=30/1 ! \
         compositor name=mix background=black ! \
         videoconvert ! {sink}"
    );
    let pipeline = gst::parse::launch(&pipeline_str)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow::anyhow!("not a pipeline"))?;

    let compositor = pipeline.by_name("mix").context("no compositor")?;

    let colors_base: Vec<[u8; 4]> = (0..N_SLOTS)
        .map(|i| {
            let h = (i as f32) * 360.0 / (N_SLOTS as f32);
            let (r, g, b) = hsv_to_rgb(h, 1.0, 0.3);
            [r, g, b, 255]
        })
        .collect();
    let colors_active: Vec<[u8; 4]> = (0..N_SLOTS)
        .map(|i| {
            let h = (i as f32) * 360.0 / (N_SLOTS as f32);
            let (r, g, b) = hsv_to_rgb(h, 1.0, 1.0);
            [r, g, b, 255]
        })
        .collect();

    let always_pump = std::env::var_os("ALWAYS_PUMP").is_some();
    let mut pumps: Vec<SlotPump> = Vec::with_capacity(N_SLOTS);
    for idx in 0..N_SLOTS {
        let pump = build_slot(&pipeline, &compositor, idx, colors_base[idx], always_pump)?;
        pumps.push(pump);
    }
    if always_pump {
        println!("ALWAYS_PUMP=1 — pumps push at 30 fps continuously (baseline mode)");
    }

    println!("starting pipeline with {N_SLOTS} lazy pumps");
    pipeline.set_state(gst::State::Playing)?;

    // Driver thread: every 1.5 s, pick a slot and trigger a burst.
    let pump_handles: Vec<_> = pumps
        .iter()
        .map(|p| (p.state.clone(), p.wake.clone(), p.pushed.clone()))
        .collect();

    // PURE_IDLE=1 → never trigger; just measure idle cost.
    // ALWAYS_PUMP=1 → make every pump push at 30 fps continuously
    //                 (baseline). Triggers still mark "active" frames
    //                 differently but pump rate is fixed.
    let pure_idle = std::env::var_os("PURE_IDLE").is_some();
    let driver = thread::spawn({
        let handles = pump_handles.clone();
        let active_colors = colors_active;
        move || {
            // First, give the pipeline a beat to negotiate caps and the
            // initial init-buffer to land.
            thread::sleep(Duration::from_millis(500));
            if pure_idle {
                return;
            }
            for cycle in 0..6 {
                let idx = cycle % N_SLOTS;
                let trigger_at = Instant::now();
                {
                    let (state, wake, _) = &handles[idx];
                    let mut s = state.lock().unwrap();
                    s.active_color = Some(active_colors[idx]);
                    s.burst_remaining = BURST_FRAMES;
                    wake.notify_one();
                }
                println!(
                    "[t={:>5}ms] trigger slot {idx} ({} burst frames)",
                    trigger_at.elapsed().as_millis(),
                    BURST_FRAMES
                );
                thread::sleep(Duration::from_millis(1500));
            }
        }
    });

    // Reporter thread: every second, print push-counters per slot.
    let reporter_handles = pump_handles.clone();
    let reporter_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_signal = reporter_stop.clone();
    let reporter = thread::spawn(move || {
        let mut last = vec![0_u64; N_SLOTS];
        let start = Instant::now();
        while !stop_signal.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));
            let mut deltas = Vec::with_capacity(N_SLOTS);
            for (i, (_, _, pushed)) in reporter_handles.iter().enumerate() {
                let now = pushed.load(Ordering::Relaxed);
                deltas.push(now - last[i]);
                last[i] = now;
            }
            let totals: Vec<_> = last.iter().copied().collect();
            println!(
                "[t={:>5}ms] pushes/sec per slot: {:?} | totals: {:?}",
                start.elapsed().as_millis(),
                deltas,
                totals
            );
        }
    });

    // Bus pump until driver finishes + a bit of tail.
    let bus = pipeline.bus().expect("bus");
    let deadline = Instant::now() + Duration::from_secs(11);
    while Instant::now() < deadline {
        let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) else {
            continue;
        };
        use gst::MessageView;
        match msg.view() {
            MessageView::Eos(..) => {
                println!("EOS");
                break;
            }
            MessageView::Error(err) => {
                println!("ERROR: {} ({:?})", err.error(), err.debug());
                break;
            }
            _ => {}
        }
    }
    reporter_stop.store(true, Ordering::Relaxed);
    let _ = driver.join();
    let _ = reporter.join();
    pipeline.set_state(gst::State::Null)?;
    Ok(())
}

fn build_slot(
    pipeline: &gst::Pipeline,
    compositor: &gst::Element,
    idx: usize,
    base_color: [u8; 4],
    always_pump: bool,
) -> Result<SlotPump> {
    let appsrc = AppSrc::builder()
        .name(format!("slot-{idx}-src"))
        .caps(
            &gst::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .field("width", W)
                .field("height", H)
                .field("framerate", gst::Fraction::new(30, 1))
                .build(),
        )
        .format(gst::Format::Time)
        .is_live(true)
        .block(true)
        .stream_type(gst_app::AppStreamType::Stream)
        .build();
    appsrc.set_property("max-buffers", 1_u64);

    let convert = gst::ElementFactory::make("videoconvert")
        .name(format!("slot-{idx}-cv"))
        .build()?;
    let queue = gst::ElementFactory::make("queue")
        .name(format!("slot-{idx}-q"))
        .build()?;
    queue.set_property("max-size-buffers", 1_u32);
    queue.set_property("max-size-time", 0_u64);
    queue.set_property("max-size-bytes", 0_u32);

    pipeline.add_many([appsrc.upcast_ref(), &convert, &queue])?;
    gst::Element::link_many([appsrc.upcast_ref(), &convert, &queue])?;

    let sink_pad = compositor
        .request_pad_simple("sink_%u")
        .context("sink pad")?;
    queue
        .static_pad("src")
        .context("queue src")?
        .link(&sink_pad)?;

    // 8-wide grid layout (N=48 → 8x6). alpha controlled below.
    let cols: i32 = 8;
    let cell = 64; // smaller cell so a full grid fits in 640x480
    let (col, row) = (
        i32::try_from(idx).unwrap() % cols,
        i32::try_from(idx).unwrap() / cols,
    );
    sink_pad.set_property("xpos", col * cell);
    sink_pad.set_property("ypos", row * cell);
    sink_pad.set_property("alpha", 0.6_f64);
    // max-last-buffer-repeat defaults to u64::MAX (∞) — explicit here
    // for clarity; this is the property doing the actual work.
    sink_pad.set_property("max-last-buffer-repeat", u64::MAX);

    let state = Arc::new(Mutex::new(PumpState {
        base_color,
        active_color: None,
        burst_remaining: 0,
        next_pts: gst::ClockTime::ZERO,
    }));
    let wake = Arc::new(Condvar::new());
    let pushed = Arc::new(AtomicU64::new(0));

    spawn_pump(
        idx,
        appsrc,
        state.clone(),
        wake.clone(),
        pushed.clone(),
        always_pump,
    );

    Ok(SlotPump {
        state,
        wake,
        pushed,
    })
}

fn spawn_pump(
    idx: usize,
    appsrc: AppSrc,
    state: Arc<Mutex<PumpState>>,
    wake: Arc<Condvar>,
    pushed: Arc<AtomicU64>,
    always_pump: bool,
) {
    thread::Builder::new()
        .name(format!("slot-{idx}-pump"))
        .spawn(move || {
            // Frame duration for active bursts.
            let active_dur = gst::ClockTime::from_mseconds(33);

            // Push the initial base buffer so the compositor has
            // something to repeat. PTS=0 — the segment running-time
            // will be ahead by the time this lands, but with
            // max-last-buffer-repeat=∞ it gets reused regardless.
            let init_buf = build_buffer(
                {
                    let s = state.lock().unwrap();
                    s.base_color
                },
                gst::ClockTime::ZERO,
                active_dur,
            );
            if appsrc.push_buffer(init_buf).is_ok() {
                pushed.fetch_add(1, Ordering::Relaxed);
                let mode = if always_pump { "always-pump" } else { "lazy" };
                println!("slot {idx}: pushed init buffer ({mode})");
            }

            loop {
                // Lazy mode sleeps on the condvar until armed; always-pump
                // mode never sleeps and just pushes whatever color is
                // current (base while idle, active during a burst).
                let (color, pts, just_finished_burst) = {
                    let mut s = state.lock().unwrap();
                    if !always_pump {
                        while s.burst_remaining == 0 {
                            s = wake.wait(s).unwrap();
                        }
                    }
                    let color = s.active_color.unwrap_or(s.base_color);
                    let pts = s.next_pts;
                    s.next_pts += active_dur;
                    let mut just_finished = false;
                    if s.burst_remaining > 0 {
                        s.burst_remaining -= 1;
                        if s.burst_remaining == 0 {
                            s.active_color = None;
                            just_finished = true;
                        }
                    }
                    (color, pts, just_finished)
                };

                let buf = build_buffer(color, pts, active_dur);
                if appsrc.push_buffer(buf).is_err() {
                    println!("slot {idx}: push failed, exiting");
                    return;
                }
                pushed.fetch_add(1, Ordering::Relaxed);

                // Lazy mode is about to re-sleep — push one final base
                // buffer so the compositor's `max-last-buffer-repeat`
                // shows base, not the last active color, while we sleep.
                // Always-pump would emit the same base on the next tick
                // anyway, so it skips this.
                if just_finished_burst && !always_pump {
                    let (base, pts) = {
                        let mut s = state.lock().unwrap();
                        let p = s.next_pts;
                        s.next_pts += active_dur;
                        (s.base_color, p)
                    };
                    let buf = build_buffer(base, pts, active_dur);
                    if appsrc.push_buffer(buf).is_ok() {
                        pushed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        })
        .expect("spawn pump");
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let h6 = (h / 60.0).rem_euclid(6.0);
    let x = c * (1.0 - (h6 % 2.0 - 1.0).abs());
    let (r, g, b) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

fn build_buffer(color: [u8; 4], pts: gst::ClockTime, dur: gst::ClockTime) -> gst::Buffer {
    let pixels: Vec<u8> = (0..usize::try_from(W * H).unwrap())
        .flat_map(|_| color)
        .collect();
    let mut buf = gst::Buffer::with_size(pixels.len()).expect("buffer alloc");
    {
        let bm = buf.get_mut().expect("fresh buffer");
        bm.copy_from_slice(0, &pixels).expect("copy");
        bm.set_pts(pts);
        bm.set_duration(dur);
    }
    buf
}
