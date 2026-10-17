#!/usr/bin/env python3
"""Capture exact x64 Cargo test artifacts for separately assessed Windows replay."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
SCHEMA = "hiroute.windows-native-artifacts/v1"
CAPTURE_SCHEMA = "hiroute.windows-native-artifact-capture/v1"
TARGET = "x86_64-pc-windows-msvc"
PE_MACHINE = 0x8664
SHA = re.compile(r"^[0-9a-f]{40}$")
HASH = re.compile(r"^[0-9a-f]{64}$")
PORTABLE = re.compile(r"^[A-Za-z0-9_-][A-Za-z0-9._-]*$")
RESERVED = re.compile(r"^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)", re.I)
MAX_FILE = 2 * 1024 * 1024 * 1024
MAX_JSON = 32 * 1024 * 1024
SOURCE_FILES = (
    "scripts/build-release.py",
    "scripts/windows-release-toolchain.cmake",
    "crates/cli/src/service/windows/install.rs",
    "crates/host-runtime/src/standalone_install/windows.rs",
    "scripts/package-standalone.py",
    "scripts/package-standalone-windows.py",
    "scripts/windows-ci-package.py",
    "scripts/windows-headless-ci.ps1",
    "scripts/windows-native-artifacts.py",
    "scripts/windows-native-replay.ps1",
    "scripts/windows-native-scopes.ps1",
    "scripts/windows-upgrade-inputs.ps1",
    "scripts/windows-native-runtime-audit.py",
    "scripts/windows-agent-fixtures.py",
)


def absolute(path):
    return Path(os.path.abspath(path))


def no_reparse(path, missing=False):
    path = absolute(path)
    for part in reversed((path,) + tuple(path.parents)):
        try:
            facts = part.lstat()
        except FileNotFoundError:
            if missing:
                continue
            raise ValueError("artifact path is missing")
        if stat.S_ISLNK(facts.st_mode) or getattr(facts, "st_file_attributes", 0) & 0x400:
            raise ValueError("artifact path contains a symlink or reparse point")
    return path


def regular(path):
    path = no_reparse(path)
    if not stat.S_ISREG(path.stat().st_mode):
        raise ValueError("artifact is not a regular file")
    return path


def relative(value):
    if (not isinstance(value, str) or not value or len(value) > 1024
            or "\\" in value or ":" in value):
        raise ValueError("unsafe artifact relative path")
    path = PurePosixPath(value)
    if (path.is_absolute() or path.as_posix() != value
            or any(not PORTABLE.fullmatch(part) or part.endswith(".")
                   or RESERVED.match(part) for part in path.parts)):
        raise ValueError("unsafe artifact relative path")
    return value


def decode_json(encoded):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON key")
            result[key] = value
        return result

    return json.loads(encoded, object_pairs_hook=unique)


def json_load(path):
    path = regular(path)
    if path.stat().st_size > MAX_JSON:
        raise ValueError("JSON evidence is oversized")
    return decode_json(path.read_text(encoding="utf-8-sig"))


def closed(value, keys, label):
    if not isinstance(value, dict) or set(value) != set(keys):
        raise ValueError("invalid " + label)


def text_list(value, nonempty, label):
    if (not isinstance(value, list) or (nonempty and not value) or len(value) > 100000
            or any(not isinstance(item, str) or not item.strip() or len(item) > 4096
                   or any(ord(char) < 0x20 for char in item) for item in value)):
        raise ValueError("invalid " + label)
    return value


def recipe(value):
    closed(value, ("required_cases", "cargo_arguments", "harness_arguments", "filter"), "recipe")
    cases = text_list(value["required_cases"], True, "required cases")
    if len(set(cases)) != len(cases):
        raise ValueError("required cases contain duplicates")
    text_list(value["cargo_arguments"], True, "Cargo arguments")
    text_list(value["harness_arguments"], False, "harness arguments")
    selected = value["filter"]
    if selected is not None:
        text_list([selected], True, "filter")
    return value


def digest(path):
    value = hashlib.sha256()
    with regular(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def command(*arguments):
    return subprocess.check_output(arguments, cwd=ROOT, text=True).strip()


def checkout(expected):
    if not isinstance(expected, str) or not SHA.fullmatch(expected):
        raise ValueError("exact candidate SHA required")
    if command("git", "rev-parse", "HEAD") != expected:
        raise ValueError("checkout differs from expected SHA")
    command("git", "diff", "--exit-code", "HEAD", "--")
    command("git", "ls-files", "--error-unmatch", "--", "Cargo.lock", *SOURCE_FILES)
    return {
        "source_sha": expected,
        "cargo_lock_sha256": digest(ROOT / "Cargo.lock"),
        "source_files": {name: {"sha256": digest(ROOT / name),
                                "size": regular(ROOT / name).stat().st_size}
                         for name in SOURCE_FILES},
    }


def producer(expected):
    result = checkout(expected)
    if os.environ.get("HIROUTE_BUILD_SOURCE_SHA") != expected:
        raise ValueError("compiler source SHA environment is not bound")
    verbose = command("rustc", "-vV")
    hosts = re.findall(r"(?m)^host: (\S+)\r?$", verbose)
    if hosts != [TARGET]:
        raise ValueError("native Windows x64 rustc host required")
    result.update(target=TARGET, rustc_host=TARGET, rustc_verbose=verbose)
    return result


def packager():
    spec = importlib.util.spec_from_file_location("native_artifact_packager", ROOT / "scripts/package-standalone.py")
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def file_record(path, destination, kind, pe):
    path = regular(path)
    size = path.stat().st_size
    if not 0 < size <= MAX_FILE:
        raise ValueError("artifact has an invalid size")
    if pe:
        with path.open("rb") as stream:
            packager().verify_windows_stream(stream, TARGET)
    return {"path": relative(destination), "kind": kind, "size": size,
            "sha256": digest(path), "pe_machine": PE_MACHINE if pe else None}


def copy_bytes(source, evidence, destination, kind, pe=False):
    source = regular(source)
    destination = relative(destination)
    output = no_reparse(evidence / destination, missing=True)
    output.parent.mkdir(parents=True, exist_ok=True)
    no_reparse(output.parent)
    expected = file_record(source, destination, kind, pe)
    if output.exists():
        if file_record(output, destination, kind, pe) != expected:
            raise ValueError("artifact destination already contains different bytes")
        return expected
    temporary = None
    try:
        # copyfile/copystat/CopyFile are intentionally not used: upload/replay
        # restores only bytes and must establish fresh consumer ACLs itself.
        with source.open("rb") as input_stream:
            before = os.fstat(input_stream.fileno())
            with tempfile.NamedTemporaryFile(dir=output.parent, prefix=".capture-", delete=False) as output_stream:
                temporary = Path(output_stream.name)
                for chunk in iter(lambda: input_stream.read(1024 * 1024), b""):
                    output_stream.write(chunk)
                output_stream.flush()
                os.fsync(output_stream.fileno())
            after = os.fstat(input_stream.fileno())
        identity = lambda facts: (facts.st_dev, facts.st_ino, facts.st_size, facts.st_mtime_ns)
        if identity(before) != identity(after) or file_record(temporary, destination, kind, pe) != expected:
            raise ValueError("artifact changed while being captured")
        no_reparse(source)
        no_reparse(output, missing=True)
        os.replace(temporary, output)
        temporary = None
        return expected
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def write_json(path, value):
    path = no_reparse(path, missing=True)
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()
    if path.exists() and regular(path).read_bytes() == encoded:
        return
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".manifest-", delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        no_reparse(path, missing=True)
        os.replace(temporary, path)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def cargo_artifact(path):
    path = regular(path)
    if path.stat().st_size > MAX_JSON:
        raise ValueError("Cargo JSON is oversized")
    artifacts, finished = [], []
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        if not line.strip():
            continue
        event = decode_json(line)
        if not isinstance(event, dict):
            raise ValueError("invalid Cargo JSON event")
        if event.get("reason") == "build-finished":
            finished.append(event.get("success"))
        if (event.get("reason") == "compiler-artifact"
                and isinstance(event.get("profile"), dict)
                and event["profile"].get("test") is True
                and event.get("executable") is not None):
            artifacts.append(event)
    if finished != [True] or len(artifacts) != 1:
        raise ValueError("exactly one successfully built Cargo test executable required")
    event = artifacts[0]
    target = event.get("target")
    if (not isinstance(target, dict) or not isinstance(target.get("name"), str)
            or not PORTABLE.fullmatch(target["name"])):
        raise ValueError("invalid Cargo target")
    kinds = text_list(target.get("kind"), True, "Cargo target kinds")
    if len(set(kinds)) != len(kinds) or any(not PORTABLE.fullmatch(kind) for kind in kinds):
        raise ValueError("invalid Cargo target kinds")
    executable = event["executable"]
    if not isinstance(executable, str) or not executable or "\0" in executable:
        raise ValueError("invalid Cargo executable path")
    source = regular(ROOT / executable)
    try:
        source_artifact = source.relative_to(absolute(ROOT)).as_posix()
    except ValueError:
        raise ValueError("Cargo executable is outside this checkout") from None
    relative(source_artifact)
    if not source_artifact.startswith("target/") or source.suffix.lower() != ".exe":
        raise ValueError("checkout-local Cargo executable required")
    return source, source_artifact, {"name": target["name"], "kind": kinds}


def capture(args):
    name = relative(args.name)
    if "/" in name or len(name) > 128:
        raise ValueError("portable unique artifact name required")
    build = producer(args.expected_sha)
    evidence = no_reparse(args.evidence, missing=True)
    selected = recipe(json_load(args.recipe_json))
    source, source_artifact, target = cargo_artifact(args.cargo_json)
    destination = "native-binaries/" + name + "/" + source.name
    record = copy_bytes(source, evidence, destination, "test-executable", True)
    item = dict(selected, name=name, target=target, source_artifact=source_artifact,
                executable=destination)
    result = {"schema": CAPTURE_SCHEMA, "build": build, "recipe": item, "file": record}
    receipt = evidence / "native-recipes" / (name + ".json")
    if receipt.exists() and json_load(receipt) != result:
        raise ValueError("capture name already describes another recipe")
    if producer(args.expected_sha) != build:
        raise ValueError("producer changed while capturing")
    write_json(receipt, result)
    return result


def verify_file(evidence, value):
    closed(value, ("path", "kind", "size", "sha256", "pe_machine"), "artifact file")
    path = relative(value["path"])
    if not path.startswith("native-binaries/"):
        raise ValueError("artifact is outside the native binary inventory")
    if (type(value["size"]) is not int or not 0 < value["size"] <= MAX_FILE
            or not isinstance(value["sha256"], str) or not HASH.fullmatch(value["sha256"])):
        raise ValueError("invalid artifact size or hash")
    pe = value["kind"] in ("test-executable", "product-executable")
    if value["kind"] not in ("test-executable", "product-executable", "package-archive", "package-manifest", "package-evidence"):
        raise ValueError("unknown artifact file kind")
    if value["pe_machine"] != (PE_MACHINE if pe else None):
        raise ValueError("artifact PE architecture differs")
    if file_record(evidence / path, path, value["kind"], pe) != value:
        raise ValueError("artifact bytes differ from captured receipt")


def verified_package(result, expected):
    if (not isinstance(result, dict) or result.get("source_revision") != expected
            or result.get("target") != TARGET
            or result.get("deterministic_publication") != "green"):
        raise ValueError("package evidence belongs to another candidate or target")
    for key in ("archive", "manifest"):
        if not isinstance(result.get(key), str) or not result[key] or "\0" in result[key]:
            raise ValueError("invalid package producer path")
    archive = regular(result["archive"])
    manifest = regular(result["manifest"])
    verified = packager().verify(manifest, archive)
    if (verified["revision"] != expected or verified["target"] != TARGET
            or result.get("archive_sha256") != digest(archive)
            or result.get("manifest_sha256") != digest(manifest)):
        raise ValueError("verified package differs from candidate evidence")
    return archive, manifest


def finalize(args):
    build = producer(args.expected_sha)
    evidence = no_reparse(args.evidence)
    fragments = no_reparse(evidence / "native-recipes")
    recipes, files = [], []
    for path in sorted(fragments.iterdir()):
        if path.suffix != ".json":
            raise ValueError("unexpected capture receipt")
        item = json_load(path)
        closed(item, ("schema", "build", "recipe", "file"), "capture receipt")
        if item["schema"] != CAPTURE_SCHEMA or item["build"] != build:
            raise ValueError("capture receipt belongs to another producer")
        verify_file(evidence, item["file"])
        recipes.append(item["recipe"])
        files.append(item["file"])
    if not recipes:
        raise ValueError("native recipe inventory is empty")
    products = {}
    if args.include_product_binaries:
        for key, filename in (("hiroute", "hiroute.exe"), ("hirouted", "hirouted.exe"),
                              ("cpa_process_fixture", "hiroute-cpa-process-fixture.exe")):
            source = ROOT / "target/debug" / filename
            if key == "cpa_process_fixture" and not source.exists():
                continue
            destination = "native-binaries/product/" + filename
            files.append(copy_bytes(source, evidence, destination, "product-executable", True))
            products[key] = destination
    package = None
    if args.package_result is not None:
        result = json_load(args.package_result)
        archive, manifest = verified_package(result, args.expected_sha)
        producer_evidence = regular(manifest.parent.parent / "package-evidence.json")
        if json_load(producer_evidence) != result:
            raise ValueError("complete package evidence differs from producer result")
        package = {}
        for key, source in (("archive", archive), ("manifest", manifest), ("evidence", producer_evidence)):
            destination = "native-binaries/package/" + source.name
            files.append(copy_bytes(source, evidence, destination, "package-" + key))
            package[key] = destination
    value = {"schema": SCHEMA, "build": build, "recipes": recipes, "files": files,
             "product_binaries": products, "package": package, "runtime_state": "not_assessed"}
    if producer(args.expected_sha) != build:
        raise ValueError("producer changed while finalizing")
    verify_manifest(value, evidence, args.expected_sha)
    path = evidence / "native-artifacts.json"
    if path.exists() and json_load(path).get("build", {}).get("source_sha") != args.expected_sha:
        raise ValueError("existing manifest belongs to another candidate")
    write_json(path, value)
    return value


def verify_manifest(value, evidence, expected):
    closed(value, ("schema", "build", "recipes", "files", "product_binaries", "package", "runtime_state"), "native artifact manifest")
    if value["schema"] != SCHEMA or value["runtime_state"] != "not_assessed":
        raise ValueError("native artifact manifest cannot claim runtime acceptance")
    build = value["build"]
    closed(build, ("source_sha", "target", "rustc_host", "rustc_verbose", "cargo_lock_sha256", "source_files"), "build provenance")
    current = checkout(expected)
    if (any(build[key] != facts for key, facts in current.items()) or build["target"] != TARGET
            or build["rustc_host"] != TARGET or not isinstance(build["rustc_verbose"], str)
            or re.findall(r"(?m)^host: (\S+)\r?$", build["rustc_verbose"]) != [TARGET]):
        raise ValueError("artifact build SHA, lock, scripts or compiler host differ")
    if not isinstance(value["files"], list) or not value["files"]:
        raise ValueError("native file inventory is empty")
    inventory = {}
    for record in value["files"]:
        verify_file(evidence, record)
        path = record["path"]
        if path.casefold() in inventory:
            raise ValueError("native file inventory contains a duplicate path")
        inventory[path.casefold()] = record
    referenced, names = [], set()
    if not isinstance(value["recipes"], list) or not value["recipes"]:
        raise ValueError("native recipe inventory is empty")
    for item in value["recipes"]:
        closed(item, ("required_cases", "cargo_arguments", "harness_arguments", "filter", "name", "target", "source_artifact", "executable"), "captured recipe")
        recipe({key: item[key] for key in ("required_cases", "cargo_arguments", "harness_arguments", "filter")})
        name = relative(item["name"])
        if "/" in name or len(name) > 128 or name.casefold() in names:
            raise ValueError("native recipe name is invalid or duplicated")
        names.add(name.casefold())
        closed(item["target"], ("name", "kind"), "captured Cargo target")
        if (not isinstance(item["target"]["name"], str)
                or not PORTABLE.fullmatch(item["target"]["name"])):
            raise ValueError("captured Cargo target name is invalid")
        kinds = text_list(item["target"]["kind"], True, "captured Cargo target kinds")
        if len(set(kinds)) != len(kinds) or any(not PORTABLE.fullmatch(kind) for kind in kinds):
            raise ValueError("captured Cargo target kinds are invalid")
        source_artifact = relative(item["source_artifact"])
        if not source_artifact.startswith("target/") or not source_artifact.lower().endswith(".exe"):
            raise ValueError("captured Cargo source path is not checkout-local")
        executable = relative(item["executable"])
        if (not executable.startswith("native-binaries/" + name + "/")
                or inventory.get(executable.casefold(), {}).get("kind") != "test-executable"):
            raise ValueError("recipe has no matching captured test executable")
        referenced.append(executable)
    products = value["product_binaries"]
    if (not isinstance(products, dict) or set(products) - {"hiroute", "hirouted", "cpa_process_fixture"}
            or (products and not {"hiroute", "hirouted"}.issubset(products))):
        raise ValueError("incomplete product binary inventory")
    for path in products.values():
        if inventory.get(relative(path).casefold(), {}).get("kind") != "product-executable":
            raise ValueError("product binary has no matching captured file")
        referenced.append(path)
    if value["package"] is not None:
        package = value["package"]
        closed(package, ("archive", "manifest", "evidence"), "captured package")
        for key, path in package.items():
            if inventory.get(relative(path).casefold(), {}).get("kind") != "package-" + key:
                raise ValueError("package has no matching captured file")
            referenced.append(path)
        manifest_path, archive_path = evidence / package["manifest"], evidence / package["archive"]
        verified = packager().verify(manifest_path, archive_path)
        original = json_load(evidence / package["evidence"])
        # Original absolute producer paths are historical evidence only. Replay
        # uses the portable captured mapping above and establishes new ACLs.
        if (verified["revision"] != expected or verified["target"] != TARGET
                or original.get("source_revision") != expected or original.get("target") != TARGET
                or original.get("archive_sha256") != digest(archive_path)
                or original.get("manifest_sha256") != digest(manifest_path)
                or original.get("deterministic_publication") != "green"):
            raise ValueError("captured package differs from the build candidate")
    if (len(referenced) != len(set(path.casefold() for path in referenced))
            or set(path.casefold() for path in referenced) != set(inventory)):
        raise ValueError("native file inventory has missing or unreferenced files")
    actual = set()
    for directory, subdirs, filenames in os.walk(no_reparse(evidence / "native-binaries"), followlinks=False):
        no_reparse(directory)
        for name in subdirs:
            no_reparse(Path(directory) / name)
        for name in filenames:
            path = regular(Path(directory) / name)
            relative_path = relative(path.relative_to(evidence).as_posix()).casefold()
            if relative_path in actual:
                raise ValueError("actual native inventory contains a case collision")
            actual.add(relative_path)
    if actual != set(inventory):
        raise ValueError("actual native file inventory differs from manifest")
    return value


def verify(args):
    evidence = no_reparse(args.evidence)
    return verify_manifest(json_load(evidence / "native-artifacts.json"), evidence, args.expected_sha)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for verb in ("capture", "finalize", "verify"):
        child = commands.add_parser(verb)
        child.add_argument("--expected-sha", required=True)
        child.add_argument("--evidence", type=Path, required=True)
        if verb == "capture":
            child.add_argument("--name", required=True)
            child.add_argument("--cargo-json", type=Path, required=True)
            child.add_argument("--recipe-json", type=Path, required=True)
        if verb == "finalize":
            child.add_argument("--include-product-binaries", action="store_true")
            child.add_argument("--package-result", type=Path)
    args = parser.parse_args()
    try:
        value = {"capture": capture, "finalize": finalize, "verify": verify}[args.command](args)
        print(json.dumps({"schema": value["schema"], "source_sha": value["build"]["source_sha"],
                          "recipes": len(value.get("recipes", [value.get("recipe")]))}))
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
