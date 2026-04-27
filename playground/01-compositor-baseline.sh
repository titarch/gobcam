#!/usr/bin/env bash
# Sanity: a single `compositor` with one `videotestsrc` input reaches PLAYING.
set -euo pipefail
echo '== videotestsrc ! compositor ! videoconvert ! autovideosink =='
exec gst-launch-1.0 -e \
    videotestsrc num-buffers=120 pattern=ball ! \
    video/x-raw,width=640,height=480,framerate=30/1 ! \
    compositor name=mix background=black ! \
    videoconvert ! autovideosink
