#!/usr/bin/env bash
# Install the host-side runtime prerequisites: v4l2loopback kernel module
# and the GStreamer 1.x plugin packages the daemon links against.
set -euo pipefail

SUDO="sudo"
[[ "$EUID" -eq 0 ]] && SUDO=""

if [[ -f /etc/arch-release ]]; then
  $SUDO pacman -S --needed --noconfirm \
    v4l2loopback-dkms v4l-utils \
    gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav
elif [[ -f /etc/debian_version ]]; then
  $SUDO apt-get update
  $SUDO apt-get install -y \
    v4l2loopback-dkms v4l-utils \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav
else
  cat >&2 <<EOF
[setup-host] unsupported distro. Install manually:
  - v4l2loopback-dkms
  - GStreamer 1.x runtime + dev packages
  - GStreamer plugins: base, good, bad, ugly, libav
EOF
  exit 1
fi

echo "[setup-host] loading v4l2loopback (devices=1 video_nr=10 exclusive_caps=1)..."
$SUDO modprobe v4l2loopback devices=1 video_nr=10 card_label="Gobcam" exclusive_caps=1

echo "[setup-host] done. /dev/video10 should now exist."
