"""Offline selected-test .new lifetime replay. No cache, hosted service or compiler.

Measures st_blocks * 512 while populated private snapshot files are live,
including the copy being identified. Excludes compression and publications.
"""
from pathlib import Path
from tempfile import TemporaryDirectory


def peak(root, hits, release):
    with TemporaryDirectory(dir=root) as scratch:
        scratch = Path(scratch)
        maximum = 0
        for i, hit in enumerate(hits):
            path = scratch / f"{i}.new"
            path.write_bytes(bytes([i + 1]) * (1 << 20))
            maximum = max(maximum, sum(p.stat().st_blocks * 512 for p in scratch.iterdir()))
            if hit and release:
                path.unlink()
        return maximum


if __name__ == "__main__":
    with TemporaryDirectory() as root:
        for name, hits in (("warm", [True] * 8), ("mixed", [True, False] * 4)):
            before, after = (peak(root, hits, release) for release in (False, True))
            print(f"{name}: peak allocated snapshot bytes {before} -> {after}")
            assert 0 < after < before
