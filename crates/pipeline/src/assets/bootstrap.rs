//! Background prefetch of every static 3D preview at daemon start.
//!
//! Spawns a fixed-size pool of worker threads that walk
//! [`Catalog::entries`] and call [`Downloader::ensure`] for each
//! emoji's preview path. Idempotent: workers skip files already on
//! disk, so a restart after partial completion finishes the rest.
//! Progress is exposed through [`SyncProgress`] and surfaced by the
//! IPC `sync_status` command.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::assets::cache::{Base, CacheRoot, Downloader};
use crate::assets::catalog::{Catalog, CatalogEntry};

const WORKER_COUNT: usize = 8;

#[derive(Debug, Default)]
pub(crate) struct SyncProgress {
    fetched: AtomicU32,
    total: AtomicU32,
    complete: AtomicBool,
}

impl SyncProgress {
    pub(crate) fn snapshot(&self) -> (u32, u32, bool) {
        (
            self.fetched.load(Ordering::Acquire),
            self.total.load(Ordering::Acquire),
            self.complete.load(Ordering::Acquire),
        )
    }
}

/// Kick off the predownload pool. Returns immediately; workers run
/// in the background and finish when the queue empties.
pub(crate) fn spawn(
    catalog: &Arc<Catalog>,
    cache: &CacheRoot,
    downloader: &Arc<Downloader>,
) -> Arc<SyncProgress> {
    let progress = Arc::new(SyncProgress::default());
    let total_u32 = u32::try_from(catalog.len()).unwrap_or(u32::MAX);
    progress.total.store(total_u32, Ordering::Release);

    let queue = Arc::new(Mutex::new(0_usize));
    for worker_id in 0..WORKER_COUNT {
        let catalog = Arc::clone(catalog);
        let cache = cache.clone();
        let downloader = Arc::clone(downloader);
        let progress = Arc::clone(&progress);
        let queue = Arc::clone(&queue);
        thread::Builder::new()
            .name(format!("preview-prefetch-{worker_id}"))
            .spawn(move || worker_loop(&catalog, &cache, &downloader, &progress, &queue))
            .expect("spawning preview-prefetch worker");
    }

    // A finalizer thread flips `complete` once `fetched == total`.
    let finalizer_progress = Arc::clone(&progress);
    let total = catalog.len();
    thread::Builder::new()
        .name("preview-prefetch-done".into())
        .spawn(move || {
            loop {
                if finalizer_progress.fetched.load(Ordering::Acquire) >= total_u32 {
                    finalizer_progress.complete.store(true, Ordering::Release);
                    tracing::info!(total, "preview prefetch complete");
                    return;
                }
                thread::sleep(std::time::Duration::from_millis(250));
            }
        })
        .expect("spawning prefetch-done thread");

    progress
}

fn worker_loop(
    catalog: &Arc<Catalog>,
    cache: &CacheRoot,
    downloader: &Arc<Downloader>,
    progress: &Arc<SyncProgress>,
    queue: &Arc<Mutex<usize>>,
) {
    loop {
        let idx = {
            let mut next = queue.lock().expect("queue poisoned");
            let i = *next;
            *next += 1;
            i
        };
        let Some(entry) = catalog.entries().get(idx) else {
            return;
        };
        if let Err(e) = fetch_preview(entry, cache, downloader) {
            // Failure is logged but the bootstrap doesn't abort — the
            // single emoji simply has no preview until the user
            // triggers it (which forces a synchronous retry).
            tracing::warn!(id = %entry.id, error = %e, "preview prefetch failed");
        }
        progress.fetched.fetch_add(1, Ordering::AcqRel);
    }
}

fn fetch_preview(
    entry: &CatalogEntry,
    cache: &CacheRoot,
    downloader: &Arc<Downloader>,
) -> anyhow::Result<()> {
    if entry.static_path.is_empty() {
        return Ok(());
    }
    let dest = cache.preview_path(&crate::assets::EmojiId::new(entry.id.clone()));
    downloader.ensure(&dest, Base::Static, &entry.static_path)
}
