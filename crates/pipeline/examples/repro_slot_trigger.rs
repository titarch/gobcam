//! Playground: activate/deactivate cycle on pre-allocated appsrc slots.
//! Validates the Step 3 design: slots pre-attached at build time, each with
//! a Mutex<SlotState> the pump reads. Activate = swap source data + alpha=1,
//! deactivate = source=transparent + alpha=0.
//!
//! Timeline (auto):
//!   t=0   pipeline starts, all slots idle (alpha=0)
//!   t=1s  activate slot 0 (red),  alpha 0 -> 1
//!   t=4s  deactivate slot 0,                alpha 1 -> 0
//!   t=5s  activate slot 1 (green), alpha 0 -> 1
//!   t=8s  pipeline EOS

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{self as gst_app, AppSrc};

const N_SLOTS: usize = 4;
const W: i32 = 256;
const H: i32 = 256;
const FRAME_DUR_MS: u64 = 33;

#[derive(Clone)]
struct Slot {
    sink_pad: gst::Pad,
    state: Arc<Mutex<SlotState>>,
}

struct SlotState {
    /// Pixels to render. Initially transparent. Activate swaps in real data.
    pixels: Arc<Vec<u8>>,
    next_pts: gst::ClockTime,
}

impl Slot {
    fn activate(&self, pixels: Arc<Vec<u8>>) {
        self.state.lock().unwrap().pixels = pixels;
        self.sink_pad.set_property("alpha", 1.0_f64);
    }

    fn deactivate(&self, transparent: Arc<Vec<u8>>) {
        self.state.lock().unwrap().pixels = transparent;
        self.sink_pad.set_property("alpha", 0.0_f64);
    }
}

fn main() -> Result<()> {
    gst::init()?;

    let pipeline = gst::parse::launch(
        "videotestsrc num-buffers=240 pattern=ball ! \
         video/x-raw,width=640,height=480,framerate=30/1 ! \
         compositor name=mix background=black ! \
         videoconvert ! autovideosink",
    )?
    .downcast::<gst::Pipeline>()
    .map_err(|_| anyhow::anyhow!("not a pipeline"))?;

    let compositor = pipeline.by_name("mix").context("no compositor")?;
    let transparent = Arc::new(solid_pixels([0, 0, 0, 0]));

    let mut slots = Vec::with_capacity(N_SLOTS);
    for idx in 0..N_SLOTS {
        slots.push(build_slot(
            &pipeline,
            &compositor,
            idx,
            transparent.clone(),
        )?);
    }

    println!("starting pipeline ({N_SLOTS} slots, alpha=0)");
    pipeline.set_state(gst::State::Playing)?;

    let red = Arc::new(solid_pixels([255, 0, 0, 255]));
    let green = Arc::new(solid_pixels([0, 255, 0, 255]));

    let slots_for_test = slots.clone();
    let transparent_for_test = transparent.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(1));
        println!("[t=1s] activate slot 0 (red)");
        slots_for_test[0].activate(red);

        thread::sleep(Duration::from_secs(3));
        println!("[t=4s] deactivate slot 0");
        slots_for_test[0].deactivate(transparent_for_test.clone());

        thread::sleep(Duration::from_secs(1));
        println!("[t=5s] activate slot 1 (green)");
        slots_for_test[1].activate(green);
    });

    // Bus pump until EOS / error / 10s safety timeout.
    let bus = pipeline.bus().expect("bus");
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(200)) else {
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
    pipeline.set_state(gst::State::Null)?;
    println!("done");
    Ok(())
}

fn solid_pixels(rgba: [u8; 4]) -> Vec<u8> {
    (0..(W * H) as usize).flat_map(|_| rgba).collect()
}

fn build_slot(
    pipeline: &gst::Pipeline,
    compositor: &gst::Element,
    idx: usize,
    transparent: Arc<Vec<u8>>,
) -> Result<Slot> {
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
    appsrc.set_property("max-buffers", 2_u64);
    let convert = gst::ElementFactory::make("videoconvert")
        .name(format!("slot-{idx}-cv"))
        .build()?;
    let queue = gst::ElementFactory::make("queue")
        .name(format!("slot-{idx}-q"))
        .build()?;

    pipeline.add_many([appsrc.upcast_ref(), &convert, &queue])?;
    gst::Element::link_many([appsrc.upcast_ref(), &convert, &queue])?;

    let sink_pad = compositor
        .request_pad_simple("sink_%u")
        .context("sink pad")?;
    queue
        .static_pad("src")
        .context("queue src")?
        .link(&sink_pad)?;

    let (col, row) = (idx as i32 % 2, idx as i32 / 2);
    sink_pad.set_property("xpos", col * (W + 16));
    sink_pad.set_property("ypos", row * (H + 16));
    sink_pad.set_property("alpha", 0.0_f64);

    let state = Arc::new(Mutex::new(SlotState {
        pixels: transparent,
        next_pts: gst::ClockTime::ZERO,
    }));

    spawn_pump(idx, appsrc, state.clone());

    Ok(Slot { sink_pad, state })
}

fn spawn_pump(idx: usize, appsrc: AppSrc, state: Arc<Mutex<SlotState>>) {
    thread::Builder::new()
        .name(format!("slot-{idx}-pump"))
        .spawn(move || {
            let frame_dur = gst::ClockTime::from_mseconds(FRAME_DUR_MS);
            loop {
                let (pixels, pts) = {
                    let mut s = state.lock().unwrap();
                    let pixels = s.pixels.clone();
                    let pts = s.next_pts;
                    s.next_pts += frame_dur;
                    (pixels, pts)
                };

                let Ok(mut buffer) = gst::Buffer::with_size(pixels.len()) else {
                    return;
                };
                {
                    let bm = buffer.get_mut().unwrap();
                    if bm.copy_from_slice(0, &pixels).is_err() {
                        return;
                    }
                    bm.set_pts(pts);
                    bm.set_duration(frame_dur);
                }
                if appsrc.push_buffer(buffer).is_err() {
                    println!("slot {idx}: push failed, exiting");
                    return;
                }
            }
        })
        .expect("spawn pump");
}
