//! Integration-level smoke test: confirms gstreamer-rs links against the host
//! libraries and the passthrough descriptor parses. Does not open real devices.

use gobcam_pipeline::Cli;

#[test]
fn gstreamer_init_succeeds() {
    gstreamer::init().expect("gstreamer init");
}

#[test]
fn passthrough_descriptor_is_valid() {
    gstreamer::init().expect("gstreamer init");
    // Use a placeholder device path — parse::launch only validates syntax and
    // element factories, not device availability.
    let cli = Cli {
        input: "/dev/null".into(),
        output: "/dev/null".into(),
    };
    // The pipeline module is private; parsing the same descriptor here proves
    // the v4l2src and v4l2sink factories are registered on the test host.
    let desc = format!(
        "v4l2src device={} ! videoconvert ! v4l2sink device={} sync=false",
        cli.input.display(),
        cli.output.display(),
    );
    let _ = gstreamer::parse::launch(&desc).expect("descriptor parses");
}
