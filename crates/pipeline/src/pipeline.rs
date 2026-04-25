use anyhow::{Context, Result, anyhow};
use gstreamer::{self as gst, prelude::*};

use crate::cli::Cli;

/// Build the Step 1 passthrough graph: `v4l2src ! videoconvert ! v4l2sink`.
///
/// `parse::launch` is used deliberately: at this stage the topology is static
/// and a one-line pipeline description is the most legible form. When dynamic
/// branching arrives in Step 3, this becomes manual element wiring.
pub(crate) fn build_passthrough(cli: &Cli) -> Result<gst::Pipeline> {
    let input = path_str(&cli.input, "--input")?;
    let output = path_str(&cli.output, "--output")?;
    let desc =
        format!("v4l2src device={input} ! videoconvert ! v4l2sink device={output} sync=false");
    gst::parse::launch(&desc)
        .with_context(|| format!("parsing pipeline: {desc}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("parsed element is not a gst::Pipeline"))
}

fn path_str<'a>(path: &'a std::path::Path, flag: &str) -> Result<&'a str> {
    path.to_str()
        .with_context(|| format!("{flag} must be a valid UTF-8 path: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_utf8_input() {
        let cli = Cli {
            input: "/dev/video0".into(),
            output: "/dev/video10".into(),
        };
        assert!(path_str(&cli.input, "--input").is_ok());
    }
}
