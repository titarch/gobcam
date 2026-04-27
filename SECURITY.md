# Security policy

## Reporting a vulnerability

Please email **parsybaptiste@gmail.com** with the details
instead of opening a public issue. Include:

- A description of the issue and its impact.
- Steps (or a minimal reproducer) that demonstrate the problem.
- The Gobcam version and your distro / kernel / GStreamer versions.

I'll try to acknowledge the report within a few days. For issues that
need a fix before public discussion, we can coordinate timing by email.

## Scope

Gobcam runs as your user with no elevated privileges at runtime.
The only privileged surface is the optional `gobcam-setup` script
(invoked once via `pkexec` or `sudo`), which writes a narrow
`/etc/sudoers.d/gobcam` rule that grants passwordless `modprobe`
and `rmmod` of the `v4l2loopback` module to your account — and
nothing else. Findings against that rule, the postinst script, or
the `pkexec` helper are in scope.

Out-of-scope: misconfiguration of an installed `v4l2loopback` by
unrelated software; kernel-side issues in v4l2 or v4l2loopback
itself.
