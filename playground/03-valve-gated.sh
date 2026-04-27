#!/usr/bin/env bash
# Validate the valve-gated overlay approach: a pre-attached overlay branch
# with `valve drop=true` blocks the overlay's buffer flow without stalling
# the rest of the pipeline. If this works at gst-launch level, it's the
# right primitive for Step 3 in the daemon.
#
# Test: with the overlay's valve set to drop=true at startup, the base
# videotestsrc should still flow through the compositor and reach the sink.
set -euo pipefail
echo '== base + valve(drop=true) overlay branch — base must still flow =='

exec gst-launch-1.0 -e \
    videotestsrc num-buffers=120 pattern=ball ! \
    video/x-raw,width=640,height=480,framerate=30/1 ! \
    compositor name=mix background=black ! \
    videoconvert ! autovideosink \
    \
    videotestsrc num-buffers=120 pattern=smpte ! \
    video/x-raw,width=256,height=256,framerate=30/1 ! \
    valve name=v drop=true ! \
    queue ! \
    mix.sink_1
