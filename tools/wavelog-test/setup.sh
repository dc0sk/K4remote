#!/usr/bin/env bash
# A throwaway local Wavelog for K4 Remote's logging tests (FR-LOG): MariaDB + Wavelog as a rootless
# podman pod on 127.0.0.1 only, installed by script, with a test user N0CALL, one station location
# and two API v2 tokens. Never point K4 Remote's write tests at a real logbook.
#
#   tools/wavelog-test/setup.sh            # pod "wavelog-test" on 127.0.0.1:8086
#   POD=wavelog-x PORT=8087 tools/wavelog-test/setup.sh
#
# Secrets go to $STATE (default ~/.wavelog-test-<pod>/, mode 700): the DB and admin passwords and
# tokens.env (FULL=…, LOOKUP_ONLY=…). Remove everything with teardown.sh.
set -euo pipefail
POD=${POD:-wavelog-test}
PORT=${PORT:-8086}
STATE=${STATE:-$HOME/.wavelog-test-$POD}
HERE=$(cd "$(dirname "$0")" && pwd)
URL="http://127.0.0.1:$PORT/"

if podman pod exists "$POD"; then
  echo "pod $POD exists already; run teardown.sh first" >&2
  exit 1
fi
mkdir -p "$STATE" && chmod 700 "$STATE"
rnd() { head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n'; }
DBPW=$(rnd); ADMINPW=$(rnd)
printf '%s\n' "$DBPW" > "$STATE/dbpw"; printf '%s\n' "$ADMINPW" > "$STATE/adminpw"
chmod 600 "$STATE/dbpw" "$STATE/adminpw"

podman pod create --name "$POD" -p "127.0.0.1:$PORT:80" > /dev/null
podman run -d --pod "$POD" --name "$POD-db" \
  -e MARIADB_RANDOM_ROOT_PASSWORD=yes -e MARIADB_DATABASE=wavelog -e MARIADB_USER=wavelog \
  -e MARIADB_PASSWORD="$DBPW" -v "$POD-dbdata:/var/lib/mysql" docker.io/library/mariadb:11.8 > /dev/null
podman run -d --pod "$POD" --name "$POD-main" -e CI_ENV=docker \
  -v "$POD-config:/var/www/html/application/config/docker" \
  -v "$POD-uploads:/var/www/html/uploads" -v "$POD-userdata:/var/www/html/userdata" \
  ghcr.io/wavelog/wavelog:latest > /dev/null

# Wait for the web server (bounded: 5 minutes).
for _ in $(seq 150); do
  code=$(curl -s -o /dev/null -w '%{http_code}' "${URL}install/" || true)
  [ "$code" = 200 ] || [ "$code" = 302 ] && break
  sleep 2
done
# And for the database to take connections (the installer needs it).
for _ in $(seq 150); do
  podman exec "$POD-db" mariadb -uwavelog -p"$DBPW" -e 'SELECT 1' wavelog > /dev/null 2>&1 && break
  sleep 2
done

python3 -I "$HERE/install.py" "$URL" "$DBPW" "$ADMINPW"

# API v2 tokens, made as Wavelog's own Api_v2_model does: "wl2_" + 40 hex, SHA-256 stored.
FULL="wl2_$(head -c 20 /dev/urandom | od -An -tx1 | tr -d ' \n')"
LOOKUP="wl2_$(head -c 20 /dev/urandom | od -An -tx1 | tr -d ' \n')"
hf=$(printf %s "$FULL" | sha256sum | cut -d' ' -f1)
hl=$(printf %s "$LOOKUP" | sha256sum | cut -d' ' -f1)
podman exec "$POD-db" mariadb -uwavelog -p"$DBPW" wavelog -e \
  "INSERT INTO api_token (user_id, created_by, token_name, token_hash, scopes) VALUES
   (1,1,'k4remote-full','$hf','qso:read,qso:write,qso:delete,station:read,lookup:read'),
   (1,1,'k4remote-lookup','$hl','lookup:read');"
printf 'URL=%s\nFULL=%s\nLOOKUP_ONLY=%s\n' "$URL" "$FULL" "$LOOKUP" > "$STATE/tokens.env"
chmod 600 "$STATE/tokens.env"

status=$(curl -s "${URL}index.php/api/v2/status")
echo "$status" | grep -q '"status":"ok"' || { echo "API v2 not answering: $status" >&2; exit 1; }
owner=$(curl -s -H "Authorization: Bearer $FULL" "${URL}index.php/api/v2/token" | grep -o '"owner":"[^"]*"')
echo "Wavelog ready at $URL ($owner); tokens in $STATE/tokens.env"
