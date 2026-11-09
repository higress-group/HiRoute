#!/usr/bin/env python3
import json
import socket
import sys
import time

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
        elif action == "install":
            for key in ("tab", "tab", "spc"):
                command("send-key", keys=[{"type": "qcode", "data": key}])
                time.sleep(0.25)
        else:
            raise ValueError(action)
