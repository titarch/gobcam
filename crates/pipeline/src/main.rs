use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    gobcam_pipeline::run(&gobcam_pipeline::Cli::parse())
}
