#!/usr/bin/env bash
# Validate alpha-gating: a pre-attached overlay branch with sink_pad alpha=0
# is invisible but buffers keep flowing — preroll succeeds, the rest of the
# pipeline never stalls. To "fire" a reaction, set alpha=1.
set -euo pipefail
echo '== base + overlay branch with sink_1 alpha=0 (invisible) =='

# Run the pipeline. We can't toggle alpha at runtime via gst-launch; this
# experiment only validates that alpha=0 doesn't break preroll.
exec gst-launch-1.0 -e \
    videotestsrc num-buffers=120 pattern=ball ! \
    video/x-raw,width=640,height=480,framerate=30/1 ! \
    compositor name=mix background=black \
        sink_1::xpos=192 sink_1::ypos=112 sink_1::alpha=0.0 \
    ! videoconvert ! autovideosink \
    \
    videotestsrc num-buffers=120 pattern=smpte ! \
    video/x-raw,width=256,height=256,framerate=30/1 ! \
    queue ! mix.sink_1
