#!/usr/bin/env python3
"""Complete Wavelog's web installer non-interactively (local test instance only)."""
import http.cookiejar, json, re, sys, urllib.parse, urllib.request

BASE, DBPW, ADMINPW = sys.argv[1], sys.argv[2], sys.argv[3]
jar = http.cookiejar.CookieJar()
op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))

def req(path, data=None, headers=None, timeout=600):
    body = urllib.parse.urlencode(data).encode() if data is not None else None
    r = urllib.request.Request(BASE + path, data=body, headers=headers or {})
    with op.open(r, timeout=timeout) as resp:
        return resp.read().decode()

page = req("install/index.php")
form_token = re.search(r'name="form_token" value="([0-9a-f]+)"', page).group(1)
fields = {
    "db_hostname": "127.0.0.1", "db_name": "wavelog", "db_username": "wavelog", "db_password": DBPW,
    "directory": "", "websiteurl": BASE, "username": "k4test", "password": ADMINPW,
    "cnfm_password": ADMINPW, "user_email": "k4test@example.invalid", "callsign": "N0CALL",
    "userlocator": "JO31AA", "firstname": "K4", "lastname": "Test", "timezone": "0",
    "dxcc": "230", "city": "Test", "userlanguage": "english", "log_threshold": "0",
    "global_call_lookup": "", "callbook_username": "", "callbook_password": "", "callbook_token": "",
}
run = req("install/run.php", dict(fields, form_token=form_token))
token = re.search(r"let _installer_token = '([0-9a-f]+)'", run).group(1)
H = {"X-Installer-Token": token, "X-Requested-With": "XMLHttpRequest"}
nested = {f"data[{k}]": v for k, v in fields.items()}
for step in ["run_config_file", "run_database_file", "run_database_tables"]:
    out = req("install/ajax.php", dict(nested, **{step: 1}), H)
    print(step, "->", out.strip()[:80])
    if out.strip() != "success":
        sys.exit(f"{step} failed")
mig = json.loads(req("index.php/migrate", headers={"X-Requested-With": "XMLHttpRequest"}))
print("migrate ->", mig)
cron = req("install/ajax.php", {"run_cron_token": 1}, H).strip()
dx = req("index.php/update/dxcc", headers={"X-Wavelog-Auth": cron}).strip()
print("dxcc ->", dx[:80])
print("lock ->", req("install/ajax.php", {"run_installer_lock": 1}, H).strip()[:80])
