use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use image::AnimationDecoder;
use image::codecs::png::PngDecoder;

use super::{AnimatedFrame, AnimatedFrames};

pub(crate) fn load(path: &Path) -> Result<AnimatedFrames> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let decoder = PngDecoder::new(BufReader::new(file))
        .with_context(|| format!("not a PNG: {}", path.display()))?
        .apng()
        .with_context(|| format!("not an animated PNG: {}", path.display()))?;

    let mut frames = Vec::new();
    for frame in decoder.into_frames() {
        let frame = frame.context("decoding APNG frame")?;
        let delay = delay_to_duration(frame.delay());
        frames.push(AnimatedFrame {
            rgba: Arc::new(frame.into_buffer()),
            delay,
        });
    }
    if frames.is_empty() {
        anyhow::bail!("APNG had zero frames: {}", path.display());
    }
    Ok(AnimatedFrames::new(frames))
}

fn delay_to_duration(delay: image::Delay) -> Duration {
    let (num, den) = delay.numer_denom_ms();
    if den == 0 {
        return Duration::from_millis(100);
    }
    Duration::from_micros(u64::from(num) * 1_000 / u64::from(den))
}
