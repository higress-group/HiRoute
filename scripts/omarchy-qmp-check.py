#!/usr/bin/env python3
import csv
import io
import json
import re
import socket
import subprocess
import sys
import time


def tokens(value):
    return re.findall(r"\d+(?:\.\d+)+|[a-z0-9]+", value.lower())


def recognize(screenshot):
    enlarged = subprocess.run(
        ["convert", screenshot, "-resize", "200%", "-colorspace", "Gray", "png:-"],
        check=True, capture_output=True, timeout=10,
    ).stdout
    ocr = subprocess.run(
        ["tesseract", "stdin", "stdout", "--psm", "11", "tsv"],
        input=enlarged, check=True, capture_output=True, timeout=15,
    )
    return list(csv.DictReader(io.StringIO(ocr.stdout.decode()), delimiter="\t", quoting=csv.QUOTE_NONE))


def matches_screen(rows, guards):
    screen = " " + " ".join(token for row in rows for token in tokens(row.get("text") or "")) + " "
    matched = [
        next((option for option in guard.split("|") if " " + " ".join(tokens(option)) + " " in screen), None)
        for guard in guards
    ]
    return matched if all(matched) else None


def locate_button(rows, label):
    lines = {}
    for row in rows:
        if row["level"] == "5":
            line = tuple(row[key] for key in ("block_num", "par_num", "line_num"))
            lines.setdefault(line, []).extend((token, row) for token in tokens(row.get("text") or ""))
    expected = tokens(label)
    matches = []
    for line in lines.values():
        for offset in range(len(line) - len(expected) + 1):
            window = line[offset:offset + len(expected)]
            if [token for token, _ in window] == expected:
                matches.append([row for _, row in window])
    if len(matches) != 1:
        return None
    left = min(int(row["left"]) for row in matches[0])
    top = min(int(row["top"]) for row in matches[0])
    right = max(int(row["left"]) + int(row["width"]) for row in matches[0])
    bottom = max(int(row["top"]) + int(row["height"]) for row in matches[0])
    page = next(row for row in rows if row["level"] == "1")
    return ((left + right) // 2, (top + bottom) // 2, int(page["width"]), int(page["height"]))


path, action, *arguments = sys.argv[1:]
with socket.socket(socket.AF_UNIX) as connection:
    connection.settimeout(15)
    connection.connect(path)
    with connection.makefile("rwb") as messages:
        assert "QMP" in json.loads(messages.readline())

        def command(name, **arguments):
            messages.write(json.dumps({"execute": name, "arguments": arguments}).encode() + b"\n")
            messages.flush()
            while True:
                result = json.loads(messages.readline())
                if "return" in result:
                    return result["return"]
                if "error" in result:
                    raise RuntimeError(result["error"])

        command("qmp_capabilities")
        if action == "kvm":
            status = command("query-kvm")
            assert status["enabled"] and status["present"], status
            print(json.dumps(status, sort_keys=True))
        elif action == "screenshot":
            command("screendump", filename=arguments[0])
        elif action in ("locate", "await-screen"):
            screenshot, timeout, *criteria = arguments
            label, *guards = criteria if action == "locate" else ("", *criteria)
            deadline = time.monotonic() + int(timeout)
            while time.monotonic() < deadline:
                command("screendump", filename=screenshot)
                rows = recognize(screenshot)
                matched = matches_screen(rows, guards)
                if matched is not None:
                    if not label:
                        print(*matched, sep="\n")
                        break
                    bounds = locate_button(rows, label)
                    if bounds is not None:
                        print(*bounds)
                        break
                time.sleep(1)
            else:
                raise SystemExit(f"The expected HiRoute screen did not appear: {label or guards[0]}")
        elif action == "click":
            x, y, width, height = map(int, arguments)
            assert 1 < width and 1 < height and 0 <= x < width and 0 <= y < height
            command("input-send-event", events=[
                {"type": "abs", "data": {"axis": "x", "value": round(x * 32767 / (width - 1))}},
                {"type": "abs", "data": {"axis": "y", "value": round(y * 32767 / (height - 1))}},
            ])
            time.sleep(0.15)
            for down in (True, False):
                command("input-send-event", events=[{"type": "btn", "data": {"button": "left", "down": down}}])
                time.sleep(0.08)
        elif action in ("install", "confirm"):
            keys = ("tab", "tab", "spc") if action == "install" else ("tab", "spc")
            for key in keys:
                command("send-key", keys=[{"type": "qcode", "data": key}])
                time.sleep(0.25)
        else:
            raise ValueError(action)
