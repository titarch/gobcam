//! Gobcam pipeline daemon: v4l2 capture → optional emoji compositor →
//! v4l2loopback sink. See `docs/v4l2sink-thread-safety.md` for the
//! `firewall` module's rationale.

mod animation;
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
use std::time::Duration;

use anyhow::{Context, Result};
use gobcam_protocol::AnimationConfig;
use tracing::info;

use crate::animation::AnimationStore;
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
    let (pipeline, slots) = pipeline::build(cli)?;
    let preview_server = if cli.preview {
        Some(preview::install(&pipeline).context("installing preview MJPEG server")?)
    } else {
        None
    };

    let downloader = Arc::new(Downloader::new()?);
    let progress = bootstrap::spawn(&catalog, &cache, &downloader);
    info!(
        catalog_size = catalog.len(),
        "catalog loaded; preview prefetch started"
    );

    let library: Arc<dyn Library> = Arc::new(FluentLibrary::new(cache, catalog, downloader));
    let animation_store = AnimationStore::new(AnimationConfig::default());
    let canvas = (
        i32::try_from(cli.width).unwrap_or(1280),
        i32::try_from(cli.height).unwrap_or(720),
    );
    let reactor = Arc::new(Reactor::new(slots, library, animation_store, canvas));

    if let Some(emoji_id) = &cli.overlay {
        // ZERO duration = always-on; slot stays armed until shutdown.
        reactor.activate(emoji_id, Some(Duration::ZERO))?;
    }

    if cli.triggers_stdin {
        reactions::spawn_stdin_reader(Arc::clone(&reactor));
    } else if cli.exit_on_stdin_eof {
        spawn_stdin_watchdog();
    }

    let preview_url = preview_server.as_ref().map(|p| p.url.clone());
    let _socket_guard = match &cli.socket {
        Some(path) => Some(ipc::serve(
            DispatchCtx {
                reactor: Arc::clone(&reactor),
                progress,
                output_device: cli.output.clone(),
                preview_url,
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

/// UI-supervisor pattern: pipe-EOF on stdin triggers daemon shutdown when the UI dies.
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
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "stdin read error — shutting down");
                        std::process::exit(0);
                    }
                }
            }
        })
        .expect("spawning stdin watchdog");
}
