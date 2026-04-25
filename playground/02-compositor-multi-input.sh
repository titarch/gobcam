#!/usr/bin/env bash
# Reproduce the daemon's preroll-deadlock failure mode: a compositor with one
# base input + N pre-allocated overlay inputs (videotestsrc each, like our
# pre-allocated slots). Goal: see whether multi-input compositor with all
# inputs producing buffers from t=0 reaches PLAYING.
set -euo pipefail
echo '== videotestsrc x5 ! compositor ! videoconvert ! autovideosink =='

# Base camera-equivalent + 4 overlay-equivalent inputs.
exec gst-launch-1.0 -e \
    videotestsrc num-buffers=120 pattern=ball ! \
    video/x-raw,width=640,height=480,framerate=30/1 ! \
    compositor name=mix background=black \
        sink_1::xpos=0   sink_1::ypos=0   sink_1::alpha=0.5 \
        sink_2::xpos=384 sink_2::ypos=0   sink_2::alpha=0.5 \
        sink_3::xpos=0   sink_3::ypos=224 sink_3::alpha=0.5 \
        sink_4::xpos=384 sink_4::ypos=224 sink_4::alpha=0.5 \
    ! videoconvert ! autovideosink \
    \
    videotestsrc num-buffers=120 pattern=snow    ! video/x-raw,width=256,height=256,framerate=30/1 ! mix.sink_1 \
    videotestsrc num-buffers=120 pattern=smpte   ! video/x-raw,width=256,height=256,framerate=30/1 ! mix.sink_2 \
    videotestsrc num-buffers=120 pattern=circular ! video/x-raw,width=256,height=256,framerate=30/1 ! mix.sink_3 \
    videotestsrc num-buffers=120 pattern=zone-plate ! video/x-raw,width=256,height=256,framerate=30/1 ! mix.sink_4
