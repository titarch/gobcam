//! Integration-level smoke test: confirms gstreamer-rs links against the host
//! libraries and that pipeline descriptors parse and reach READY without real
//! v4l2 devices in the loop.

use gstreamer::prelude::*;

#[test]
fn gstreamer_init_succeeds() {
    gstreamer::init().expect("gstreamer init");
}

#[test]
fn pipeline_with_compositor_reaches_ready() {
    gstreamer::init().expect("gstreamer init");
    // Substitute videotestsrc + fakesink for v4l2{src,sink} so the test runs
    // without real hardware. The compositor topology mirrors the production graph.
    let desc = "videotestsrc num-buffers=1 ! videoconvert ! \
                compositor name=mix background=black ! videoconvert ! fakesink";
    let pipeline = gstreamer::parse::launch(desc)
        .expect("descriptor parses")
        .downcast::<gstreamer::Pipeline>()
        .expect("is a Pipeline");
    pipeline.set_state(gstreamer::State::Ready).expect("ready");
    pipeline.set_state(gstreamer::State::Null).expect("null");
}
