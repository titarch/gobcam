#!/usr/bin/env bash
# Closer to the daemon's animated-overlay path: imagefreeze emits a single
# image at a chosen framerate. Replaces the daemon's appsrc → frame-pump.
# Use `videotestsrc num-buffers=1 ! imagefreeze` to feed one frame forever.
set -euo pipefail
echo '== base + imagefreeze overlay (single frame held) =='

exec gst-launch-1.0 -e \
    videotestsrc num-buffers=120 pattern=ball ! \
    video/x-raw,width=640,height=480,framerate=30/1 ! \
    compositor name=mix background=black \
        sink_1::xpos=192 sink_1::ypos=112 \
    ! videoconvert ! autovideosink \
    \
    videotestsrc num-buffers=1 pattern=smpte ! \
    video/x-raw,width=256,height=256,framerate=30/1 ! \
    imagefreeze ! \
    queue ! mix.sink_1
