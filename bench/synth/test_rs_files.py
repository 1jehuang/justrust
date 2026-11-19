"""Package enumeration preserves probe ordering without opening excluded trees."""
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import gen


class ProbeFilesTests(unittest.TestCase):
    def test_pruning_order_links_and_special_targets(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            excluded = ['src/target', 'examples', 'benches', 'member.rs']
            keep = ['src/a.rs', 'src/.rs', 'a.rs', '.hidden/z.rs']
            special = dict(custom='custom-build', demo='example', perf='bench')
            targets = []
            for name, kind in special.items():
                targets.append(dict(src_path=str(base / (name + '.rs')), kind=[kind]))
            for name in keep + [n + '/bad.rs' for n in excluded] + [n + '.rs' for n in special]:
                path = base / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.touch()
            (base / 'linked-dir.rs').symlink_to(base / 'src')
            (base / 'linked-file.rs').symlink_to(base / 'a.rs')
            (base / 'broken.rs').symlink_to(base / 'absent')
            p, nested = [gen.Pkg(d.name, d, d / 'Cargo.toml', targets, True)
                         for d in (base, base / 'member.rs')]
            ws = gen.Workspace('fixture', 'snap', base, base, {q.name: q for q in (p, nested)}, [], [])
            with patch('os.scandir', wraps=os.scandir) as scan:
                paths = gen.rs_files(p, ws)
            self.assertFalse({base / n for n in excluded} & {Path(call.args[0]) for call in scan.call_args_list})
            self.assertEqual(paths, sorted(base / n for n in keep + ['member.rs', 'linked-dir.rs', 'linked-file.rs', 'broken.rs']))
            with self.assertRaises(FileNotFoundError):
                gen.candidates(base / 'broken.rs')
            with self.assertRaises(IsADirectoryError):
                gen.candidates(base / 'member.rs')
