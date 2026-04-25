//! Playground: a `compositor` with one base `videotestsrc` input and N
//! pre-allocated `appsrc` slots, each driven by a pump thread pushing RGBA
//! frames. Mirrors the daemon's failed slot implementation closely so we
//! can see what specifically goes wrong.
//!
//! Run with `cargo run --example pg_appsrc_slots`.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{self as gst_app, AppSrc};

const N_SLOTS: usize = 4;
const W: i32 = 256;
const H: i32 = 256;

fn main() -> Result<()> {
    gst::init()?;

    let pipeline = gst::parse::launch(
        "videotestsrc num-buffers=300 pattern=ball ! \
         video/x-raw,width=640,height=480,framerate=30/1 ! \
         compositor name=mix background=black ! \
         videoconvert ! autovideosink",
    )?
    .downcast::<gst::Pipeline>()
    .map_err(|_| anyhow::anyhow!("not a pipeline"))?;

    let compositor = pipeline.by_name("mix").context("no compositor")?;

    for idx in 0..N_SLOTS {
        build_slot(&pipeline, &compositor, idx)?;
    }

    println!("starting pipeline with {N_SLOTS} appsrc slots");
    pipeline.set_state(gst::State::Playing)?;

    // Pump bus messages until EOS / error / 8s.
    let bus = pipeline.bus().expect("bus");
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(8) {
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
            MessageView::StateChanged(s)
                if s.src().map(|x| x.name().to_string()).as_deref() == Some("pipeline0") =>
            {
                println!("pipeline state: {:?} -> {:?}", s.old(), s.current());
            }
            _ => {}
        }
    }
    pipeline.set_state(gst::State::Null)?;
    Ok(())
}

fn build_slot(pipeline: &gst::Pipeline, compositor: &gst::Element, idx: usize) -> Result<()> {
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

    // Position each slot in a 2x2 grid.
    let (col, row) = (idx as i32 % 2, idx as i32 / 2);
    sink_pad.set_property("xpos", col * (W + 16));
    sink_pad.set_property("ypos", row * (H + 16));
    sink_pad.set_property("alpha", 0.5_f64);

    spawn_pump(idx, appsrc);
    Ok(())
}

fn spawn_pump(idx: usize, appsrc: AppSrc) {
    let pixel_color = match idx {
        0 => [255, 0, 0, 255],
        1 => [0, 255, 0, 255],
        2 => [0, 0, 255, 255],
        _ => [255, 255, 0, 255],
    };
    let frame_pixels: Vec<u8> = (0..(W * H) as usize).flat_map(|_| pixel_color).collect();
    let frame = Arc::new(frame_pixels);

    let pts_state = Arc::new(Mutex::new(gst::ClockTime::ZERO));
    let appsrc_for_thread = appsrc;
    thread::Builder::new()
        .name(format!("slot-{idx}-pump"))
        .spawn(move || {
            let frame_dur = gst::ClockTime::from_mseconds(33);
            loop {
                let pts = {
                    let mut p = pts_state.lock().unwrap();
                    let v = *p;
                    *p += frame_dur;
                    v
                };

                let Ok(mut buffer) = gst::Buffer::with_size(frame.len()) else {
                    return;
                };
                {
                    let bm = buffer.get_mut().unwrap();
                    if bm.copy_from_slice(0, &frame).is_err() {
                        return;
                    }
                    bm.set_pts(pts);
                    bm.set_duration(frame_dur);
                }
                if appsrc_for_thread.push_buffer(buffer).is_err() {
                    println!("slot {idx}: push failed, exiting");
                    return;
                }
            }
        })
        .expect("spawn pump");
}
