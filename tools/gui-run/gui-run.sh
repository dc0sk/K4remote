#!/usr/bin/env bash
# Run K4 Remote for a screen test, isolated from the operator's session: a private X display, a
# scratch config directory, and a private D-Bus on which NO keyring can start — proven by a probe
# whose answer is the condition the app is started under.
#
#   tools/gui-run/gui-run.sh <scratch-dir> <k4remote-binary> [app args…]   # e.g. … --demo
#   tools/gui-run/gui-run.sh --self-test
#
# Needs an Xvfb on $DISPLAY (default :99), started beforehand:
#   Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp
# The config goes to <scratch-dir>/cfg; the app's own PID to <scratch-dir>/app.pid — stop the app
# with `kill "$(cat <scratch-dir>/app.pid)"`, not by killing this script (that leaves it running).
#
# Why each part (incidents 2026-10-10/27/28): `DISPLAY` alone opened a window on the operator's
# desktop (the toolkit prefers `WAYLAND_DISPLAY`); unsetting `DBUS_SESSION_BUS_ADDRESS` let a test
# token into the real keyring; a default `dbus-run-session` activated gnome-keyring, which went for
# the real daemon; a probe printed before the launch, instead of gating it, let the app start anyway.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
export DISPLAY=${DISPLAY_TEST:-:99}

# The probe's verdict: isolated only if the bus says no Secret Service exists. Exit status cannot
# decide — gdbus exits 1 on exactly the answer wanted.
isolated() {
  grep -q 'org.freedesktop.DBus.Error.ServiceUnknown' <<<"$1"
}

probe() {
  timeout 5 gdbus call --session --dest org.freedesktop.secrets \
    --object-path /org/freedesktop/secrets \
    --method org.freedesktop.Secret.Service.SearchItems "{'service': 'k4remote'}" 2>&1
}

# Inside the private bus: probe, then start the app only on the isolated verdict.
if [ "${1:-}" = "--inner" ]; then
  scratch=$2
  shift 2
  unset GNOME_KEYRING_CONTROL SSH_AUTH_SOCK
  out=$(probe)
  if ! isolated "$out"; then
    echo "gui-run: NOT isolated, not starting the app. Probe said: $out" >&2
    exit 3
  fi
  echo "$$" > "$scratch/app.pid"
  exec env -u WAYLAND_DISPLAY DISPLAY="$DISPLAY" XDG_CONFIG_HOME="$scratch/cfg" "$@"
fi

if [ "${1:-}" = "--self-test" ]; then
  fail() { echo "SELF-TEST: FAIL — $1"; exit 1; }
  isolated 'Error: GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown: The name org.freedesktop.secrets was not provided by any .service files' \
    || fail "the ServiceUnknown answer was not taken as isolated"
  ! isolated "([objectpath '/org/freedesktop/secrets/collection/login/10'], @ao [])" \
    || fail "a keyring answer was taken as isolated"
  ! isolated "" || fail "an empty (timed-out) probe was taken as isolated"
  ! isolated "Error: GDBus.Error:org.freedesktop.DBus.Error.NoReply: Did not receive a reply" \
    || fail "a hung keyring was taken as isolated"
  t=$(mktemp -d "$HOME/.gui-run-selftest.XXXXXX")
  trap 'rm -rf "$t"' EXIT
  mkdir -p "$t/cfg"
  # Known-fail, live: on the operator's own session bus (a read-only query) the launch is refused.
  if [ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || [ -S "/run/user/$(id -u)/bus" ]; then
    out=$("$0" --inner "$t" sh -c 'echo LAUNCHED' 2>&1)
    case "$out" in
      *LAUNCHED*) fail "it started the app on the operator's session bus" ;;
      *"NOT isolated"*) ;;
      *) fail "unexpected output on the operator's bus: $out" ;;
    esac
  else
    echo "SELF-TEST: note — no session bus here, the live known-fail was skipped"
  fi
  # Known-pass, live: on the private bus the probe says ServiceUnknown and the app starts.
  out=$(dbus-run-session --config-file="$HERE/bus.conf" -- "$0" --inner "$t" sh -c 'echo LAUNCHED' 2>&1)
  case "$out" in
    *LAUNCHED*) ;;
    *) fail "the private bus was not taken as isolated: $out" ;;
  esac
  echo "SELF-TEST: PASS — refuses on a keyring answer, an empty or hung probe, and the operator's own bus;"
  echo "           starts on the private bus with no service directories"
  exit 0
fi

[ $# -ge 2 ] || { sed -n '6,7p' "$0" >&2; exit 2; }
scratch=$1
shift
pgrep -x Xvfb >/dev/null || { echo "gui-run: no Xvfb running (start one on $DISPLAY first)" >&2; exit 2; }
mkdir -p "$scratch/cfg"
exec dbus-run-session --config-file="$HERE/bus.conf" -- "$0" --inner "$scratch" "$@"
