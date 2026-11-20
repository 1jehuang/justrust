"""Read-only admission for synthetic comparisons running on a constrained local disk."""
from dataclasses import dataclass
from pathlib import Path
import shutil


class SpaceDeferred(RuntimeError):
    pass


def filesystem(path):
    path = Path(path)
    if not path.is_absolute():
        raise ValueError(f"bench: ambiguous relative storage path {path}")
    while True:
        try:
            dev = path.stat().st_dev
            break
        except FileNotFoundError:
            if path.is_symlink() or path == path.parent:
                raise
            path = path.parent
    if not path.is_dir():
        raise ValueError(f"bench: not a storage directory: {path}")
    return dev, shutil.disk_usage(path).free


@dataclass
class PairSpace:
    floor: int
    arm_bytes: dict[str, int]

    def validate(self, names):
        names = list(names)
        if (len(set(names)) < 2 or len(set(names)) != len(names) or set(names) != set(self.arm_bytes)
                or any(type(v) is not int or v <= 0 for v in [self.floor, *self.arm_bytes.values()])):
            raise ValueError("bench: constrained pairs require a positive floor and an explicit positive byte bound for each distinct arm")

    def admit(self, paths, remaining, point):
        # Each bound covers ALL growth for its arm. Charge it conservatively on
        # every filesystem the arm writes, but never twice on the same device.
        remaining = set(remaining)
        devices = {}
        for arm, roots in paths.items():
            for root in roots:
                try:
                    dev, free = filesystem(root)
                except (OSError, ValueError) as exc:
                    raise SpaceDeferred(f"bench: point {point} deferred: storage unknown at {root}: {exc}") from exc
                prev, arms = devices.setdefault(dev, (free, set()))
                devices[dev] = min(prev, free), arms
                arms.add(arm)
        for dev, (free, arms) in devices.items():
            required = self.floor + sum(self.arm_bytes[a] for a in arms & remaining)
            if free <= required:
                raise SpaceDeferred(f"bench: point {point} deferred: filesystem {dev} has {free} bytes free, "
                                    f"requires more than {required} including floor {self.floor}")
