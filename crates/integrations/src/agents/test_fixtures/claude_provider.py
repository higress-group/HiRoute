#!/usr/bin/env python3
"""A native-process fixture for the isolated provider challenge, never a product backend."""
import json
import os
import sys
import urllib.error
import urllib.request

CAPABLE = True
VERSION = "0.0.1"

if "--version" in sys.argv:
    print(VERSION + " (Claude Code)")
    raise SystemExit(0)

assert os.environ["CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"] == "1"
assert os.environ["CLAUDE_CONFIG_DIR"].startswith(os.environ["HOME"] + os.sep)
assert os.getcwd() != os.environ["HOME"]
assert "HIROUTE_RUN_TOKEN" not in os.environ
settings = json.load(open(sys.argv[sys.argv.index("--settings") + 1]))
route = dict(os.environ)
if not CAPABLE:
    route.update(settings["env"])
request = urllib.request.Request(
    route["ANTHROPIC_BASE_URL"] + "/v1/messages",
    json.dumps({"model": route["ANTHROPIC_MODEL"], "messages": []}).encode(),
    {"Authorization": "Bearer " + route["ANTHROPIC_AUTH_TOKEN"]},
)
try:
    with urllib.request.urlopen(request, timeout=2) as response:
        assert response.status == 200
        response.read()
except (OSError, urllib.error.URLError):
    raise SystemExit(1)
