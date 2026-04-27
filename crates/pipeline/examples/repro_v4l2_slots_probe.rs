//! Playground: same N-slot v4l2 topology as `repro_v4l2_slots`, plus a
//! CAPS-query probe on `v4l2sink`'s sink pad that answers with fixed caps
//! and returns `Handled`. Tests the hypothesis that intercepting CAPS
//! queries before they reach `gst_v4l2_object_probe_caps` prevents the
//! concurrent-iteration race in `gstv4l2object.c`.

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
         videoconvert ! \
         video/x-raw,format=YUY2,width=1280,height=720,framerate=30/1 ! \
         identity drop-allocation=true ! \
         v4l2sink name=sink device=/dev/video10 sync=false",
    )?
    .downcast::<gst::Pipeline>()
    .map_err(|_| anyhow::anyhow!("not a pipeline"))?;

    let compositor = pipeline.by_name("mix").context("no compositor")?;
    let v4l2sink = pipeline.by_name("sink").context("no v4l2sink")?;

    // Caps-query firewall:
    //   1. Build a temporary, standalone v4l2sink and bring it to READY so it
    //      opens /dev/video10 and probes device-specific caps once,
    //      single-threaded. NULL-state caps would be the V4L2 plugin's
    //      template (every supported codec/format), which is too broad for
    //      compositor fixation.
    //   2. Intersect with our preferred output (YUY2 1280×720 30fps) so the
    //      probe answers with a single-format spec the compositor can fixate.
    //   3. Install QUERY_DOWNSTREAM probe handling both CAPS and ACCEPT_CAPS.
    //      Log empty intersections — those indicate negotiation mismatches.
    let firewall_caps = derive_firewall_caps()?;
    println!("firewall_caps: {firewall_caps}");

    let sink_pad = v4l2sink
        .static_pad("sink")
        .context("v4l2sink missing sink pad")?;
    sink_pad.add_probe(gst::PadProbeType::QUERY_DOWNSTREAM, move |_pad, info| {
        let Some(query) = info.query_mut() else {
            return gst::PadProbeReturn::Ok;
        };
        match query.view_mut() {
            gst::QueryViewMut::Caps(caps_q) => {
                let result = match caps_q.filter() {
                    Some(filter) => firewall_caps.intersect(filter),
                    None => firewall_caps.clone(),
                };
                if result.is_empty() {
                    eprintln!(
                        "CAPS firewall: empty intersection. filter={:?}",
                        caps_q.filter()
                    );
                }
                caps_q.set_result(&result);
                gst::PadProbeReturn::Handled
            }
            gst::QueryViewMut::AcceptCaps(q) => {
                let proposed = q.caps();
                let acceptable = proposed.can_intersect(&firewall_caps);
                q.set_result(acceptable);
                gst::PadProbeReturn::Handled
            }
            _ => gst::PadProbeReturn::Ok,
        }
    });

    for idx in 0..N_SLOTS {
        build_slot(&pipeline, &compositor, idx)?;
    }

    println!("starting pipeline ({N_SLOTS} appsrc slots, v4l2 + caps-query firewall)");
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

/// Bring a fresh v4l2sink to READY (which opens the device and probes
/// device caps), query its sink-pad caps single-threaded, then drop it.
/// Intersect with our preferred YUY2/1280×720/30fps. Returns the caps that
/// the firewall probe will replay for all CAPS queries.
fn derive_firewall_caps() -> Result<gst::Caps> {
    let probe_sink = gst::ElementFactory::make("v4l2sink")
        .property("device", "/dev/video10")
        .property("sync", false)
        .build()
        .context("creating probe v4l2sink")?;
    probe_sink
        .set_state(gst::State::Ready)
        .context("probe v4l2sink to READY")?;
    let (_change, _state, _) = probe_sink.state(gst::ClockTime::from_seconds(2));

    let pad = probe_sink
        .static_pad("sink")
        .context("probe sink missing pad")?;
    let device_caps = pad.query_caps(None);
    println!("device caps from READY-state v4l2sink: {device_caps}");

    probe_sink.set_state(gst::State::Null).ok();

    let preferred = gst::Caps::builder("video/x-raw")
        .field("format", "YUY2")
        .field("width", 1280_i32)
        .field("height", 720_i32)
        .field("framerate", gst::Fraction::new(30, 1))
        .build();
    let intersected = device_caps.intersect(&preferred);
    if intersected.is_empty() {
        anyhow::bail!(
            "device_caps ∩ preferred is empty. device_caps={device_caps:?}, preferred={preferred:?}"
        );
    }
    Ok(intersected)
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
