#!/usr/bin/env bash
# Reproduce daemon's preroll deadlock at gst-launch level: real v4l2src +
# v4l2sink (matching daemon) with N pre-allocated overlay inputs. If this
# stalls the same way, it's a GStreamer behavior with v4l2 boundaries that
# the daemon must work around. If it WORKS, the daemon has an impl bug.
set -euo pipefail
echo '== v4l2src + 4 videotestsrc overlays + v4l2sink =='

exec gst-launch-1.0 -e \
    v4l2src device=/dev/video0 ! \
    video/x-raw,width=1280,height=720,framerate=30/1 ! \
    queue ! videoconvert ! \
    compositor name=mix background=black \
        sink_1::xpos=900 sink_1::ypos=400 sink_1::alpha=0.7 \
        sink_2::xpos=900 sink_2::ypos=140 sink_2::alpha=0.7 \
        sink_3::xpos=100 sink_3::ypos=400 sink_3::alpha=0.7 \
        sink_4::xpos=100 sink_4::ypos=140 sink_4::alpha=0.7 \
    ! video/x-raw,format=YUY2,width=1280,height=720,framerate=30/1 \
    ! videoconvert ! v4l2sink device=/dev/video10 sync=false \
    \
    videotestsrc num-buffers=120 pattern=smpte ! video/x-raw,width=256,height=256,framerate=30/1 ! mix.sink_1 \
    videotestsrc num-buffers=120 pattern=snow  ! video/x-raw,width=256,height=256,framerate=30/1 ! mix.sink_2 \
    videotestsrc num-buffers=120 pattern=ball  ! video/x-raw,width=256,height=256,framerate=30/1 ! mix.sink_3 \
    videotestsrc num-buffers=120 pattern=zone-plate ! video/x-raw,width=256,height=256,framerate=30/1 ! mix.sink_4
