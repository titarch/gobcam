# Gobcam Step 3 — debug report

**Status:** blocked. Two related failure modes, both reproducible. Looking for prior art / suggestions.

This document is self-contained — it covers what we're building, the two failure modes we hit, the minimal reproducers (committed in the repo), everything we've tried, and the open questions. Safe to share with other LLMs / Stack Overflow / GStreamer maintainers.

---

## TL;DR

We're building a Linux virtual webcam (`v4l2src` → `compositor` → `v4l2sink`) in Rust with `gstreamer-rs`. **Step 2 — always-on emoji overlay attached at startup — works.** **Step 3 — triggering reactions at runtime — does not**, and we've hit two distinct GStreamer failure modes when trying to make it work:

1. **Dynamic add of a new `compositor.request_pad_simple` + `appsrc` chain to a running pipeline** consistently surfaces `gst_base_src_loop: streaming stopped, reason not-linked (-1)` from the just-attached `appsrc`, across nine variations of link/state-sync ordering.
2. **Pre-allocating N `compositor` sink pads at build time** with per-pad `appsrc` pumps works perfectly in a `videotestsrc → autovideosink` test rig (committed playground), but the same code reproducibly aborts with `free(): invalid pointer` or stalls preroll when `v4l2src` is the base source and `v4l2sink` is the terminal sink.

We have shell `gst-launch-1.0` reproducers for the topology shape and Rust binary reproducers for the appsrc-driven slot variant. We **cannot** reproduce failure mode #2 without the `v4l2src + v4l2sink` boundaries.

---

## Environment

| | |
|---|---|
| OS | Arch Linux, kernel 6.19.14 |
| Architecture | x86_64 |
| GStreamer | 1.28.2 (system: gstreamer-1.0, gstreamer-app-1.0, gstreamer-base-1.0) |
| `gstreamer-rs` | 0.25 |
| `gstreamer-app` (rs) | 0.25 |
| Rust | 1.92.0 stable |
| Camera | Logitech BRIO via UVC, offers YUYV/MJPEG up to 1920×1080 |
| Loopback | `v4l2loopback-dkms` 0.13.x with `exclusive_caps=1`, `card_label="Gobcam"`, `video_nr=10` |

The repo is at <https://github.com/titarch/gobcam> (commit `970fa3a` is the working Step 2 baseline). The full source for the daemon plus the playground reproducers lives there.

---

## What we're building (relevant context)

A virtual webcam daemon. Pipeline (working state, Step 2):

```
v4l2src device=/dev/video0
    ! video/x-raw,width=1280,height=720,framerate=30/1
    ! queue
    ! videoconvert
    ! compositor name=mix background=black                  # sink_0 = camera
    ! videoconvert
    ! v4l2sink device=/dev/video10 sync=false
```

Overlays are emoji (Microsoft Fluent Emoji set: 256×256 RGBA, animated as APNG). Goal for Step 3: fire a reaction by writing `fire\n` to the daemon's stdin → emoji shows for ~3 seconds → disappears. Multiple reactions should stack via separate compositor sink pads.

For Step 2 we built a `gst::Bin` per overlay containing `appsrc → videoconvert → queue → ghost-src` and called `compositor.request_pad_simple("sink_%u")` once at startup. **It works for all 5 emoji.** The frame pump is a Rust thread that pushes RGBA buffers into the `appsrc` with monotonic PTS:

```rust
let appsrc = AppSrc::builder()
    .caps(&video_x_raw_rgba_256_256_30fps)
    .format(gst::Format::Time)
    .is_live(true)
    .block(true)
    .stream_type(AppStreamType::Stream)
    .build();
appsrc.set_property("max-buffers", 2_u64);
// elements added to pipeline, linked, ghost pad created, then runner sets PLAYING.
```

---

## Failure 1 — dynamic add to a running compositor

### Goal

While the pipeline is in `PLAYING`, attach a new overlay subgraph: build `appsrc → videoconvert → queue`, add to pipeline, request `sink_N` on `compositor`, link, sync state. After 3 seconds, detach.

### Symptom

Every attach attempt crashes the pipeline with the same bus error:

```
ERROR: from element /GstPipeline:pipeline0/GstAppSrc:react-fire-0-src:
    Internal data stream error.
Additional debug info:
    ../gstreamer/subprojects/gstreamer/libs/gst/base/gstbasesrc.c(3187):
        gst_base_src_loop ():
            /GstPipeline:pipeline0/GstAppSrc:react-fire-0-src:
        streaming stopped, reason not-linked (-1)
```

### Variations attempted

All independently, none worked:

1. Link first, then `sync_state_with_parent`.
2. `sync_state_with_parent` first, then link.
3. Sync state in **reverse** (downstream → upstream) so `appsrc` activates last.
4. Drop the `gst::Bin` wrapper; add the elements directly to the pipeline (no ghost pads).
5. Keep ghost pad, with explicit `set_active(true)` on it.
6. Without `set_active`, letting bin transition activate it.
7. Explicit `sink_pad.set_active(true)` on the compositor request pad before linking.
8. `is_live=true` vs `is_live=false` on the appsrc.
9. `AppSrc::set_callbacks(need_data)` instead of a thread pump.

### What we did not try

The dynamic-pipelines blog ([coaxion.net 2014](https://coaxion.net/blog/2014/01/gstreamer-dynamic-pipelines/)) describes the canonical pattern as: **install an IDLE pad probe on an EXISTING data-path pad** (e.g. the camera-side `videoconvert.src`), and in the probe callback link the new branch and bring it to PLAYING. Then return `GST_PAD_PROBE_REMOVE`. Our IDLE probes were on the new branch's src pad, only used for clean **detach**, not for attach.

This is the prime candidate for "did we just get it wrong?" — but we didn't validate it before pivoting.

---

## Failure 2 — pre-allocated slots + v4l2 boundaries

### Architecture

Allocate N permanent compositor sink pads at build time, each with its own `appsrc → videoconvert → queue → compositor.sink_N` chain. Idle slots have `alpha=0` and the pump pushes a transparent placeholder frame. Triggering = swap the pump's source pointer (`Mutex<Arc<AnimatedFrames>>`) and set `alpha=1`. After duration, swap back to transparent + `alpha=0`.

The graph never changes shape after `set_state(Playing)` — only properties and per-slot Mutex state mutate. This avoids dynamic-add entirely.

### Symptom

The same code with two different bases:

| Base / sink | Behavior |
|---|---|
| `videotestsrc num-buffers=300` → `autovideosink` (4 slots, 4 pump threads, 4 `appsrc`s) | **Works.** PREROLLED → PLAYING → frames flow → EOS → clean shutdown. |
| `v4l2src device=/dev/video0` → `v4l2sink device=/dev/video10` (4 slots) | **Aborts with `free(): invalid pointer`** after `pipeline state: Paused -> Playing` is logged. |

The `videotestsrc`-base playground also validates the activate/deactivate cycle (see `pg_slot_trigger.rs`): swap source mid-flight from transparent to red, ramp alpha 0→1, then swap to transparent + alpha 0, then activate a different slot with green. All clean transitions.

### Bisect on slot count (with v4l2 base)

| `SLOT_COUNT` | Plain passthrough (no slot activated) |
|---|---|
| 0 (no slots) | works |
| 1 | works |
| 2+ | stalls preroll (loopback never receives data) |

Activating a slot at startup (before `set_state(Playing)` returns) **also** stalls. Deferring activation by 500ms after `set_state(Playing)` makes 1-slot `--overlay <id>` work for some emoji and hang for others (flaky).

### `gdb`-equivalent observations

We haven't attached `gdb`. The `free(): invalid pointer` is a glibc heap-corruption abort, so the actual call stack is opaque from logs. `GST_DEBUG=2` shows the latency-query warnings during preroll but no specific element-level errors before the abort. Pump thread `debug!` logs show the pumps started and pushed a few frames before the abort.

### Variations attempted

All with the v4l2 base:

1. `is_live=true` vs `is_live=false` on every slot's appsrc.
2. With/without `block=true`, with/without `max-buffers=2`.
3. Pin compositor output caps to `video/x-raw,format=YUY2,width=1280,height=720,framerate=30/1` vs let it negotiate freely.
4. `compositor` properties `ignore-inactive-pads=true`, `start-time-selection=first`.
5. Push one priming buffer per slot at build time, before transitioning the pipeline (per the discourse thread that says appsrc needs a preroll buffer); then have the pump take over.
6. No priming, pump pushes from t=0 with monotonic PTS (this is what works in the videotestsrc playground).
7. Activate the slot before pipeline → PLAYING vs after a 500 ms delay vs not at all.
8. Single shared transparent frame Arc vs per-slot transparent frame.
9. Reduce slot count from 4 to 1 (only made the failure go from "abort" to "stall" — still didn't work for activation).

---

## What works (for grounding)

- Daemon: passthrough (`v4l2src ! videoconvert ! v4l2sink`) — works.
- Daemon: Step 2 always-on overlay (`compositor` + one bin attached at build, before `set_state(Playing)`) — works for all 5 curated emoji.
- Shell: `v4l2src + 4 videotestsrc overlays + compositor + v4l2sink` (i.e. multi-input compositor with v4l2 boundaries, when overlays are `videotestsrc` not `appsrc`) — works (see `playground/06-v4l2-multi-input.sh`).
- Shell: `compositor` with 5 `videotestsrc` inputs at fixed resolutions — works (`playground/02`).
- Shell: alpha=0 on a compositor sink pad does not block preroll, buffers still flow — works (`playground/04`).
- Rust playground: 4 appsrc-driven slots with `videotestsrc + autovideosink` base — works (`pg_appsrc_slots.rs`).
- Rust playground: same architecture + activate/deactivate cycle — works (`pg_slot_trigger.rs`).

## What doesn't work

- Dynamic `compositor.request_pad_simple` + add of `appsrc` chain to running pipeline: `not-linked` from base_src_loop. (Failure 1)
- Pre-allocated 4-slot architecture with `v4l2src + v4l2sink` boundaries: heap corruption on PAUSED→PLAYING. (Failure 2)
- 1-slot variant + activate-before-PLAYING: stalls preroll.
- 1-slot variant + activate-after-500 ms-delay: works for some emoji, flaky for others.

## What we haven't tried

1. **Canonical IDLE-probe-on-existing-pad pattern for dynamic add.** Most likely thing we got wrong on Failure 1.
2. **`glvideomixer`** (GPU-side) instead of `compositor`. Different aggregator; might not have the same negotiation quirks. Conflicts with our "no GPU paths until profiling" commitment but worth a 30-min spike.
3. **Pre-bake reactions to short WebM/AV1 clips at build time** and play with `decodebin` per trigger. Sidesteps appsrc + v4l2 + multi-input entirely. Drops the procedural-transforms elegance.
4. **Smaller / longer-lived gdb capture** of the heap corruption. Run the v4l2 playground under gdb, get a backtrace.
5. **`gst-rs` issue / GStreamer Discourse post** with the playground's `pg_v4l2_slots.rs` as the minimal repro.

---

## Reproducers in this repo

```
playground/
├── README.md
├── 01-compositor-baseline.sh          # passes — sanity
├── 02-compositor-multi-input.sh       # passes — 5 inputs, all videotestsrc
├── 03-valve-gated.sh                  # FAILS preroll (valve drop=true blocks all buffers)
├── 04-alpha-gated.sh                  # passes — alpha=0 doesn't block preroll
├── 05-imagefreeze-overlay.sh          # passes
└── 06-v4l2-multi-input.sh             # passes — v4l2src + 4 videotestsrc overlays + v4l2sink

crates/pipeline/examples/
├── pg_appsrc_slots.rs                 # passes — 4 appsrc slots with videotestsrc + autovideosink
├── pg_slot_trigger.rs                 # passes — activate/deactivate cycle
└── pg_v4l2_slots.rs                   # FAILS — same architecture with v4l2 boundaries
```

Run any shell experiment in <10 s; each Rust example via `cargo run --example pg_<name>`.

The minimal repro of Failure 2 is `pg_v4l2_slots.rs` (~150 lines, no daemon code involved).

---

## Open questions for prior art / other LLMs

1. Is the canonical dynamic-add pattern (IDLE probe on an existing pipeline-side pad, link new branch in callback, return `GST_PAD_PROBE_REMOVE`) reliable in 2026 with `gstreamer-rs` 0.25 + GStreamer 1.28? Any examples?

2. Why does `gst_base_src_loop: not-linked` fire on a freshly-attached `appsrc` whose `src` pad is linked to a `videoconvert.sink` we explicitly linked seconds before? What is `base_src_loop` checking that fails?

3. Why does a multi-`appsrc` + `compositor` topology that prerolls cleanly with `videotestsrc + autovideosink` corrupt the heap when wired to `v4l2src + v4l2sink`? Is there a known interaction between `v4l2sink`'s buffer pool, `compositor`'s aggregator, and parallel `appsrc` push threads?

4. For a fixed-N pre-allocated overlay-slot architecture (one of our chosen designs), what's the right mechanism to "gate" each slot's contribution to the compositor without blocking preroll?
   - `valve drop=true` blocks even the preroll buffer (our `playground/03` test).
   - `alpha=0` on the compositor sink pad works for buffer flow but the activate/deactivate path still corrupts the heap with v4l2 boundaries.
   - Is there a `compositor` property like "skip pad if its `alpha==0`" that avoids the buffer flow entirely?

5. Is there a pattern for swapping `appsrc` source data atomically that's safe across `v4l2sink`'s buffer-pool lifecycle? Our `Mutex<Arc<AnimatedFrames>>` swap is documented as safe with the videotestsrc playground; the v4l2 abort is the only difference.

---

## Hypotheses we've considered and not eliminated

- **Heap corruption is not in our Rust code.** No `unsafe` is used. `Arc<AnimatedFrames>` and `Vec<u8>` lifetimes are straightforward. The same code without v4l2 boundaries does not corrupt.
- **Possible `v4l2sink` buffer-pool interaction with multi-input `compositor`.** `v4l2sink` advertises specific allocators that `compositor` may use; the upstream `appsrc` pumps may interact with that pool in some unsafe way.
- **Possible double-free on `appsrc` shutdown** when multiple appsrcs in a multi-input pipeline are all in flushing state. No supporting evidence yet.
- **The `latency` property mismatch between live `v4l2src` and live `appsrc`s** — we see "Can't give latency since framerate isn't fixated" warnings during preroll, even though the camera caps are pinned to `framerate=30/1`. May be benign noise, may be related.

---

## Code references

The smallest reproducers are in `crates/pipeline/examples/pg_v4l2_slots.rs` (failing) and `crates/pipeline/examples/pg_appsrc_slots.rs` (passing). They differ only in the pipeline string (v4l2 vs videotestsrc/autovideosink). About 150 lines each.

The pump pattern is identical across both — `Mutex<gst::ClockTime>` for monotonic PTS, `appsrc.push_buffer(b)` in a loop, exit on `Flushing`/`Eos`. The buffer is built fresh each iteration with `gst::Buffer::with_size(N).get_mut().copy_from_slice(0, &raw)`, no buffer pool reuse.

We're happy to provide GStreamer logs at any verbosity, gdb backtraces (with guidance), or a self-contained C reproducer if that's useful.

---

## Round 2 — followup after external review

A second LLM pass surfaced additional workarounds. We tried the cheap ones; results below.

### What we tried this round

| Attempt | In playground | In daemon |
|---|---|---|
| `identity drop-allocation=true` before `v4l2sink` | abort still reproduces in `pg_v4l2_slots` | — |
| Deferred pump start (after `set_state(Playing)` returns) + per-pad priming buffer | preroll deadlock when only priming buffers, abort when also using `ignore-inactive-pads=true` | — |
| `block=false` + `leaky-type=downstream` + `max-buffers=1` | not tested in playground after preceding failures | abort still reproduces |
| **Single 1280×720 RGBA overlay canvas** (one `appsrc`, one compositor sink_1, application-side compositing of all reactions) | **`pg_v4l2_canvas.rs` runs cleanly end-to-end**: pipeline reaches PLAYING, loopback flips to Capture, `gst-launch v4l2src` consumer reads frames | **flaky**: plain passthrough sometimes works, sometimes stalls preroll; `--overlay <id>` activated *after* the pipeline reaches PLAYING (deferred 500ms) sometimes works, sometimes shows Video Output |

The single-canvas approach is **architecturally sound**: it has only ONE `appsrc` (collapsing N reactions into application-side compositing), so it sidesteps the multi-`appsrc` interaction. And the playground proves the pipeline shape works with `v4l2src` + `v4l2sink`. But the daemon's port — code-equivalent to the playground modulo the application-side blit + a `Mutex<Vec<ActiveReaction>>` — exhibits the same Video-Output-stuck flakiness as the slot approaches. We could not find a reliably-passing variant in the daemon despite many attempts.

### Working theory after round 2

Something about the daemon's startup sequence (vs the playground's) makes preroll non-deterministic with `v4l2src + v4l2sink` + ANY `appsrc`-driven compositor input. The "lock-and-fill canvas" work in `compose_into` is the only meaningful difference from the playground's "push the same buffer with advancing PTS" pump — and that work happens *between* lock acquire/release, so it can't deadlock with anything.

We suspect this is a v4l2sink buffer-pool / aggregator interaction that's race-prone but not consistently fatal. The flakiness pattern (sometimes-Capture, sometimes-Output, occasionally heap-corrupting) is consistent with a race that wins or loses based on scheduling jitter at startup.

### Still untried

1. **`gdb` capture with `MALLOC_CHECK_=3` + `G_DEBUG=fatal-criticals,fatal-warnings`** under `pg_v4l2_slots.rs` to get a real backtrace of the heap abort.
2. **Build/install `v4l2loopback` `main`** which contains [PR #656](https://github.com/v4l2loopback/v4l2loopback/issues/656) (Jan 2026) fixing `VIDIOC_DQBUF` returning unqueued buffers — that fix is upstream of the abort path.
3. **The canonical IDLE-probe dynamic-add pattern** — block an existing live data-path pad (e.g. `videoconvert.src` upstream of `compositor`), and inside the probe callback link the new branch + bring it to PLAYING. We never tested this; it might just work and obviate the multi-`appsrc` question.

### Current shipping state

Daemon is at commit `970fa3a` baseline: `--overlay <id>` always-on works for all 5 emoji. `--triggers-stdin` not wired. Playground experiments preserved at `crates/pipeline/examples/pg_*.rs` and `playground/*.sh`.

---

## Round 3 — `gdb` backtrace under `MALLOC_CHECK_=3`

Captured with:

```bash
MALLOC_CHECK_=3 G_DEBUG=fatal-criticals,fatal-warnings GST_DEBUG=2 \
gdb --batch \
    --ex run --ex 'bt full' --ex 'thread apply all bt 30' \
    --args ./target/debug/examples/pg_v4l2_slots
```

The reproducer aborts at PAUSED→PLAYING with `malloc(): smallbin double linked list corrupted`. **The corruption is inside the GStreamer V4L2 plugin (`libgstvideo4linux2.so`), reached from the caps-query path, with multiple `appsrc` streaming threads invoking it concurrently.**

### The smoking gun (Thread 10, the aborter)

```
#0  syscall                  /usr/lib/libc.so.6
#1  raise                    /usr/lib/libc.so.6
#2  abort                    /usr/lib/libc.so.6
#3-#6                        libc.so.6 (heap-corruption detection)
#7  g_slist_foreach          /usr/lib/libglib-2.0.so.0
#8  ???                      /usr/lib/gstreamer-1.0/libgstvideo4linux2.so   ← V4L2 plugin
#9  ???                      /usr/lib/gstreamer-1.0/libgstvideo4linux2.so   ← V4L2 plugin
#10 ???                      /usr/lib/libgstbase-1.0.so.0
#11 ???                      /usr/lib/libgstbase-1.0.so.0
#12 gst_pad_query             /usr/lib/libgstreamer-1.0.so.0
#13 gst_pad_peer_query        /usr/lib/libgstreamer-1.0.so.0
#14 gst_pad_peer_query_caps   /usr/lib/libgstreamer-1.0.so.0
... (recurses through compositor + queues)
#29 (top of recursion)       /usr/lib/libgstvideo-1.0.so.0
```

### The other clue (Thread 11, blocked)

A second appsrc streaming thread is blocked on `__lll_lock_wait_private` inside the **same** V4L2-plugin GSList iteration path:

```
#0  __lll_lock_wait_private   /usr/lib/libc.so.6
#1  ???                       /usr/lib/libc.so.6
#2  g_slist_foreach           /usr/lib/libglib-2.0.so.0
#3  ???                       /usr/lib/gstreamer-1.0/libgstvideo4linux2.so   ← V4L2 plugin
#4  ???                       /usr/lib/gstreamer-1.0/libgstvideo4linux2.so   ← V4L2 plugin
... (same path as Thread 10)
```

So at the moment of corruption: **one thread is iterating an internal V4L2-plugin GSList, another thread is contending on the same path's lock**, and the iteration trips heap corruption. Threads 8–11 are all `slot-N-src:src` — appsrc streaming threads, not our application pump threads. They're propagating `peer_query_caps` upstream during preroll's caps negotiation.

### Diagnosis (symbolized via Arch's `debuginfod`)

A second `gdb` capture with `set debuginfod enabled on` resolved every frame to source line. The race is concrete and named:

```
Thread 10 (slot-0-src:src) — aborter:
  gst_v4l2_object_get_format_list  v4l2object=0x555555aab4a0  gstv4l2object.c:1418
  gst_v4l2_object_probe_caps       v4l2object=0x555555aab4a0  gstv4l2object.c:5363
  gst_v4l2_object_get_caps         v4l2object=0x555555aab4a0  gstv4l2object.c:5545
  gst_v4l2sink_get_caps                                       gstv4l2sink.c:497
  gst_base_sink_query_caps                                    gstbasesink.c:636
  gst_base_sink_default_query                                 gstbasesink.c:5611
  gst_pad_query / gst_pad_peer_query / gst_pad_peer_query_caps
  ... (recurses upward through identity, capsfilter, videoconvert, compositor)

Thread 9 (slot-2-src:src) — concurrent on the SAME v4l2object:
  gst_v4l2_object_probe_caps       v4l2object=0x555555aab4a0  gstv4l2object.c:5427  (different line)
  gst_v4l2_object_get_caps         v4l2object=0x555555aab4a0  gstv4l2object.c:5545
  gst_v4l2sink_get_caps                                       gstv4l2sink.c:497
  ... (same path)
```

Identical `v4l2object` pointer, two `appsrc` streaming threads, both inside `gst_v4l2_object_probe_caps`. The function is not safe for concurrent invocation against a single object: it iterates internal GSLists (formats, colorimetries, frame intervals) via `g_slist_foreach` without a mutex.

The bug is in **`gst-plugins-good/sys/v4l2/gstv4l2object.c`**, lines around 1418/5363/5427/5545. Specifically:
- `gst_v4l2_object_get_format_list` builds/returns the supported-format GSList lazily.
- `gst_v4l2_object_probe_caps` iterates it (and other GSLists) while building a caps result.
- `gst_v4l2_object_get_caps` calls `probe_caps`.
- `gst_v4l2sink_get_caps` calls `get_caps`.

When N `appsrc` inputs reach `compositor.sink_N` and the compositor's caps-negotiation propagates each input's `peer_query_caps` upstream, those queries each terminate at `v4l2sink`'s caps handler. With N parallel streaming threads, N concurrent calls to `gst_v4l2sink_get_caps` → N concurrent `probe_caps` on the same `v4l2object` → race on the format GSList → heap corruption.

### Why other configurations don't crash

- `videotestsrc` base + `autovideosink`: no V4L2 plugin in the graph at all. Caps queries don't reach it. The same `pg_appsrc_slots.rs` runs cleanly.
- 1 slot only: only one appsrc streaming thread doing peer_query_caps; no concurrency on the V4L2 path.
- Step 2 (the working baseline): uses `imagefreeze` and a single startup-attached overlay. Whatever caps queries it does, they happen serially before `set_state(Playing)` and not from multiple appsrc streaming threads simultaneously.

### Implications for the workarounds

| Workaround | Why it didn't help |
|---|---|
| `identity drop-allocation=true` before `v4l2sink` | drops allocation queries, but **not caps queries** — that's the actual crashing path. |
| `block=false` + `leaky-type=downstream` + `max-buffers=1` | changes appsrc backpressure, doesn't change the caps-query fan-in. |
| Deferred pump start | doesn't matter — the caps queries that crash are part of preroll, before our pump starts pushing. |
| Pre-allocated slots vs dynamic add | both produce N concurrent appsrc streaming threads doing peer_query_caps into V4L2. Same race. |

### Implications for the actual fix

1. **Bug worth filing against `gst-plugins-good`** at <https://gitlab.freedesktop.org/gstreamer/gstreamer/-/issues>. The MWE is `pg_v4l2_slots.rs` (~150 lines, no `unsafe`, deterministically aborts under `MALLOC_CHECK_=3` at PAUSED→PLAYING). With the symbolized backtrace above, the bug report is essentially complete.
2. **High-confidence workaround: shield `v4l2sink` from upstream caps queries with a `capsfilter`.** A `capsfilter caps="video/x-raw,format=YUY2,width=1280,height=720,framerate=30/1"` immediately upstream of `v4l2sink` answers `peer_query_caps` from its own pinned caps and never forwards the query past it. With the query terminated at the capsfilter, `gst_v4l2_object_probe_caps` is never called from concurrent threads. Untested but principled.
3. **Single-canvas architecture is the right product-level pivot regardless.** With one `appsrc` there's only one streaming thread, so even without the capsfilter shield the V4L2 fan-in is gone. The daemon's earlier flakiness with single canvas needs a clean re-investigation now that we have the right mental model.

### Still-untried (lower-priority now)

- Testing `v4l2loopback` `main` with PR #656 — confirmed unrelated; the abort is in user-space `libgstvideo4linux2.so`, not the kernel module.

---

## Round 4 — query-probe workarounds (partial progress)

Following round 3's conclusion ("intercept CAPS queries before they reach `gst_v4l2_object_probe_caps`"), tried two probe variants on `v4l2sink.sink_pad`. See `crates/pipeline/examples/pg_v4l2_slots_probe.rs`.

### Variant A: hand-built fixed caps

```rust
let fixed_caps = gst::Caps::builder("video/x-raw")
    .field("format", "YUY2")
    .field("width", 1280_i32).field("height", 720_i32)
    .field("framerate", gst::Fraction::new(30, 1))
    .build();
sink_pad.add_probe(QUERY_DOWNSTREAM, |_, info| {
    if let QueryViewMut::Caps(q) = info.query_mut().unwrap().view_mut() {
        q.set_result(&fixed_caps);
        return PadProbeReturn::Handled;
    }
    PadProbeReturn::Ok
});
```

**Result:** no more heap corruption — pipeline reaches PAUSED cleanly, `gst_v4l2_object_probe_caps` is bypassed. But fails at PAUSED→PLAYING with `streaming stopped, reason not-negotiated (-4)` from each `slot-N-src`. The hand-built caps lack fields the compositor's negotiation needs (probably colorimetry / pixel-aspect-ratio / interlace-mode), so intersection with upstream produces incomplete caps and negotiation fails.

### Variant B: cache v4l2sink's actual caps once, replay them

```rust
let cached = sink_pad.query_caps(None);  // single-threaded, before pipeline transitions
sink_pad.add_probe(QUERY_DOWNSTREAM, move |_, info| {
    if let QueryViewMut::Caps(q) = info.query_mut().unwrap().view_mut() {
        let result = match q.filter() {
            Some(f) => cached.intersect(f),
            None => cached.clone(),
        };
        q.set_result(&result);
        return PadProbeReturn::Handled;
    }
    PadProbeReturn::Ok
});
```

**Result:** also no heap corruption — same `not-negotiated`. The "cached" caps queried before `set_state` is the V4L2 plugin's *template* (every supported codec/format/range), not the loopback's specific accepted caps, because `v4l2sink` is still in NULL state and the device hasn't been opened. The cached caps are too broad for compositor to fixate to a single output format.

### Refining the cache

The right time to cache is *after* `v4l2sink` has opened the device but *before* the racy multi-thread negotiation begins. Two sub-options not yet tested:

1. Set the pipeline to `PAUSED` once on the main thread, query caps (now the device-specific subset), set back to `READY`, install probe, then go to `PLAYING`.
2. Use `gst_pad_set_query_function` to install a custom query handler that runs the default handler under a mutex — serializes concurrent calls instead of replacing the result. Cleaner conceptually but more involved than a probe.

### Larger implication: the race triggers with N ≥ 2 upstream tasks (not specifically N ≥ 2 appsrcs)

In the daemon's single-canvas topology, the camera-side `v4l2src` thread *also* propagates CAPS queries downstream during compositor negotiation. So even with one `appsrc` (the canvas), the compositor has two upstream input pads — `v4l2src` and `appsrc` — each producing a caps query through to `v4l2sink`. **That's the same race**, just with N=2 instead of N=4. This explains why the daemon's single-canvas attempt was flaky despite the playground (`pg_v4l2_canvas` — same architecture but base `videotestsrc → autovideosink` instead of v4l2) being clean.

So even single-canvas needs the query firewall to be reliable on real v4l2 hardware, contrary to what we initially thought.

### Summary going into round 5

We have:
- The bug, named to file/line. Filing-quality.
- A fully reproducible MWE (`pg_v4l2_slots.rs`) that aborts deterministically under `MALLOC_CHECK_=3`.
- Confirmation that intercepting CAPS queries before `v4l2sink` removes the abort (no more heap corruption).
- The remaining problem: intercepted caps must be the device-specific accepted caps, not the V4L2-template superset. Variants A and B both got this wrong.

What we need (or what an upstream fix would do): mutex `GstV4l2Object`'s caps-probe path. Until that lands, the workaround needs to be a probe / custom query function that returns the device's actual probed caps, OR a serialization-of-concurrent-calls mechanism.

Daemon shipping state unchanged — at commit `970fa3a` baseline, `--overlay <id>` works; Step 3 deferred pending upstream resolution or a working query-firewall variant.

---

## Round 5 — working query-firewall recipe

After the round-4 partial progress, applied the round-4 advice from the second LLM session: **device-specific caps from a single-threaded standalone `v4l2sink` in READY**, intersected with a fully-specified preference, and **ACCEPT_CAPS handled too**. Result in `crates/pipeline/examples/pg_v4l2_slots_probe.rs`:

```text
device caps from READY-state v4l2sink: video/x-raw(memory:DMABuf), ... ; video/x-raw, format=(string){ YUY2, UYVY, NV12, ... }, width=(int)[ 2, 8192 ], ...
firewall_caps: video/x-raw, format=(string)YUY2, width=(int)1280, height=(int)720, pixel-aspect-ratio=(fraction)1/1, colorimetry=(string){ ... }, framerate=(fraction)30/1
starting pipeline (4 appsrc slots, v4l2 + caps-query firewall)
pipeline state: Null -> Ready
pipeline state: Ready -> Paused
pipeline state: Paused -> Playing
```

End-to-end verified: loopback flips to `Video Capture`, downstream `gst-launch-1.0 v4l2src device=/dev/video10` consumer reads frames cleanly, daemon exits 0 on SIGINT. **No abort, no `not-negotiated`, no flakiness.** Reproducible across many runs.

### The recipe that works

```rust
// 1. Derive device-specific caps from a temporary, isolated v4l2sink.
//    NULL-state caps are the V4L2 plugin's template (every supported
//    codec); READY-state caps are device-specific and include the right
//    colorimetry, pixel-aspect-ratio, etc., so compositor can fixate.
fn derive_firewall_caps() -> Result<gst::Caps> {
    let probe = gst::ElementFactory::make("v4l2sink")
        .property("device", "/dev/video10")
        .property("sync", false)
        .build()?;
    probe.set_state(gst::State::Ready)?;
    let _ = probe.state(gst::ClockTime::from_seconds(2));
    let device_caps = probe.static_pad("sink").unwrap().query_caps(None);
    probe.set_state(gst::State::Null).ok();

    let preferred = gst::Caps::builder("video/x-raw")
        .field("format", "YUY2")
        .field("width", 1280_i32)
        .field("height", 720_i32)
        .field("framerate", gst::Fraction::new(30, 1))
        .build();
    let firewall = device_caps.intersect(&preferred);
    if firewall.is_empty() {
        anyhow::bail!("device ∩ preferred is empty — pick a different format");
    }
    Ok(firewall)
}

// 2. Install QUERY_DOWNSTREAM probe on the real v4l2sink. Handles BOTH
//    CAPS and ACCEPT_CAPS so v4l2sink's default handler is never invoked
//    from streaming threads.
let firewall_caps = derive_firewall_caps()?;
let sink_pad = real_v4l2sink.static_pad("sink").unwrap();
sink_pad.add_probe(gst::PadProbeType::QUERY_DOWNSTREAM, move |_pad, info| {
    let Some(query) = info.query_mut() else { return gst::PadProbeReturn::Ok };
    match query.view_mut() {
        gst::QueryViewMut::Caps(q) => {
            let result = match q.filter() {
                Some(f) => firewall_caps.intersect(f),
                None => firewall_caps.clone(),
            };
            q.set_result(&result);
            gst::PadProbeReturn::Handled
        }
        gst::QueryViewMut::AcceptCaps(q) => {
            q.set_result(q.caps().can_intersect(&firewall_caps));
            gst::PadProbeReturn::Handled
        }
        _ => gst::PadProbeReturn::Ok,
    }
});
```

That's the entire workaround. ~30 lines.

### Why each piece matters (insights from the LLM bounce)

- **READY (not NULL)** — v4l2sink in NULL returns the V4L2 plugin's pad-template caps (every codec it can handle). READY opens the device and probes its actual capabilities. Single-threaded execution at this point is what makes it safe.
- **Intersect with preferred** — narrows a many-format READY result to the single output we want compositor to fixate on. Without the intersection, the firewall caps are still too broad and compositor can't pick.
- **Handle ACCEPT_CAPS too** — without it, downstream still asks v4l2sink "do you accept these caps?" via `ACCEPT_CAPS` queries, which (we suspect) re-enter the racy V4L2 path. Intercepting both query types closes the door fully.
- **`PadProbeReturn::Handled`** — must replace, not just observe. Returning `Ok` would let the query proceed to v4l2sink's default handler (the racy path).

### Status

- **Bug confirmed in `gst-plugins-good`** at the file/line level (round 3) and **workaround validated** (this round). The MWE `pg_v4l2_slots_probe.rs` runs cleanly under `MALLOC_CHECK_=3` for any number of slots.
- **Ready to file upstream.** The bug report has both a deterministic crash repro AND a working application-level workaround that confirms the diagnosis.
- **Daemon Step 3 is unblocked.** Architectural choice: pre-allocated N slots OR single canvas — both work with the firewall. Multi-slot keeps the original Step 3 plan; single-canvas is simpler and gains unbounded reaction stacking via application-side compositing.
