# Demo footage

`assets/demo/conference-stand-in.mp4` is the input clip used by
`just demo-capture` as a stand-in for a real webcam feed when
generating the hero GIF for the project README. The
file is gitignored — fetch it on demand with:

```sh
scripts/fetch-demo-clip.sh
```

`capture-demo.sh` invokes the fetcher automatically when the clip
isn't present, so a fresh clone "just works".

The static emoji-picker screenshot is generated separately with
`just demo-ui-capture`. That path runs the Svelte panel in Vite with
mocked Tauri commands, so it does not need a daemon, Tauri window, or
loopback device.

## Source

[*Big Buck Bunny*][bbb] © Blender Foundation, licensed
[CC-BY 3.0][cc-by-3].

`fetch-demo-clip.sh` uses ffmpeg's HTTP range-seek to pull only the
chosen window (a few MB) instead of the full ~170 MB asset. Override
`START`, `DURATION`, `WIDTH`, `HEIGHT`, or `CRF` env vars to trim a
different segment.

[bbb]: https://peach.blender.org/
[cc-by-3]: https://creativecommons.org/licenses/by/3.0/
