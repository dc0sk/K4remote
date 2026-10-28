# Running K4 Remote for a screen test, isolated

`gui-run.sh` starts K4 Remote (usually `--demo`) where a screen test cannot touch the operator's
session:

- **its own X display**: an Xvfb on `:99`, with `WAYLAND_DISPLAY` unset (the toolkit would
  otherwise prefer the desktop's Wayland);
- **its own config directory** (`<scratch>/cfg`);
- **a private D-Bus with nothing activatable** (`bus.conf` has no `<servicedir>`), so no keyring
  daemon can start. The app is started only if a probe on that bus answers `ServiceUnknown` for
  the Secret Service. A token typed in the app is kept for that session only.

```sh
Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp &          # once
tools/gui-run/gui-run.sh /path/to/scratch target/debug/k4remote --demo &
kill "$(cat /path/to/scratch/app.pid)"                    # stop the app itself
tools/gui-run/gui-run.sh --self-test                      # the gate refuses and accepts as it should
```

Stop the app by `app.pid`. Killing the script's own PID leaves the app and its bus running. In
`--demo` a tune shows for about 2 s before the display falls back to the demo frequency.

Each part exists because its absence caused an incident: a window on the operator's desktop
(2026-10-10), a test token in the real keyring (2026-10-27), and a default `dbus-run-session`
starting gnome-keyring, which went looking for the real daemon (2026-10-28).
