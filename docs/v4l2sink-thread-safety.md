# Thread-safety bug in `gst-plugins-good`'s v4l2 caps probing

A post-mortem on a heap-corruption bug that surfaces when an N-input
GStreamer compositor feeds a `v4l2sink`. The workaround is documented
in [`crates/pipeline/src/firewall.rs`][firewall]; this page explains
why it's there.

## Symptom

A pipeline of the shape `compositor → videoconvert → v4l2sink` aborts
during preroll with one of:

- `free(): invalid pointer`
- `double free or corruption (out)`
- silent stall in PAUSED, `gst-launch-1.0` never reaches PLAYING

The crash is intermittent and depends on the compositor's input count.
With a single sink pad it almost never reproduces; with N ≥ 2 it
reproduces within a few seconds; at N = 16+ it's almost immediate.

Replacing `v4l2sink` with `autovideosink` or `fakesink` makes the
problem vanish entirely. Replacing `v4l2src` with `videotestsrc` also
helps but does not eliminate it — the v4l2 boundary on the sink side
is what triggers the bug.

## Root cause

`gst_v4l2_object_probe_caps` in `gst-plugins-good`'s `gstv4l2object.c`
maintains an internal `GSList` of probed caps without a lock. When
multiple upstream tasks query `v4l2sink.sink`'s caps concurrently —
which the compositor's aggregator does, once per sink pad on
preroll — they race on the same list and free overlapping nodes.

The race is not on the GStreamer-level pad locking (that's correct);
it's inside the v4l2 plugin's caps-cache implementation. The crash
manifests as a glibc heap detection trap because the freed pointer
gets reused before the plugin notices.

## Repro

A minimal Rust reproducer is at [`crates/pipeline/examples/repro_v4l2_slots.rs`][repro];
see `cargo run --release --example repro_v4l2_slots`. The same shape
in shell:

```sh
gst-launch-1.0 -v \
    videotestsrc ! video/x-raw,width=1280,height=720,framerate=30/1 \
                 ! videoconvert ! mix.sink_0 \
    videotestsrc pattern=ball ! video/x-raw,width=256,height=256 \
                 ! videoconvert ! mix.sink_1 \
    videotestsrc pattern=snow ! video/x-raw,width=256,height=256 \
                 ! videoconvert ! mix.sink_2 \
    compositor name=mix ! videoconvert ! v4l2sink device=/dev/video10
```

(Requires a `v4l2loopback` device at `/dev/video10`.)

Tested on:

- GStreamer 1.28.2 (Arch), `gstreamer-rs` 0.25
- GStreamer 1.24.x (Debian Trixie)

Both reproduce; the precise stack trace varies but the failure mode
is the same.

## Workaround — caps-query firewall

The fix is to never let the streaming-thread caps queries reach
`v4l2sink`'s probe path. `firewall::install` does this in three steps:

1. At pipeline build, briefly transition a temporary `v4l2sink` to
   `READY` on the main thread. This drives one safe call into
   `gst_v4l2_object_probe_caps` while no upstream tasks are running.
2. Cache the resulting caps as a single fixated set
   (`YUY2/<width>×<height>/<fps>`).
3. Attach a pad probe on the real `v4l2sink.sink` that intercepts
   `CAPS` and `ACCEPT_CAPS` queries and answers them from the cache,
   so subsequent queries from streaming threads never invoke the
   racy code path.

The cache deliberately does **not** intersect with whatever the device
currently reports — `v4l2loopback` only advertises the format the
last writer set, and intersecting would reject any mode change.

A working reproducer of the workaround sits next to the bug
reproducer: [`crates/pipeline/examples/repro_v4l2_slots_probe.rs`][probe].

## Upstream

The upstream GStreamer report is still pending. This document and the
two reproducers are the current notes for that report. If you hit this
in another project, the same firewall pattern works: install a
single-pad probe between your compositor and `v4l2sink` that answers
caps queries from a fixed cache.

[firewall]: ../crates/pipeline/src/firewall.rs
[repro]: ../crates/pipeline/examples/repro_v4l2_slots.rs
[probe]: ../crates/pipeline/examples/repro_v4l2_slots_probe.rs
