#!/usr/bin/env python3
"""Bounded managed-checkout GC using local-rust's preview/apply safety policy.

Preview is the default. Ordinary age-based GC protects keep. Explicit timed expiry
bounds keep, preserves evidence and retains active owners or changed identities.
The caller's scheduler can run a bounded expiry pass without a cleanup daemon.
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time


spec = importlib.util.spec_from_file_location(
    "hiroute_local_rust_gc_runner", Path(__file__).with_name("local-rust.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
ID = re.compile(r"[0-9a-f]{32}\Z")


def sweep(store, apply=False, older_than_hours=48, limit=None, expire_after_hours=None):
    if expire_after_hours is not None:
        runner.expiry_hours(expire_after_hours)
    elif older_than_hours < 48:
        raise ValueError("Age must be at least 48 hours and limit must be positive")
    if limit is not None and limit < 1:
        raise ValueError("Age must be at least 48 hours and limit must be positive")
    cutoff = time.time() - older_than_hours * 3600
    policy = {} if expire_after_hours is None else dict(expire_after_hours=expire_after_hours)
    selected, retained, removed = [], [], []
    for path in sorted((store.root / "runs").glob("*/result.json")):
        rid = path.parent.name
        if not ID.fullmatch(rid):
            continue
        try:
            row = store.load(rid)
            if row.get("kind") != "managed" or row.get("status") != "terminal" or row.get("removed"):
                continue
            if expire_after_hours is None:
                finished = row.get("finished_at")
                if (isinstance(finished, bool) or not isinstance(finished, (int, float))
                        or not math.isfinite(finished)):
                    raise ValueError("Missing or invalid finish time")
                if finished >= cutoff:
                    continue
            else:
                if not (store.root / "checkouts" / rid).exists():
                    continue
                if runner.expiry_deadline(row, expire_after_hours) > time.time():
                    continue
            if row.get("id") != rid:
                raise ValueError("Record ID does not match its directory")
            candidate = store.preview_one(row, **policy)
            if candidate["state"] != "candidate":
                retained.append(dict(id=rid, state="retained", reason=candidate["reason"]))
                continue
            selected.append(rid)
            if apply:
                # The shared owner re-reads the record, checks the snapshot token,
                # exports evidence and holds both locks throughout deletion.
                removed.append(store.apply(rid, candidate["token"], **policy)["id"])
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            retained.append(dict(id=rid, state="retained", reason=str(error)))
        if limit is not None and len(selected) >= limit:
            break  # Bound attempts, including candidates refused during apply.
    return dict(mode="apply" if apply else "preview",
                older_than_hours=older_than_hours if expire_after_hours is None else None,
                **policy,
                selected=selected, removed=removed, retained=retained,
                free_gib=round(shutil.disk_usage(store.root).free / 1024 ** 3, 1))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="Apply the shared cleanup policy")
    age = parser.add_mutually_exclusive_group()
    age.add_argument("--older-than-hours", type=int)
    age.add_argument("--expire-after-hours", type=float,
                     help="Expire terminal managed builds, including keep, after completion or keep renewal")
    parser.add_argument("--limit", type=int, help="Maximum candidates attempted in this pass")
    parser.add_argument('--backend-workbench', action='append', default=[],
                        help='Also expire this HOME-relative SSH workbench on the execution host')
    args = parser.parse_args()
    older_than_hours = 48 if args.older_than_hours is None else args.older_than_hours
    if args.expire_after_hours is not None:
        try:
            runner.expiry_hours(args.expire_after_hours)
        except ValueError as error:
            parser.error(str(error))
    elif older_than_hours < 48:
        parser.error("Age must be at least 48 hours and limit must be positive")
    if args.limit is not None and args.limit < 1:
        parser.error("Age must be at least 48 hours and limit must be positive")
    if args.backend_workbench and args.expire_after_hours is None:
        parser.error('--backend-workbench requires --expire-after-hours')
    try:
        workbenches = list(dict.fromkeys(runner.remote.workbench_path(p) for p in args.backend_workbench))
    except ValueError as error:
        parser.error(str(error))
    report = sweep(runner.Store(), args.apply, older_than_hours, args.limit, args.expire_after_hours)
    failed = False
    remaining = None if args.limit is None else args.limit - len(report['selected'])
    if workbenches:
        report['backend_workbenches'] = []
    for workbench in workbenches:
        if remaining == 0:
            report['backend_workbenches'].append(dict(workbench=workbench, state='limit reached'))
            continue
        root = Path.home() / workbench
        try:
            if not root.is_dir() or not (root / 'runs').is_dir():
                raise ValueError('Configured backend workbench is missing')
            result = runner.remote.gc_terminal_runs(root, 0, args.apply, True,
                                                    args.expire_after_hours, remaining)
            report['backend_workbenches'].append(dict(workbench=workbench, **result))
            if remaining is not None:
                remaining -= len(result['candidates'])
            failed |= args.apply and len(result['cleaned']) != len(result['candidates'])
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
            report['backend_workbenches'].append(dict(workbench=workbench, error=str(error)))
            failed = True
    print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    return int(failed)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
