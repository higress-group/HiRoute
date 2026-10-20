"""v0.3.1 package -> current startup and managed Agent attribution, without DB writes.

Run with the repository and verified Linux x86_64 release archive as arguments.
Current binaries come from HIROUTE_VALIDATION_PRODUCT_BIN_DIR or target/debug.
Native probes and the external upstream use the existing bounded fixtures; all
connection transactions, grant issuance, upgrade and requests use real binaries.
"""
import hashlib
import http.client
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import time

from publication_product import Product, encoded
from publication_process import bootstrap, configure_model_settings_v2
from publication_requests import request

RELEASE_REVISION = "13856abd873da5b4373d39322493b4cc105c4867"
PACKAGE_SHA256 = "dfa35ada12fcef9d0d2e10e680422cd0bf6a0454a8809c27879b74ba932d21aa"


def message(product, token, prompt):
    client = http.client.HTTPConnection("127.0.0.1", product.port, timeout=45)
    try:
        client.request("POST", "/v1/messages", body=encoded({
            "model": product.model_alias, "max_tokens": 16,
            "messages": [{"role": "user", "content": prompt}],
        }), headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
        response = client.getresponse()
        return response.status, response.read()
    finally:
        client.close()


def sessions(product):
    deadline = time.monotonic() + 10
    while True:
        _, result = product.public_cli("sessions list --include-unlinked --limit 50", success=False)
        if result["status"] == "succeeded":
            return result["data"]["sessions"]
        assert result["status"] == "unavailable" and time.monotonic() < deadline, result
        time.sleep(.1)


def upstream_fixture(product, contents):
    binary = product.cpa_fixture / "cpa_upstream.py"
    binary.write_bytes(contents)
    product.cpa_args = [
        "--cpa-binary", str(binary), "--cpa-sha256", hashlib.sha256(contents).hexdigest(),
    ]


def run(repository, archive):
    archive = Path(archive).resolve()
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == PACKAGE_SHA256, "wrong released package"
    candidate = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=repository, text=True).strip()
    product = Product(repository)
    current_bin = product.bin
    try:
        released = product.root / "released"
        released.mkdir(mode=0o700)
        # Only extract the authenticated archive. Do not overwrite caller-owned paths.
        with tarfile.open(archive) as package:
            for member in package.getmembers():
                assert not Path(member.name).is_absolute() and ".." not in Path(member.name).parts
                assert member.isfile() or member.isdir(), "unexpected release archive member"
            package.extractall(released)
        product.bin = released / "bin"
        product.enable_cpa()
        # Each daemon pins its own external-component version. Use that release's
        # exact fixture before upgrade, then the candidate fixture after replacing
        # the daemon; do not weaken either component handshake.
        upstream_fixture(product, subprocess.check_output([
            "git", "show", RELEASE_REVISION + ":crates/daemon/tests/support/cpa_upstream.py",
        ], cwd=repository))
        bootstrap(product)
        connections = {"agent_codex_default": product.agent_connection}
        product.agent_context_id = None
        configure_model_settings_v2(
            product, [product.plan_id], "released-claude", agent_id="agent_claude_default")
        connections["agent_claude_default"] = product.agent_connection
        tokens = {agent: product.bearer(connection) for agent, connection in connections.items()}
        for agent, token in tokens.items():
            send = request if agent == "agent_codex_default" else None
            status, body = (send(product, product.model_alias, "before upgrade " + agent, token)
                            if send else message(product, token, "before upgrade " + agent))
            assert status == 200 and b"fixture answer" in body, (agent, status, body)
        assert len(sessions(product)) == 2, "released requests did not persist"
        native_before = {path: path.read_bytes() for path in
                         (product.settings, product.codex_settings)}
        product.stop()
        product.bin = current_bin
        upstream_fixture(product, (
            Path(repository) / "crates/daemon/tests/support/cpa_upstream.py"
        ).read_bytes())
        product.env["HIROUTE_RECOVERY_PACKAGE"] = str(archive)
        for restart in range(2):
            product.start()
            for path, original in native_before.items():
                assert path.read_bytes() == original, "startup rewrote native settings"
            for agent, old_token in tokens.items():
                assert product.bearer(connections[agent]) == old_token, "startup rotated a grant"
                prompt = f"after upgrade {restart} {agent}"
                status, body = (request(product, product.model_alias, prompt, old_token)
                                if agent == "agent_codex_default"
                                else message(product, old_token, prompt))
                assert status == 200 and b"fixture answer" in body, (agent, status, body)
            deadline = time.monotonic() + 10
            while True:
                observed = sessions(product)
                if len(observed) == 2 * (restart + 1):
                    break
                assert time.monotonic() < deadline, observed
                time.sleep(.1)
            assert {row["agent_id"] for row in observed} == set(connections), observed
            assert all(row["agent_id"] for row in observed), observed
            product.stop()
        print(json.dumps({
            "scenario": "released-connected-agent-attribution-upgrade", "state": "green",
            "source_revision": RELEASE_REVISION, "package_sha256": PACKAGE_SHA256,
            "candidate": candidate, "released_sessions": 2, "current_attributed_sessions": 4,
            "agents": sorted(connections), "current_startups": 2,
            "native_settings_and_grants_unchanged": True,
        }), flush=True)
    finally:
        product.close()


if __name__ == "__main__":
    run(sys.argv[1], sys.argv[2])
