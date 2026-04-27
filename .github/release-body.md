## Install

**Debian / Ubuntu / Mint / Pop!_OS / WSL** — `.deb` package:

```bash
sudo apt install ./Gobcam_*_amd64.deb
```

The post-install script loads `v4l2loopback`, drops the
auto-load snippets under `/etc/`, and writes a narrow sudoers
rule so the in-app loopback reset doesn't prompt. After install,
launch **Gobcam** from any application launcher.

**Other distros** — AppImage:

```bash
chmod +x Gobcam_*_amd64.AppImage
./Gobcam_*_amd64.AppImage
```

The `chmod +x` step is unavoidable — browsers/`curl` strip the
executable bit on download. On first launch, a "Set up Gobcam"
prompt appears that runs the `v4l2loopback` install via `pkexec`
(graphical password dialog).

Each artifact has a corresponding `.sha256` sibling. Verify the one
you downloaded with `sha256sum -c Gobcam_*.deb.sha256` or
`sha256sum -c Gobcam_*.AppImage.sha256`.

**Arch / Manjaro / EndeavourOS** — AUR publishing is upcoming. For now,
build from a checkout with `just aur-install-local`, then run
`sudo gobcam-setup`.
