#!/usr/bin/env bash
# Fetch the current stable HiRoute standalone package for this Docker host architecture,
# verify its digest and refresh .env so the compose build picks up the new image.
#
# Usage: ./update.sh
#   HIROUTE_ARCH=x86_64 ./update.sh   # override autodetected architecture
set -euo pipefail
cd "$(dirname "$0")"

ARCH="${HIROUTE_ARCH:-$(uname -m)}"
case "$ARCH" in
    x86_64|amd64) HIROUTE_ARCH=x86_64 ;;
    aarch64|arm64) HIROUTE_ARCH=aarch64 ;;
    *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
esac
export HIROUTE_ARCH

python3 - <<'PY'
import hashlib
import json
import os
import pathlib
import re
import shutil
import sys
import urllib.request

arch = os.environ["HIROUTE_ARCH"]
root = pathlib.Path.cwd()


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": "HiRoute-Compose-Update/1"})
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read()


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


install_script = fetch("https://hiroute.ai/install.sh").decode("utf-8")
pattern = re.compile(
    r"manifest_url='https://hiroute\.ai/releases/([^/]+)/"
    r"HiRoute-\1-linux-(x86_64|aarch64)\.tar\.gz\.json'"
)
match = next((item for item in pattern.finditer(install_script) if item.group(2) == arch), None)
if match is None:
    raise SystemExit(f"No stable HiRoute package is published for {arch}")
version, manifest_url = match.group(1), match.group(0).split("=", 1)[1].strip("'")

manifest = json.loads(fetch(manifest_url))
if manifest.get("version") != version or manifest.get("target", "").startswith(arch) is False:
    raise SystemExit("Manifest version/architecture mismatch")
archive = manifest["archive"]["filename"]
archive_sha = manifest["archive"]["sha256"]
archive_size = manifest["archive"]["size"]
cpa_sha = manifest["cpa"]["sha256"]

archive_path = root / archive
if archive_path.exists():
    if sha256_bytes(archive_path.read_bytes()) != archive_sha:
        print(f"Replacing {archive} (digest mismatch)")
        archive_path.unlink()
    else:
        print(f"Using existing {archive}")
if not archive_path.exists():
    print(f"Downloading {archive} ({archive_size} bytes) ...")
    data = fetch(f"https://hiroute.ai/releases/{version}/{archive}")
    if len(data) != archive_size or sha256_bytes(data) != archive_sha:
        raise SystemExit("Downloaded archive failed verification")
    archive_path.write_bytes(data)

env_path = root / ".env"
if not env_path.exists():
    shutil.copyfile(root / "env.example", env_path)
    print("Created .env from env.example; edit ADVERTISE_ADDRESS before first boot")
values = {
    "HIROUTE_VERSION": version,
    "ARCHIVE": archive,
    "ARCHIVE_SHA256": archive_sha,
    "CPA_SHA256": cpa_sha,
}
text = env_path.read_text(encoding="utf-8")
for key, value in values.items():
    if re.search(rf"^{key}=.*$", text, re.MULTILINE):
        text = re.sub(rf"^{key}=.*$", f"{key}={value}", text, flags=re.MULTILINE)
    else:
        text += f"{key}={value}\n"
env_path.write_text(text, encoding="utf-8")
print(f"Updated .env to HiRoute {version} ({arch})")
PY

cat <<'EOF'

Rebuild and restart:
  docker compose build
  docker compose up -d
  docker exec -it hiroute hiroute console open
EOF