#!/usr/bin/env python3
"""Adds the managed mark to PNG icons, idempotently.

`src/linux/icons.rs` and the installers treat a PNG carrying the literal bytes
`X-Hex-Managed=true` as written by Voinc: the app replaces only marked icons in
the user's icon theme, and `install-linux.sh` removes only them. The brand art
in `.voinc/assets/` carries no mark (it is raw artwork); the mark is added to
the shipped `packaging/linux/icons` set by the P9 patch, regenerated with this
tool after the copies it patches:

    python3 .voinc/managed-mark.py packaging/linux/icons/hicolor/*/apps/*.png
"""
import sys
import zlib

MARK = b"X-Hex-Managed=true"


def marked(data: bytes) -> bool:
    return MARK in data


def add_mark(data: bytes) -> bytes:
    """Inserts one tEXt chunk (`Comment\\0X-Hex-Managed=true`) before IEND."""
    if marked(data):
        return data
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    head, tail = data[:8], data[8:]
    end = tail.rindex(b"IEND")
    assert end >= 4, "no chunk header before IEND"
    text = b"Comment\x00" + MARK
    chunk = len(text).to_bytes(4, "big") + b"tEXt" + text
    chunk += zlib.crc32(b"tEXt" + text).to_bytes(4, "big")
    return head + tail[:end - 4] + chunk + tail[end - 4:]


for path in sys.argv[1:]:
    with open(path, "rb") as f:
        original = f.read()
    updated = add_mark(original)
    if updated == original:
        print(f"already marked: {path}")
        continue
    with open(path, "wb") as f:
        f.write(updated)
    print(f"marked: {path}")
