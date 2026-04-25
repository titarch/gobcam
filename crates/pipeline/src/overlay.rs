//! Build a `gst::Bin` that produces an emoji video stream from a [`Source`].
//! Both static and animated sources expose a `videoconvert ! queue ! ghost-src`
//! tail so the compositor sees identical caps on every overlay branch.
//!
//! - Static  ─ `appsrc` (single buffer + EOS) → `imagefreeze` repeats forever.
//! - Animated ─ `appsrc` driven by a frame-pump thread cycling APNG frames.

use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{self as gst_app, AppSrc};
use tracing::{debug, trace, warn};

use crate::assets::{AnimatedFrames, Source};

pub(crate) struct Overlay {
    pub bin: gst::Bin,
}

impl Overlay {
    pub(crate) fn build(source: &Source, name: &str) -> Result<Self> {
        match source {
            Source::StaticRaster(img) => Self::build_static(img, name),
            Source::Animated(frames) => Self::build_animated(frames.clone(), name),
        }
    }

    fn build_static(img: &image::RgbaImage, name: &str) -> Result<Self> {
        let (w, h) = img.dimensions();
        let bin = gst::Bin::with_name(name);

        let appsrc = AppSrc::builder()
            .name("src")
            .caps(&rgba_caps(w, h))
            .format(gst::Format::Time)
            .is_live(false)
            .stream_type(gst_app::AppStreamType::Stream)
            .build();
        let imagefreeze = make("imagefreeze")?;
        let convert = make("videoconvert")?;
        let queue = make("queue")?;

        bin.add_many([appsrc.upcast_ref(), &imagefreeze, &convert, &queue])?;
        gst::Element::link_many([appsrc.upcast_ref(), &imagefreeze, &convert, &queue])?;

        // Push exactly one buffer + EOS; `imagefreeze` repeats it indefinitely
        // downstream and ignores the upstream EOS.
        let mut buffer = gst::Buffer::with_size(img.as_raw().len())
            .context("allocating static overlay buffer")?;
        {
            let buf_mut = buffer.get_mut().expect("fresh buffer is unique");
            buf_mut
                .copy_from_slice(0, img.as_raw())
                .map_err(|n| anyhow::anyhow!("short copy at offset {n} into gst buffer"))?;
        }
        appsrc.push_buffer(buffer)?;
        appsrc.end_of_stream()?;

        ghost_src_pad(&bin, &queue)?;
        Ok(Self { bin })
    }

    fn build_animated(frames: Arc<AnimatedFrames>, name: &str) -> Result<Self> {
        let (w, h) = frames.dimensions();
        let bin = gst::Bin::with_name(name);

        let appsrc = AppSrc::builder()
            .name("src")
            .caps(&rgba_caps(w, h))
            .format(gst::Format::Time)
            .is_live(true)
            .block(true)
            .stream_type(gst_app::AppStreamType::Stream)
            .build();
        appsrc.set_property("max-buffers", 2_u64);

        let convert = make("videoconvert")?;
        let queue = make("queue")?;

        bin.add_many([appsrc.upcast_ref(), &convert, &queue])?;
        gst::Element::link_many([appsrc.upcast_ref(), &convert, &queue])?;

        spawn_frame_pump(&appsrc, frames);

        ghost_src_pad(&bin, &queue)?;
        Ok(Self { bin })
    }
}

fn rgba_caps(width: u32, height: u32) -> gst::Caps {
    gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .field("width", i32::try_from(width).unwrap_or(i32::MAX))
        .field("height", i32::try_from(height).unwrap_or(i32::MAX))
        .field("framerate", gst::Fraction::new(30, 1))
        .build()
}

fn make(factory: &str) -> Result<gst::Element> {
    gst::ElementFactory::make(factory)
        .build()
        .with_context(|| format!("creating element {factory}"))
}

fn ghost_src_pad(bin: &gst::Bin, last: &gst::Element) -> Result<()> {
    let src_pad = last.static_pad("src").context("element has no src pad")?;
    let ghost = gst::GhostPad::with_target(&src_pad).context("ghost pad")?;
    ghost.set_active(true)?;
    bin.add_pad(&ghost)?;
    Ok(())
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
                trace!(idx, ?pts, "pushed frame");
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
