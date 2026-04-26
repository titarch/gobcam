//! Gobcam pipeline daemon: a `GStreamer` graph that pulls from a v4l2
//! capture device, optionally composites emoji reactions via a pool of
//! pre-allocated compositor slots, and pushes the result to a v4l2loopback
//! sink. The `firewall` module works around a thread-safety bug in
//! `gst-plugins-good`'s `gst_v4l2_object_probe_caps`; without it the
//! pipeline aborts during preroll. See `docs/step3-debug-report.md`.

mod assets;
mod cli;
mod effects;
mod firewall;
mod inputs;
mod ipc;
mod pipeline;
mod preview;
mod profile;
mod reactions;
mod runner;
mod slots;

pub use cli::Cli;

use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use crate::assets::Library;
use crate::assets::bootstrap;
use crate::assets::cache::{CacheRoot, Downloader};
use crate::assets::catalog::Catalog;
use crate::assets::fluent::FluentLibrary;
use crate::ipc::DispatchCtx;
use crate::reactions::Reactor;

pub fn run(cli: &Cli) -> Result<()> {
    init_tracing();
    if let Some(path) = &cli.profile_log {
        profile::init(path)?;
        info!(path = %path.display(), "profile logging enabled");
    }
    gstreamer::init()?;
    info!(
        input = %cli.input.display(),
        output = %cli.output.display(),
        overlay = ?cli.overlay,
        triggers_stdin = cli.triggers_stdin,
        socket = ?cli.socket,
        cache_root = ?cli.cache_root,
        "starting pipeline"
    );

    let catalog = Arc::new(Catalog::load_bundled()?);
    let cache = match &cli.cache_root {
        Some(path) => CacheRoot::with_path(path.clone())?,
        None => CacheRoot::resolve_default()?,
    };
    let (pipeline, slots) = pipeline::build(cli, &cache)?;

    let downloader = Arc::new(Downloader::new()?);
    let progress = bootstrap::spawn(&catalog, &cache, &downloader);
    info!(
        catalog_size = catalog.len(),
        "catalog loaded; preview prefetch started"
    );

    let library: Arc<dyn Library> = Arc::new(FluentLibrary::new(cache, catalog, downloader));
    let reactor = Arc::new(Reactor::new(slots, library));

    if let Some(emoji_id) = &cli.overlay {
        // Always-on overlay = a reaction with no deadline.
        reactor.activate(emoji_id, None)?;
    }

    if cli.triggers_stdin {
        reactions::spawn_stdin_reader(Arc::clone(&reactor));
    } else if cli.exit_on_stdin_eof {
        spawn_stdin_watchdog();
    }

    let _socket_guard = match &cli.socket {
        Some(path) => Some(ipc::serve(
            DispatchCtx {
                reactor: Arc::clone(&reactor),
                progress,
                output_device: cli.output.clone(),
            },
            path.clone(),
        )?),
        None => None,
    };

    runner::run(&pipeline)
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}

/// Spawn a thread that drains stdin and `process::exit`s when it
/// reaches EOF. Used by the UI's daemon-supervisor: the UI keeps an
/// open pipe to our stdin and the kernel closes it whenever the UI
/// dies (crash, SIGKILL, normal exit) — turning that into a clean
/// daemon shutdown without needing process-level signal handling.
fn spawn_stdin_watchdog() {
    use std::io::Read;
    std::thread::Builder::new()
        .name("stdin-watchdog".into())
        .spawn(|| {
            let mut stdin = std::io::stdin().lock();
            let mut buf = [0u8; 1024];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => {
                        info!("stdin EOF — shutting down");
                        std::process::exit(0);
                    }
                    Ok(_) => {} // ignore stray bytes
                    Err(e) => {
                        tracing::warn!(error = %e, "stdin read error — shutting down");
                        std::process::exit(0);
                    }
                }
            }
        })
        .expect("spawning stdin watchdog");
}
