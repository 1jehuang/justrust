#!/usr/bin/env python3
"""Offline acceptance provenance check, not a performance claim."""
import hashlib
import json
from pathlib import Path
import tarfile

ROOT = Path(__file__).resolve().parent

def main():
    manifest = json.loads((ROOT / 'sha256.json').read_text())
    for name, expected in manifest.items():
        assert hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == expected, name
    report = json.loads((ROOT / 'acceptance.json').read_text())
    assert report['verdict'] == report['semantic_verdict'] == report['native_hit_verdict'] == 'passed'
    assert report['completed_cases'] == report['planned_cases'] == 32
    assert report['verify_ich'] and report['native_hit_required']
    count = 0
    for observation in report['observations']:
        modes = observation['modes']
        assert len(modes) == 7
        reference = modes['fresh-no']
        for mode, result in modes.items():
            for family in ('compile', 'runtime', 'dependency'):
                if family in reference:
                    assert family in result
                    for key in ('status', 'stdout_b64', 'stderr_b64'):
                        assert reference[family][key] == result[family][key], (mode, family, key)
            count += 1
    assert count == 224
    evidence = report['native_evidence']
    assert evidence['proof']['inferred_provider_avoided'] == {'warm-yes': 1, 'body-edit-yes': 1}
    assert len(evidence['observations']) == 5
    assert report['identity_before'] == report['identity_after']
    with tarfile.open(ROOT / 'native-profiles.tar.gz') as archive:
        traces = [v for v in archive.getmembers() if v.name.endswith('.mm_profdata')]
        assert len(traces) == 5
    print(json.dumps({'verdict': 'passed', 'semantic_compilations': count,
                      'native_profile_processes': 5, 'scope': 'correctness and positive native reuse only'}))

if __name__ == '__main__':
    main()
