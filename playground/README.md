# GStreamer playground

Fast-iteration experiments to validate pipeline topologies before wiring them
into the main daemon. Uses `videotestsrc` so no webcam / loopback / sudo is
needed; outputs to `autovideosink` (a window). Each script auto-ends after
~10s via `num-buffers=300`.

Run any experiment:

```sh
playground/01-compositor-baseline.sh
playground/02-compositor-multi-input.sh
playground/03-valve-gated.sh
cargo run --bin pg-dynamic-add        # Rust experiment, see crates/playground
```

Each script prints the pipeline graph it's testing on first line.

## What each one tests

| Script | Hypothesis being checked |
|---|---|
| `01-compositor-baseline.sh` | Bare `videotestsrc ! compositor ! sink` works at all. Sanity. |
| `02-compositor-multi-input.sh` | Compositor with 1 base + 4 pre-allocated inputs reaches PLAYING. Tests the preroll-deadlock failure mode the daemon hit. |
| `03-valve-gated.sh` | An overlay branch with `valve drop=true` — does it block the overlay's data without stalling the rest of the pipeline? Validates the candidate "valve-gated permanent branch" approach for Step 3. |
| `04-appsrc-multi.sh` | Same as 02 but the secondary inputs are `appsrc` chains backed by Rust shoving RGBA buffers — closest analog to the daemon's failure case. |
| `crates/playground/src/bin/pg-dynamic-add.rs` | The "block an existing pad with an IDLE probe, link new element in the callback" pattern from the dynamic-pipelines blog. |

## Convention

Scripts return non-zero if the pipeline doesn't reach PLAYING. `gst-launch-1.0`
prints `Pipeline is PREROLLED ...` and `Setting pipeline to PLAYING ...` on
success. The scripts grep for those markers and fail loudly otherwise.
