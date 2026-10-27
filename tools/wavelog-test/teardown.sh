#!/usr/bin/env bash
# Remove a test Wavelog made by setup.sh: the pod, its containers and volumes, and its secrets.
set -euo pipefail
POD=${POD:-wavelog-test}
STATE=${STATE:-$HOME/.wavelog-test-$POD}
podman pod rm -f "$POD" > /dev/null 2>&1 || true
for v in dbdata config uploads userdata; do podman volume rm -f "$POD-$v" > /dev/null 2>&1 || true; done
rm -rf "$STATE"
echo "removed $POD"
