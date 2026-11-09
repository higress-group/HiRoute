"""Check a saved Plan and its receipts across the native KVM upgrade."""

import ast
import hashlib

import http.client
import json
import os
from pathlib import Path
import shutil
import sys
import time
import tomllib
import traceback
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent / "support"))

from standalone_headless_product import (
    InstalledProduct, MODEL, NativeUpstream, NATIVE_TOKEN, native_draft
)


def event(case, **facts):
    print(json.dumps({"case": case, **facts}), flush=True)


def apply_file(product, arguments, file, family):
    product.cli(*arguments, "--apply-file", str(file))
    outcome = product.cli(*family, "apply", "--request-file", str(file))
    assert outcome["operation"]["state"] == "succeeded"


def prepare_fixture():
    staged = Path(__file__).resolve().parent
    source = (staged / "support" / "standalone_headless_product.py").read_text()
    script, = (
        ast.literal_eval(call.args[0])
        for call in ast.walk(ast.parse(source))
        if isinstance(call, ast.Call)
        and isinstance(call.func, ast.Attribute)
        and call.func.attr == "write_text"
        and isinstance(call.func.value, ast.Name)
        and call.func.value.id == "codex"
    )
    home = Path.home()
    codex = home / ".local" / "bin" / "codex"
    catalog = home / ".codex" / "models_cache.json"
    for path in (codex, catalog, catalog.with_name("config.toml")):
        if path.exists() or path.is_symlink():
            raise FileExistsError(path)
    codex.parent.mkdir(parents=True, exist_ok=True)
    catalog.parent.mkdir(mode=0o700, exist_ok=True)
    with codex.open("x") as output:
        output.write(script)
    codex.chmod(0o700)
    with catalog.open("xb") as output, (staged / "codex_bundled_catalog.json").open("rb") as data:
        shutil.copyfileobj(data, output)
    catalog.chmod(0o600)


def seed(product):
    directory = Path.home() / ".codex"
    directory.mkdir(mode=0o700, exist_ok=True)
    with (directory / "config.toml").open("x") as file:
        file.write(
            'model = ' + json.dumps(MODEL) + '\n'
            'model_provider = "fixture_native"\n'
            '[model_providers.fixture_native]\n'
            'name = "Fixture native source"\n'
            'base_url = ' + json.dumps(product.upstream.base_url) + '\n'
            'wire_api = "responses"\n'
            'experimental_bearer_token = ' + json.dumps(NATIVE_TOKEN) + '\n'
            'requires_openai_auth = false\n'
        )
    candidate_ref = "candidate/native/headless-product"
    product.register_secret(candidate_ref, NATIVE_TOKEN)
    draft = native_draft(product.upstream)
    draft["display_name"] = "Omarchy KVM fixture provider"
    checked = product.cli("compute", "connection", "test", "--request-stdin", payload={
        "kind": "native", "request": {
            "draft": draft, "input_candidate": {"candidate_ref": candidate_ref, "candidate_revision": 1}
        }
    })["data"]
    candidate = checked["candidate"]["candidate"]
    apply_file(product, (
        "compute", "connection", "preview", "--candidate", candidate["candidate_ref"],
        "--candidate-revision", str(candidate["candidate_revision"]), "--all-selectable"
    ), product.project / "source-apply.json", ("compute", "connection"))
    sources = product.cli("compute", "list")["data"]["sources"]
    source = next(item for item in sources if item["display_name"] == draft["display_name"])
    binding = source["models"][0]["binding_id"]
    event("source-saved", result="passed")
    product.cli("protected-input", "release", "--candidate", candidate_ref)
    apply_file(product, (
        "routing", "preview", "--name", "Omarchy KVM fixture route", "--purpose", "Isolated desktop acceptance",
        "--mode", "fixed_model", "--model", binding,
    ), product.project / "route-apply.json", ("routing",))
    plans = product.cli("routing", "list")["data"]["plans"]
    assert len(plans) == 1
    plan = plans[0]["head"]["reference"]["plan_id"]
    event("plan-saved", result="passed")
    target = next(
        target for target in product.cli("quickstart")["data"]["targets"]
        if target["agent_id"] == "agent_codex_default"
    )
    context = target["codex_access"]["root_context_id"]
    apply_file(product, (
        "agents", "connect", "preview", "--context", context, "--plan", plan,
        "--native-models", "hiroute_only", "--native-default", "plan",
    ), product.project / "agent-apply.json", ("agents", "connect"))
    managed_config = (directory / "config.toml").read_bytes()
    return (source["source_id"], plan, tomllib.loads(managed_config.decode()),
            hashlib.sha256(managed_config).hexdigest())


def receipt_ids(product):
    ids = set()
    data = product.cli("sessions", "list", "--include-unlinked", "--limit", "50",
                       retry_reader_busy=True)["data"]
    for summary in data["sessions"]:
        detail = product.cli("sessions", "show", summary["session_id"],
                             retry_reader_busy=True)["data"]
        for turn in detail["turns"]:
            ids.update(turn["receipt_ids"])
    return ids


def gateway(product, seed_facts, sequence, previous_receipt):
    source_id, plan, config, baseline_config_hash = seed_facts
    managed_config = (Path.home() / ".codex/config.toml").read_bytes()
    config_hash = hashlib.sha256(managed_config).hexdigest()
    assert config_hash == baseline_config_hash, "The managed Codex config changed"
    assert any(source["source_id"] == source_id
               for source in product.cli("compute", "list")["data"]["sources"])
    assert any(item["head"]["reference"]["plan_id"] == plan
               for item in product.cli("routing", "list")["data"]["plans"])
    native = config["model_providers"]["hiroute"]
    url = urlsplit(native["base_url"])
    assert url.scheme == "http" and url.hostname == "127.0.0.1"
    session = f"omarchy-kvm-{sequence}"
    existing_receipts = receipt_ids(product)
    if previous_receipt is not None:
        assert previous_receipt in existing_receipts, "The baseline receipt was not preserved"
    with product.upstream.lock:
        before = len(product.upstream.requests)
    connection = http.client.HTTPConnection(url.hostname, url.port, timeout=30)
    try:
        connection.request("POST", url.path.rstrip("/") + "/responses", json.dumps({
            "model": config["model"], "input": "isolated Omarchy KVM request", "stream": False
        }), {
            "Content-Type": "application/json", "Session-Id": session, **native["http_headers"]
        })
        reply = connection.getresponse()
        body = reply.read()
        assert reply.status == 200 and b"native product answer" in body, reply.status
    finally:
        connection.close()
    with product.upstream.lock:
        assert len(product.upstream.requests) == before + 1
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        for receipt_id in receipt_ids(product) - existing_receipts:
            receipt = product.cli("sessions", "receipt", receipt_id,
                                  retry_reader_busy=True)["data"]
            facts = [event["fact"] for event in receipt["ordered_facts"]]
            route = next((fact for fact in facts if fact["kind"] == "route_decision"), None)
            usage = next((fact for fact in facts if fact["kind"] == "usage_and_cache"), None)
            if route and route["plan_id"] == plan and usage:
                assert usage["input_tokens"] == 4 and usage["output_tokens"] == 3
                event("gateway", result="passed", sequence=sequence, source_preserved=True,
                      plan_preserved=True, status=200, fixture_requests=1, observed=True,
                      previous_receipt_preserved=previous_receipt in existing_receipts,
                      managed_codex_sha256=config_hash)
                return receipt_id
        time.sleep(0.2)
    raise AssertionError("Gateway result was not present in observation readback")


def main():
    assert os.getuid() != 0
    product = object.__new__(InstalledProduct)
    product.project = Path(__file__).resolve().parent / "business-private"
    product.project.mkdir(mode=0o700, exist_ok=False)
    product.cli_path = Path.home() / ".local/bin/hiroute"
    product.env = os.environ.copy()
    product.daemon = None
    product.outputs = []
    product.upstream = NativeUpstream()
    stage = "seed"
    try:
        prepare_fixture()
        facts = seed(product)
        previous_receipt = None
        event(stage, result="passed", sources=1, plans=1, managed_codex_sha256=facts[3])
        for count, command in enumerate(sys.stdin, 1):
            stage = command.strip()
            if stage == "request":
                previous_receipt = gateway(product, facts, count, previous_receipt)
            elif stage == "exit":
                assert count == 3 and previous_receipt is not None
                return
            else:
                raise ValueError("Unknown KVM witness command")
        raise EOFError("The witness stopped before both requests completed")
    except Exception as error:
        with (product.project / "private-diagnostic.txt").open("x") as log:
            traceback.print_exc(file=log)
        event(stage, result="failed", error_type=type(error).__name__)
        raise SystemExit(1)
    finally:
        product.upstream.close()


if __name__ == "__main__":
    os.umask(0o077)
    main()
