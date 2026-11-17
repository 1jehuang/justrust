#!/usr/bin/env python3
"""Lossless receipt archives over authenticated `justrust records`.

Schema v1 is exactly {schema, encoding, original_bytes, original_sha256,
compressed_bytes, payload}. Payload is canonical base64 of one gzip member.
Hashes bind exact source bytes, not parsed/reformatted JSON. SHA256 is integrity,
not an authentication signature: authentication is provided by the existing
account-private records CLI and the caller's independently pinned source hash.
No backend limits change. Incompressible oversized inputs fail closed.

put SOURCE ID uploads and then fetches and verifies exact reconstruction.
get ID OUTPUT --sha256 HASH fetches and creates OUTPUT exclusively after checking.
Existing source/output files are never rewritten. Records retain API retention.
"""
import argparse
import base64
import binascii
import hashlib
import json
import re
import os
import selectors
import time
import subprocess
import tempfile
import zlib
from pathlib import Path

SCHEMA = "justrust.receipt-archive.v1"
MAX_ORIGINAL_BYTES = 64 * 1024 * 1024
MAX_COMPRESSED_BYTES = 1_400_000
MAX_ENVELOPE_BYTES = 1_900_000  # below the existing 1,980,000 data cap
MAX_RESPONSE_BYTES = 2_000_000
FIELDS = {"schema", "encoding", "original_bytes", "original_sha256",
          "compressed_bytes", "payload"}


def _hash(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise ValueError("expected lowercase SHA256")
    return value


def _json(value):
    return json.dumps(value, ensure_ascii=True, separators=(",", ":")).encode("ascii")


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON field")
        result[key] = value
    return result


def read_bounded(path, limit):
    with Path(path).open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("input exceeds byte bound")
    return data


def encode(original):
    """Create a bounded envelope without parsing or changing original bytes."""
    if not isinstance(original, bytes) or len(original) > MAX_ORIGINAL_BYTES:
        raise ValueError("original exceeds byte bound or is not bytes")
    compressor = zlib.compressobj(9, zlib.DEFLATED, 31)
    chunks = []
    size = 0
    for offset in range(0, len(original), 65536):
        chunk = compressor.compress(original[offset:offset + 65536])
        size += len(chunk)
        if size > MAX_COMPRESSED_BYTES:
            raise ValueError("compressed receipt exceeds byte bound")
        chunks.append(chunk)
    chunk = compressor.flush()
    size += len(chunk)
    if size > MAX_COMPRESSED_BYTES:
        raise ValueError("compressed receipt exceeds byte bound")
    chunks.append(chunk)
    envelope = {"schema": SCHEMA, "encoding": "gzip+base64",
                "original_bytes": len(original),
                "original_sha256": hashlib.sha256(original).hexdigest(),
                "compressed_bytes": size,
                "payload": base64.b64encode(b"".join(chunks)).decode("ascii")}
    if len(_json(envelope)) > MAX_ENVELOPE_BYTES:
        raise ValueError("envelope exceeds byte bound")
    return envelope


def decode(envelope, expected_sha256):
    """Bound decompression before allocation and verify independently pinned hash."""
    _hash(expected_sha256)
    if not isinstance(envelope, dict) or set(envelope) != FIELDS:
        raise ValueError("unexpected archive fields")
    if envelope["schema"] != SCHEMA or envelope["encoding"] != "gzip+base64":
        raise ValueError("unsupported archive schema or encoding")
    for field, maximum in (("original_bytes", MAX_ORIGINAL_BYTES),
                           ("compressed_bytes", MAX_COMPRESSED_BYTES)):
        if type(envelope[field]) is not int or not 0 <= envelope[field] <= maximum:
            raise ValueError("invalid archive byte bound")
    if _hash(envelope["original_sha256"]) != expected_sha256:
        raise ValueError("archive does not match pinned SHA256")
    payload = envelope["payload"]
    if not isinstance(payload, str) or len(payload) > 4 * ((MAX_COMPRESSED_BYTES + 2) // 3):
        raise ValueError("base64 payload exceeds byte bound")
    if len(_json(envelope)) > MAX_ENVELOPE_BYTES:
        raise ValueError("envelope exceeds byte bound")
    try:
        compressed = base64.b64decode(payload, validate=True)
    except (ValueError, binascii.Error) as error:
        raise ValueError("invalid base64") from error
    if base64.b64encode(compressed).decode("ascii") != payload:
        raise ValueError("noncanonical base64")
    if len(compressed) != envelope["compressed_bytes"]:
        raise ValueError("compressed size mismatch")
    inflater = zlib.decompressobj(31)
    try:
        original = inflater.decompress(compressed, envelope["original_bytes"] + 1)
    except zlib.error as error:
        raise ValueError("invalid gzip") from error
    if (len(original) != envelope["original_bytes"] or not inflater.eof
            or inflater.unconsumed_tail or inflater.unused_data):
        raise ValueError("gzip size, termination, or trailing data mismatch")
    if hashlib.sha256(original).hexdigest() != expected_sha256:
        raise ValueError("reconstructed SHA256 mismatch")
    return original


def _id(record_id):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", record_id):
        raise ValueError("invalid record id")
    return record_id


def _command(binary, args, output_limit):
    # Bounded chunks under a wall deadline, without an unbounded communicate.
    with subprocess.Popen([binary, "records", *args], stdout=subprocess.PIPE,
                          stderr=None) as process:
        deadline = time.monotonic() + 90
        chunks = []
        size = 0
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                while True:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0 or not selector.select(remaining):
                        raise ValueError("records command timeout")
                    chunk = os.read(process.stdout.fileno(), min(65536, output_limit + 1 - size))
                    if not chunk:
                        break
                    chunks.append(chunk)
                    size += len(chunk)
                    if size > output_limit:
                        raise ValueError("records response exceeds byte bound")
            if process.wait(timeout=max(0.001, deadline - time.monotonic())) != 0:
                raise ValueError("authenticated records command failed")
            return b"".join(chunks)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()


def fetch(record_id, expected_sha256, binary="justrust"):
    """Always use authenticated account-private GET, not local upload data."""
    record_id = _id(record_id)
    raw = _command(binary, ["show", "bench", record_id], MAX_RESPONSE_BYTES)
    record = json.loads(raw, object_pairs_hook=_pairs)
    if not isinstance(record, dict) or record.get("kind") != "bench" or record.get("id") != record_id:
        raise ValueError("record identity mismatch")
    return decode(record.get("data"), expected_sha256)


def upload(path, record_id, binary="justrust"):
    original = read_bounded(path, MAX_ORIGINAL_BYTES)
    envelope = encode(original)
    record_id = _id(record_id)
    with tempfile.TemporaryDirectory(prefix="jr-receipt-archive-") as directory:
        archive = Path(directory) / "envelope.json"
        archive.write_bytes(_json(envelope))
        _command(binary, ["put", "bench", record_id, str(archive)], 4096)
    restored = fetch(record_id, envelope["original_sha256"], binary)
    if restored != original:
        raise ValueError("authenticated reconstruction differs from source")
    if read_bounded(path, MAX_ORIGINAL_BYTES) != original:
        raise ValueError("original source changed during archival")
    return {"kind": "bench", "id": record_id,
            "original_bytes": len(original), "original_sha256": envelope["original_sha256"],
            "verified_authenticated_reconstruction": True}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--justrust", default="justrust")
    commands = parser.add_subparsers(dest="command", required=True)
    put = commands.add_parser("put")
    put.add_argument("source", type=Path)
    put.add_argument("id")
    get = commands.add_parser("get")
    get.add_argument("id")
    get.add_argument("output", type=Path)
    get.add_argument("--sha256", required=True)
    args = parser.parse_args(argv)
    if args.command == "put":
        print(json.dumps(upload(args.source, args.id, args.justrust), sort_keys=True))
    else:
        original = fetch(args.id, args.sha256, args.justrust)
        with args.output.open("xb") as output:
            output.write(original)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
