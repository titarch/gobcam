//! Playground: matches the daemon's exact topology — v4l2src base, v4l2sink
//! output, N appsrc slots with pumps. Isolates whether the preroll issue is
//! specific to v4l2 + appsrc combination.
//!
//! Run with `cargo run --example pg_v4l2_slots`. Reads /dev/video0 and
//! writes /dev/video10. Reset the loopback first via `just reset-loopback`.

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
        "v4l2src device=/dev/video0 ! \
         video/x-raw,width=1280,height=720,framerate=30/1 ! \
         queue ! videoconvert ! \
         compositor name=mix background=black ! \
         videoconvert ! v4l2sink device=/dev/video10 sync=false",
    )?
    .downcast::<gst::Pipeline>()
    .map_err(|_| anyhow::anyhow!("not a pipeline"))?;

    let compositor = pipeline.by_name("mix").context("no compositor")?;

    for idx in 0..N_SLOTS {
        build_slot(&pipeline, &compositor, idx)?;
    }

    println!("starting pipeline ({N_SLOTS} appsrc slots, v4l2src + v4l2sink)");
    pipeline.set_state(gst::State::Playing)?;

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

    sink_pad.set_property("xpos", (idx as i32 % 2) * 700);
    sink_pad.set_property("ypos", (idx as i32 / 2) * 400);
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
    let frame_pixels: Arc<Vec<u8>> =
        Arc::new((0..(W * H) as usize).flat_map(|_| pixel_color).collect());

    let pts_state = Arc::new(Mutex::new(gst::ClockTime::ZERO));
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
                let Ok(mut buffer) = gst::Buffer::with_size(frame_pixels.len()) else {
                    return;
                };
                {
                    let bm = buffer.get_mut().unwrap();
                    if bm.copy_from_slice(0, &frame_pixels).is_err() {
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
