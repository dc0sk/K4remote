# A local Wavelog for logging tests (FR-LOG)

K4 Remote's write tests for Wavelog logging run against a **throwaway local Wavelog**, never
against a real logbook. These scripts make one with rootless podman, bound to `127.0.0.1`:

```sh
tools/wavelog-test/setup.sh              # pod "wavelog-test" on http://127.0.0.1:8086/
tools/wavelog-test/teardown.sh           # remove it: pod, volumes, secrets
POD=wavelog-x PORT=8087 tools/wavelog-test/setup.sh   # a second one beside it
```

`setup.sh` runs `mariadb:11.8` and `ghcr.io/wavelog/wavelog:latest` as in Wavelog's Docker
guide, completes the web installer with `install.py` (test user `N0CALL`, locator `JO31AA`, one
station location, id 1), and inserts two API v2 tokens the way Wavelog's own model makes them
(`wl2_` + 40 hex, SHA-256 stored):

- `FULL`: `qso:read`, `qso:write`, `qso:delete`, `station:read`, `lookup:read`
- `LOOKUP_ONLY`: `lookup:read` (to test a missing scope)

They are written to `~/.wavelog-test-<pod>/tokens.env` (mode 600) with the URL:

```sh
. ~/.wavelog-test-wavelog-test/tokens.env
curl -H "Authorization: Bearer $FULL" "${URL}index.php/api/v2/token"
```

Requires podman (rootless) and network access to pull the two images. Setup takes about a minute.
