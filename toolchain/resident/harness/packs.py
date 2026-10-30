"""packs.py: the sealed packs of crates/ra-jit-store, read and written as the store lays them out.

A sealed pack of format 3 is the magic, the count of blocks and the count of entries, 12 bytes
for each block (its length in the file, its length unpacked, and 0 when it is stored as it is or
1 when it is one zstd frame), 48 for each entry (the key, its block, where in the block its value
starts, its length, and the CRC-32 of key, length and value), then the blocks, which take the
rest of the file.
"""
import struct, subprocess, zlib

FORMAT = 3
MAGIC = b"rajitpk" + bytes([FORMAT])
BLOCK_HEADER, ENTRY = 12, 48


def checksum(key, value):
    return zlib.crc32(key + struct.pack("<I", len(value)) + value)


def sealed(records):
    """A sealed pack of these `(key, value)` records, each value a block of its own as it is."""
    out = bytearray(MAGIC + struct.pack("<II", len(records), len(records)))
    for _, value in records:
        out += struct.pack("<III", len(value), len(value), 0)
    for block, (key, value) in enumerate(records):
        out += key + struct.pack("<IIII", block, 0, len(value), checksum(key, value))
    for _, value in records:
        out += value
    return bytes(out)


def index(pack):
    """The blocks of a sealed pack, as `(at, stored, raw, packed)`, and its entries, as `(key,
    block, at, len, checksum)`; None when `pack` is not laid out as one."""
    if not pack.startswith(MAGIC) or len(pack) < len(MAGIC) + 8:
        return None
    count, entries = struct.unpack_from("<II", pack, len(MAGIC))
    at = len(MAGIC) + 8
    start = at + BLOCK_HEADER * count + ENTRY * entries
    if start > len(pack):
        return None
    blocks = []
    for _ in range(count):
        stored, raw, how = struct.unpack_from("<III", pack, at)
        if how not in (0, 1) or (how == 0 and stored != raw):
            return None
        blocks.append((start, stored, raw, how == 1))
        at, start = at + BLOCK_HEADER, start + stored
    if start != len(pack):
        return None
    listed = []
    for _ in range(entries):
        block, begins, length, crc = struct.unpack_from("<IIII", pack, at + 32)
        if block >= count or begins + length > blocks[block][2]:
            return None
        listed.append((pack[at:at + 32], block, begins, length, crc))
        at += ENTRY
    return blocks, listed


def blocks(pack):
    """The blocks of a sealed pack unpacked, the compressed ones by the zstd program."""
    laid = index(pack)
    if laid is None:
        raise ValueError("not a sealed pack of format 3")
    frames = b"".join(pack[at:at + stored] for at, stored, _, packed in laid[0] if packed)
    unpacked = subprocess.run(["zstd", "-dc"], input=frames, capture_output=True, check=True).stdout if frames else b""
    out, taken = [], 0
    for at, stored, raw, packed in laid[0]:
        if packed:
            out.append(unpacked[taken:taken + raw])
            taken += raw
        else:
            out.append(pack[at:at + stored])
    return out


def records(pack):
    """The `(key, value)` records of a sealed pack."""
    unpacked = blocks(pack)
    return [(key, unpacked[block][at:at + length]) for key, block, at, length, _ in index(pack)[1]]
