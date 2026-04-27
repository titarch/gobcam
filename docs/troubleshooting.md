# Troubleshooting

## Do I need to start `gobcam-pipeline` myself?

No. Start **Gobcam** from the launcher, or run `gobcam-ui` / the
AppImage. The UI checks the daemon socket on startup: if a pipeline
daemon is already running it attaches to it; if not, it starts one.

Run `gobcam-pipeline` directly only when you are debugging the pipeline,
testing a specific command-line option, or driving it from a script.

## Loopback device locked when changing mode

> "Device '/dev/video10' cannot capture at WxH; device returned size 1280x720"

`v4l2loopback` locks its format to whatever the writer first set, for as
long as a consumer is reading. If a meeting app is open and reading the
loopback, a subsequent attempt to write a different mode fails.

The UI handles this automatically when you change resolution / framerate
in Settings — it shells out to `rmmod` + `modprobe` via the narrow
`sudoers.d` rule installed by `gobcam-setup`, then retries the spawn.
If that path isn't available (e.g. you skipped `gobcam-setup`):

```sh
just loopback:reset
# or, manually:
sudo rmmod v4l2loopback
sudo modprobe v4l2loopback devices=1 video_nr=10 \
    card_label=Gobcam exclusive_caps=1
```

Closing the consumer app first usually also works.

## NVIDIA proprietary driver — UI window comes up blank

WebKitGTK's DMABUF renderer can't allocate GBM buffers under the
proprietary NVIDIA driver. The launcher needs `WEBKIT_DISABLE_DMABUF_RENDERER=1`
in its environment.

This is set automatically in:

- `justfile` recipes (`just app`, `just dev:ui`, `just build:ui`)
- The `.desktop` file shipped by the `.deb`
- The Docker build container

If you're launching from somewhere else (custom systemd unit, terminal
without `just`, hand-crafted AppImage `.desktop`), set the variable
yourself:

```sh
WEBKIT_DISABLE_DMABUF_RENDERER=1 gobcam-ui
```

## "Element failed to change its state" on first run

The daemon couldn't open `/dev/video10`. Either the loopback module
isn't loaded or it was loaded with a different `video_nr`.

The AppImage detects this and shows an in-app setup screen — clicking
"Run setup" launches `gobcam-setup` via `pkexec`, which writes the
`/etc/modules-load.d`, `/etc/modprobe.d`, and `/etc/sudoers.d` snippets
needed for the auto-reset path.

The `.deb` runs the same setup automatically via its `postinst`.

If you're running from source: `just loopback:install` once.

## Chromium-based apps don't list Gobcam as a camera

The `v4l2loopback` module needs `exclusive_caps=1` for Chromium and
its descendants (browsers, Teams-for-Linux) to enumerate the device.
The `modprobe.d` snippet `gobcam-setup` writes already includes this;
if you loaded the module manually with different options, reset it.

## Daemon dies silently after a settings change

The UI watches the daemon for ~800 ms after socket binding to surface
preroll-time errors. If the failure happens later (e.g. the camera is
unplugged mid-session), the daemon logs to its own stdout, which is
piped into the UI process. Run the UI from a terminal to see what it
says, or pass `--profile-log <path>` for structured event output.

## Pipeline pegs a CPU core during a cascade

Expected on weak hardware at the default 48 slots. Levers, in order of
impact:

1. **Drop the slot count.** Settings → "Reaction slots" — 16 or 24
   keeps the cascade dense without saturating a core.
2. **Drop the per-slot canvas dimension.** Settings →
   "Reaction quality" — 192 or 128 cuts blend cost for slots showing
   small emoji.
3. **Drop the output resolution.** A 720p loopback costs roughly half
   what a 1080p loopback does for the same number of slots.

The CPU pipeline is intentional today — a GPU blend (`glvideomixer`)
is on the [roadmap](roadmap.md) if profiling shows it's worth the
copy boundary.

## Animated emoji takes ~400 ms on first trigger

That's the lazy APNG download (~200-500 KB per emoji from Microsoft's
CDN). Subsequent triggers of the same emoji are instant — the file is
cached under `$XDG_CACHE_HOME/gobcam/animated/` and the decoded frames
are LRU-cached in memory.

## `gobcam-setup` says password prompt failed

`gobcam-setup` self-elevates via `pkexec`. If `pkexec` isn't installed
or polkit isn't running, run the script with `sudo` instead:

```sh
sudo bash scripts/gobcam-setup
```

## I want to start over

```sh
just loopback:uninstall   # removes /etc snippets and rmmods the module
rm -rf ~/.cache/gobcam ~/.config/gobcam
```

Then re-run `gobcam-setup` (or `just loopback:install` from source)
and launch the UI.
