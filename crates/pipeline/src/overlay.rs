//! Build the elements that produce an emoji video stream from a [`Source`].
//! Returned as a flat list of elements + the terminal element's src pad —
//! no `gst::Bin` wrapper, no ghost pads. On dynamic add to a running
//! pipeline the bin/ghost-pad combination races pad linkage and surfaces as
//! `not-linked` stream errors; flat element addition avoids that entirely.
//!
//! - Static  ─ `appsrc` (single buffer + EOS) → `imagefreeze` → `videoconvert` → `queue`
//! - Animated ─ `appsrc` (frame-pump thread) → `videoconvert` → `queue`

use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use gstreamer::{self as gst, glib, prelude::*};
use gstreamer_app::{self as gst_app, AppSrc};
use tracing::{debug, warn};

use crate::assets::{AnimatedFrames, Source};

/// Flat representation of an overlay subgraph: a list of elements (in chain
/// order) and the src pad of the terminal element, ready to be linked into a
/// compositor sink pad.
pub(crate) struct Overlay {
    pub elements: Vec<gst::Element>,
    pub src_pad: gst::Pad,
}

impl Overlay {
    pub(crate) fn build(source: &Source, name_prefix: &str) -> Result<Self> {
        match source {
            Source::StaticRaster(img) => build_static(img, name_prefix),
            Source::Animated(frames) => build_animated(frames.clone(), name_prefix),
        }
    }
}

fn build_static(img: &image::RgbaImage, name_prefix: &str) -> Result<Overlay> {
    let (w, h) = img.dimensions();

    let appsrc = AppSrc::builder()
        .name(format!("{name_prefix}-src"))
        .caps(&rgba_caps(w, h))
        .format(gst::Format::Time)
        .is_live(false)
        .stream_type(gst_app::AppStreamType::Stream)
        .build();
    let imagefreeze = named("imagefreeze", format!("{name_prefix}-freeze"))?;
    let convert = named("videoconvert", format!("{name_prefix}-convert"))?;
    let queue = named("queue", format!("{name_prefix}-queue"))?;

    let mut buffer =
        gst::Buffer::with_size(img.as_raw().len()).context("allocating static overlay buffer")?;
    {
        let buf_mut = buffer.get_mut().expect("fresh buffer is unique");
        buf_mut
            .copy_from_slice(0, img.as_raw())
            .map_err(|n| anyhow::anyhow!("short copy at offset {n} into gst buffer"))?;
    }
    appsrc.push_buffer(buffer)?;
    appsrc.end_of_stream()?;

    let src_pad = queue.static_pad("src").context("queue missing src pad")?;
    Ok(Overlay {
        elements: vec![appsrc.upcast(), imagefreeze, convert, queue],
        src_pad,
    })
}

fn build_animated(frames: Arc<AnimatedFrames>, name_prefix: &str) -> Result<Overlay> {
    let (w, h) = frames.dimensions();

    let appsrc = AppSrc::builder()
        .name(format!("{name_prefix}-src"))
        .caps(&rgba_caps(w, h))
        .format(gst::Format::Time)
        .is_live(true)
        .block(true)
        .stream_type(gst_app::AppStreamType::Stream)
        .build();
    appsrc.set_property("max-buffers", 2_u64);
    let convert = named("videoconvert", format!("{name_prefix}-convert"))?;
    let queue = named("queue", format!("{name_prefix}-queue"))?;

    spawn_frame_pump(&appsrc, frames);

    let src_pad = queue.static_pad("src").context("queue missing src pad")?;
    Ok(Overlay {
        elements: vec![appsrc.upcast(), convert, queue],
        src_pad,
    })
}

fn rgba_caps(width: u32, height: u32) -> gst::Caps {
    gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .field("width", i32::try_from(width).unwrap_or(i32::MAX))
        .field("height", i32::try_from(height).unwrap_or(i32::MAX))
        .field("framerate", gst::Fraction::new(30, 1))
        .build()
}

fn named(factory: &str, name: impl Into<glib::GString>) -> Result<gst::Element> {
    gst::ElementFactory::make(factory)
        .name(name)
        .build()
        .with_context(|| format!("creating element {factory}"))
}

fn spawn_frame_pump(appsrc: &AppSrc, frames: Arc<AnimatedFrames>) {
    let appsrc = appsrc.clone();
    thread::Builder::new()
        .name(format!("overlay-pump-{}", appsrc.name()))
        .spawn(move || pump(&appsrc, &frames))
        .expect("spawn frame pump");
}

fn pump(appsrc: &AppSrc, frames: &AnimatedFrames) {
    let mut pts = gst::ClockTime::ZERO;
    let mut idx = 0;
    let total = frames.frames.len();
    debug!(frames = total, "frame pump started");
    loop {
        let frame = &frames.frames[idx];
        let raw = frame.rgba.as_raw();
        let dur = gst::ClockTime::from_nseconds(
            u64::try_from(frame.delay.as_nanos()).unwrap_or(u64::MAX),
        );

        let mut buffer = match gst::Buffer::with_size(raw.len()) {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "failed to allocate overlay buffer");
                return;
            }
        };
        {
            let buf_mut = buffer.get_mut().expect("fresh buffer is unique");
            if buf_mut.copy_from_slice(0, raw).is_err() {
                warn!("failed to copy frame data");
                return;
            }
            buf_mut.set_pts(pts);
            buf_mut.set_duration(dur);
        }
        match appsrc.push_buffer(buffer) {
            Ok(_) => {
                pts += dur;
                idx = (idx + 1) % total;
            }
            Err(gst::FlowError::Flushing | gst::FlowError::Eos) => {
                debug!("frame pump exiting (flow stopped)");
                return;
            }
            Err(other) => {
                warn!(error = ?other, "frame pump push_buffer failed");
                return;
            }
        }
    }
}
