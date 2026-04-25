use anyhow::{Result, anyhow};
use gstreamer::{self as gst, prelude::*};
use tracing::{info, warn};

/// Drive the pipeline to PLAYING, pump its bus until EOS, and tear down.
/// Ctrl-C triggers a graceful EOS so downstream consumers see a clean stream end.
pub(crate) fn run(pipeline: &gst::Pipeline) -> Result<()> {
    install_sigint_handler(pipeline)?;
    pipeline.set_state(gst::State::Playing)?;
    let bus = pipeline.bus().expect("pipeline has a bus");

    let outcome = pump_bus(&bus);
    let _ = pipeline.set_state(gst::State::Null);
    outcome
}

fn install_sigint_handler(pipeline: &gst::Pipeline) -> Result<()> {
    let pipe = pipeline.clone();
    ctrlc::set_handler(move || {
        info!("interrupt received; sending EOS");
        if !pipe.send_event(gst::event::Eos::new()) {
            warn!("failed to enqueue EOS event");
        }
    })?;
    Ok(())
}

fn pump_bus(bus: &gst::Bus) -> Result<()> {
    use gst::MessageView;
    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        match msg.view() {
            MessageView::Eos(..) => {
                info!("end of stream");
                return Ok(());
            }
            MessageView::Error(err) => {
                let src = err
                    .src()
                    .map_or_else(|| "?".to_string(), |s| s.path_string().to_string());
                return Err(anyhow!("{src}: {} ({:?})", err.error(), err.debug()));
            }
            MessageView::Warning(w) => {
                let src = w
                    .src()
                    .map_or_else(|| "?".to_string(), |s| s.path_string().to_string());
                warn!("{src}: {} ({:?})", w.error(), w.debug());
            }
            _ => {}
        }
    }
    Ok(())
}
