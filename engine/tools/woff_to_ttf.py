"""Unwraps a WOFF 1.0 font (zlib-compressed tables) into the plain TrueType file the in-game menu embeds.
No dependency beyond the standard library:  python tools/woff_to_ttf.py in.woff out.ttf"""
import struct
import sys
import zlib


def woff_to_ttf(data: bytes) -> bytes:
    sig, flavor, _length, num, _res, _total, *_rest = struct.unpack(">4sIIHHI", data[:20])
    if sig != b"wOFF":
        raise SystemExit("not a WOFF 1.0 file")
    tables = []
    for i in range(num):
        tag, off, comp, orig, csum = struct.unpack(">4sIIII", data[44 + 20 * i:64 + 20 * i])
        blob = data[off:off + comp]
        if comp < orig:
            blob = zlib.decompress(blob)
        if len(blob) != orig:
            raise SystemExit(f"table {tag!r}: {len(blob)} bytes, expected {orig}")
        tables.append((tag, csum, blob))
    tables.sort(key=lambda t: t[0])
    pow2 = 1
    while pow2 * 2 <= num:
        pow2 *= 2
    out = bytearray(struct.pack(">IHHHH", flavor, num, pow2 * 16, pow2.bit_length() - 1, num * 16 - pow2 * 16))
    pos = 12 + 16 * num
    body = bytearray()
    for tag, csum, blob in tables:
        out += struct.pack(">4sIII", tag, csum, pos + len(body), len(blob))
        body += blob + b"\0" * (-len(blob) % 4)
    return bytes(out + body)


if __name__ == "__main__":
    with open(sys.argv[1], "rb") as f:
        ttf = woff_to_ttf(f.read())
    with open(sys.argv[2], "wb") as f:
        f.write(ttf)
    print(f"{sys.argv[2]}: {len(ttf)} bytes")
