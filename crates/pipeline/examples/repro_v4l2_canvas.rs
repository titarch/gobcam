//! Playground: single overlay canvas — ONE `appsrc` at 1280×720 RGBA fed by
//! ONE pump. The compositor only ever sees two inputs (camera + canvas).
//! Per the external review, this sidesteps the multi-`appsrc` + v4l2
//! interaction that crashes `repro_v4l2_slots`.
//!
//! For v1 the canvas is just a static red square at the center to confirm
//! the topology works end-to-end. The application-side compositor (the
//! Rust "render all active reactions into this RGBA buffer" logic) lives
//! in the daemon; this example only validates the pipeline shape.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{self as gst_app, AppSrc};

const W: i32 = 1280;
const H: i32 = 720;

fn main() -> Result<()> {
    gst::init()?;

    let pipeline = gst::parse::launch(
        "v4l2src device=/dev/video0 ! \
         video/x-raw,width=1280,height=720,framerate=30/1 ! \
         queue ! videoconvert ! \
         compositor name=mix background=black ! \
         videoconvert ! \
         video/x-raw,format=YUY2,width=1280,height=720,framerate=30/1 ! \
         identity drop-allocation=true ! \
         v4l2sink device=/dev/video10 sync=false",
    )?
    .downcast::<gst::Pipeline>()
    .map_err(|_| anyhow::anyhow!("not a pipeline"))?;

    let compositor = pipeline.by_name("mix").context("no compositor")?;

    // Build the single overlay canvas branch.
    let appsrc = AppSrc::builder()
        .name("canvas-src")
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
        .name("canvas-cv")
        .build()?;
    let queue = gst::ElementFactory::make("queue")
        .name("canvas-q")
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

    // Start the pump BEFORE state transition (matches the pattern that
    // works in repro_appsrc_slots with videotestsrc + autovideosink).
    spawn_pump(appsrc);

    println!("starting pipeline (single 1280x720 RGBA overlay canvas)");
    pipeline.set_state(gst::State::Playing)?;
    let _ = pipeline.state(gst::ClockTime::from_seconds(3));
    println!("pipeline state: {:?}", pipeline.current_state());

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
            _ => {}
        }
    }
    pipeline.set_state(gst::State::Null)?;
    Ok(())
}

fn spawn_pump(appsrc: AppSrc) {
    // Build a 1280x720 RGBA buffer with a transparent background and a
    // 256x256 opaque red square at the center, to validate the canvas shape.
    let mut canvas = vec![0_u8; (W * H * 4) as usize];
    let cx = (W / 2) - 128;
    let cy = (H / 2) - 128;
    for y in cy..cy + 256 {
        for x in cx..cx + 256 {
            let i = ((y * W + x) * 4) as usize;
            canvas[i] = 255; // R
            canvas[i + 1] = 0; // G
            canvas[i + 2] = 0; // B
            canvas[i + 3] = 200; // A (mostly opaque)
        }
    }
    let canvas = Arc::new(canvas);
    let pts_state = Arc::new(Mutex::new(gst::ClockTime::ZERO));
    let frame_dur = gst::ClockTime::from_mseconds(33);

    thread::Builder::new()
        .name("canvas-pump".into())
        .spawn(move || {
            loop {
                let pts = {
                    let mut p = pts_state.lock().unwrap();
                    let v = *p;
                    *p += frame_dur;
                    v
                };
                let Ok(mut buffer) = gst::Buffer::with_size(canvas.len()) else {
                    return;
                };
                {
                    let bm = buffer.get_mut().unwrap();
                    if bm.copy_from_slice(0, &canvas).is_err() {
                        return;
                    }
                    bm.set_pts(pts);
                    bm.set_duration(frame_dur);
                }
                if appsrc.push_buffer(buffer).is_err() {
                    println!("canvas pump: push failed, exiting");
                    return;
                }
            }
        })
        .expect("spawn canvas pump");
}
