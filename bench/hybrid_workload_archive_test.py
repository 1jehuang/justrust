#!/usr/bin/env python3
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

import hybrid_workload_archive as archive


class ReceiptArchive(unittest.TestCase):
    def test_only_receipts_and_matching_manifest(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'a5/source').mkdir(parents=True)
            (root / 'a5/source/private.rs').write_text('excluded')
            (root / 'a5/report.json').write_text('{"ok":true}')
            (root / 'a5/cold.log').write_text('full raw output\n')
            (root / 'a5/.env').write_text('excluded secret')
            stream = io.BytesIO()
            archive.archive(root, ['a5'], stream)
            stream.seek(0)
            with tarfile.open(fileobj=stream, mode='r:gz') as bundle:
                self.assertEqual(set(bundle.getnames()), {'a5/report.json', 'a5/cold.log', 'RECEIPTS.json'})
                manifest = json.load(bundle.extractfile('RECEIPTS.json'))
                self.assertEqual(manifest['files']['a5/cold.log']['bytes'], 16)

    def test_reject_secret_and_symlink(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'a5').mkdir()
            path = root / 'a5/raw.log'
            path.write_text('Authorization: Bearer abcdefghijklmnop')
            with self.assertRaisesRegex(ValueError, 'secret'):
                archive.receipts(root, ['a5'])
            path.unlink()
            path.symlink_to('/etc/hosts')
            with self.assertRaisesRegex(ValueError, 'regular'):
                archive.receipts(root, ['a5'])

    def test_reject_path_traversal(self):
        with self.assertRaisesRegex(ValueError, 'simple'):
            archive.receipts(Path('/unused'), ['../outside'])


if __name__ == '__main__':
    unittest.main()
