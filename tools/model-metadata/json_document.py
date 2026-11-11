"""Deterministic JSON artifacts with whitespace outside strings for line tools."""
import json


def document(value, *, sort_keys=True):
    raw = json.dumps(value, sort_keys=sort_keys, ensure_ascii=False,
                     separators=(',', ':')).encode()
    result = bytearray()
    quoted = escaped = False
    column = 0
    for byte in raw:
        result.append(byte)
        column += 1
        if byte == ord(',') and not quoted and column >= 65536:
            result.append(ord('\n'))
            column = 0
        if quoted:
            if escaped:
                escaped = False
            elif byte == ord('\\'):
                escaped = True
            elif byte == ord('"'):
                quoted = False
        elif byte == ord('"'):
            quoted = True
    result.append(ord('\n'))
    return bytes(result)
