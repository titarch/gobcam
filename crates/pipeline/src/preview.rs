//! Preview branch: pulls JPEG frames from a `GStreamer` `appsink` and
//! broadcasts them as `multipart/x-mixed-replace` (MJPEG-over-HTTP).
//! Bound on `127.0.0.1:0`; the UI fetches the URL via `Command::PreviewUrl`.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{self as gst_app, AppSink};
use tracing::{debug, warn};

pub(crate) struct PreviewServer {
    pub url: String,
}

/// Wire the pipeline's `preview` appsink to a localhost MJPEG broadcast.
/// Failed writes silently prune the client.
pub(crate) fn install(pipeline: &gst::Pipeline) -> Result<PreviewServer> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .context("binding preview listener on 127.0.0.1:0")?;
    let port = listener
        .local_addr()
        .context("reading bound preview port")?
        .port();
    let url = format!("http://127.0.0.1:{port}/preview.mjpg");

    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));

    let accept_clients = Arc::clone(&clients);
    thread::Builder::new()
        .name("preview-accept".into())
        .spawn(move || accept_loop(&listener, &accept_clients))
        .context("spawning preview accept thread")?;

    let sink = pipeline
        .by_name("preview")
        .context("preview branch's appsink not found")?
        .dynamic_cast::<AppSink>()
        .map_err(|_| anyhow::anyhow!("element named 'preview' is not an appsink"))?;
    let broadcast_clients = Arc::clone(&clients);
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                broadcast(&broadcast_clients, map.as_slice());
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    debug!(%url, "preview MJPEG server bound");
    Ok(PreviewServer { url })
}

fn accept_loop(listener: &TcpListener, clients: &Arc<Mutex<Vec<TcpStream>>>) {
    for incoming in listener.incoming() {
        let mut stream = match incoming {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "preview accept failed; listener exiting");
                return;
            }
        };
        // Drain the HTTP request without parsing; any GET streams the feed.
        let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);

        let headers = "HTTP/1.0 200 OK\r\n\
                       Content-Type: multipart/x-mixed-replace; boundary=frame\r\n\
                       Cache-Control: no-cache, no-store, private\r\n\
                       Pragma: no-cache\r\n\
                       Connection: close\r\n\r\n";
        if stream.write_all(headers.as_bytes()).is_err() {
            continue;
        }
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(None);
        clients
            .lock()
            .expect("preview clients poisoned")
            .push(stream);
    }
}

fn broadcast(clients: &Mutex<Vec<TcpStream>>, jpeg: &[u8]) {
    let part_header = format!(
        "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        jpeg.len()
    );
    let mut guard = clients.lock().expect("preview clients poisoned");
    guard.retain_mut(|stream| {
        stream.write_all(part_header.as_bytes()).is_ok()
            && stream.write_all(jpeg).is_ok()
            && stream.write_all(b"\r\n").is_ok()
    });
}
