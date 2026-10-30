#!/usr/bin/env python3
"""Offline mutation checks using one already-derived raw-profile observation set."""
import copy
import gzip
import json
from pathlib import Path
import tempfile
import unittest

import validate as v


class EvidenceMutations(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.blobs = v.load_small(v.ROOT)
        cls.decoded = v.load_json(cls.blobs, 'observations.json')
        v.validate_structure(cls.blobs, cls.decoded)

    def setUp(self):
        self.blobs = dict(self.blobs)
        self.decoded = copy.deepcopy(self.decoded)
        self.state = v.load_json(self.blobs, 'state.json')

    def reject(self):
        self.blobs['state.json'] = json.dumps(self.state).encode()
        with self.assertRaises((AssertionError, KeyError, ValueError)):
            v.validate_structure(self.blobs, self.decoded)

    def group(self, key='owned-cgu-traversal-edit', label=v.REVISIT):
        return next(g for g in self.decoded[key]['intervals'] if g['label'] == label)

    def test_baseline(self):
        self.assertEqual(v.validate_structure(self.blobs, self.decoded)['verdict'], 'passed')

    def test_missing_row(self):
        self.state['rows'].pop()
        self.reject()

    def test_reordered_rows(self):
        self.state['rows'][0], self.state['rows'][1] = self.state['rows'][1], self.state['rows'][0]
        self.reject()

    def test_driver_drift(self):
        self.state['identities_after']['rustc_driver_0']['sha256'] = '0' * 64
        self.reject()

    def test_forged_driver_both_sides(self):
        for side in ('identities_before', 'identities_after'):
            self.state[side]['rustc_driver_0']['sha256'] = '0' * 64
        self.reject()

    def test_source_not_restored(self):
        self.state['source_after']['src/summary.rs'] = v.digest(self.blobs['source.edited'])
        self.reject()

    def test_original_source_corrupted(self):
        self.blobs['source/src/summary.rs'] += b'\n'
        self.reject()

    def test_edit_bytes_corrupted(self):
        self.blobs['source.edited'] = self.blobs['source.original']
        self.reject()

    def test_wrong_edit_source_hash(self):
        self.state['rows'][1]['source_sha256'] = v.digest(self.blobs['source.original'])
        self.reject()

    def test_disabled_flag_changed(self):
        self.state['rows'][1]['environment']['RUSTFLAGS'] += ' -Zjr-cgu-shared-inline-traversal=yes'
        self.reject()

    def test_cache_enabled(self):
        self.state['rows'][4]['environment']['JUSTRUST_DEPCACHE'] = '1'
        self.reject()

    def test_runtime_hash_mismatch(self):
        self.blobs['logs/owned-off-edit.raw-output.jsonl'] += b'\n'
        self.reject()

    def test_runtime_marker_removed_with_consistent_receipt(self):
        key = 'owned-cgu-traversal-edit'
        path = 'logs/' + key + '.raw-output.jsonl'
        raw = self.blobs[path].replace(b'JR_RUNTIME_EXECUTED_47', b'JR_RUNTIME_EXECUTED_48')
        self.blobs[path] = self.blobs['journals/' + key + '/output.jsonl'] = raw
        self.state['rows'][4]['raw_output']['sha256'] = v.digest(raw)
        self.reject()

    def test_failed_test_hidden_in_parsed_summary(self):
        self.state['rows'][4]['parsed']['failed'] = 1
        self.reject()

    def test_missing_revisit(self):
        self.decoded['owned-cgu-traversal-edit']['intervals'].remove(self.group())
        self.reject()

    def test_duplicate_revisit(self):
        self.group()['intervals_ns'].append(self.group()['intervals_ns'][0])
        self.group()['records'] += 1
        self.reject()

    def test_revisit_on_wrong_thread(self):
        group = self.group()
        group['thread'] += 1
        row = self.state['rows'][4]
        next(g for g in row['profiles'][0]['intervals'] if g['label'] == v.REVISIT)['thread'] = group['thread']
        self.reject()

    def test_revisit_outside_place_same_elapsed(self):
        # Shift an interval without changing its duration/count. The compact
        # profiler summary still agrees, so interval containment must catch it.
        group = self.group()
        group['intervals_ns'] = [[a + 10**12, b + 10**12] for a, b in group['intervals_ns']]
        self.reject()

    def test_wrong_traversal_count(self):
        self.group(label=v.TRAVERSAL)['records'] -= 1
        self.reject()

    def test_unresolved_labels_cannot_prove_zero(self):
        self.decoded['owned-off-edit']['unresolved_records'] = 1
        self.state['rows'][1]['profiles'][0]['unresolved_records'] = 1
        self.reject()

    def test_profile_command_flag_missing(self):
        self.decoded['owned-cgu-traversal-edit']['metadata']['cmd'] = self.decoded['owned-cgu-traversal-edit']['metadata']['cmd'].replace(' -Zjr-cgu-shared-inline-traversal=yes', '')
        self.reject()

    def test_export_missing(self):
        path = next(p for p in self.blobs if p.startswith('exports/') and 'owned-off-' in p and p.endswith('.jsonl'))
        del self.blobs[path]
        self.reject()

    def test_export_duplicate(self):
        path = next(p for p in self.blobs if p.startswith('exports/') and 'owned-off-' in p and p.endswith('.jsonl'))
        self.blobs[path] += self.blobs[path].splitlines(keepends=True)[0]
        self.reject()

    def test_runner_failure(self):
        self.blobs['runner/profile47.exit'] = b'1\n'
        self.reject()

    def test_profile_header_corruption(self):
        with self.assertRaises(ValueError):
            v.decode_profile(b'not a measureme profile')

    def test_gzip_corruption(self):
        data = bytearray(gzip.compress(b'recorded profile', mtime=0))
        data[-8] ^= 1
        with self.assertRaises((OSError, EOFError)):
            gzip.decompress(data)

    def test_inventory_hash_corruption(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'receipt').write_bytes(b'original')
            (root / 'sha256.json').write_text(json.dumps({'receipt': v.digest(b'original')}))
            v.inventory(root)
            (root / 'receipt').write_bytes(b'mutated')
            with self.assertRaises(AssertionError):
                v.inventory(root)

    def test_interval_union_counts_overlap_once(self):
        self.assertEqual(v.union_ns([[1, 8], [2, 4], [8, 10], [12, 14]]), 11)
        with self.assertRaises(AssertionError):
            v.union_ns([[8, 1]])


if __name__ == '__main__':
    unittest.main(verbosity=2)
