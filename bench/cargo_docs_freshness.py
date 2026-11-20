#!/usr/bin/env python3
"""Compare Cargo mtime/content freshness on the real Jcode embedded-docs crate.

Runs explicit Cargo/rustc binaries in private target directories, never touching
live project sources. Pass an immutable Jcode commit for repeatable inputs.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time


def run(args):
    repo = args.repo.resolve()
    cargo, rustc = args.cargo.resolve(), args.rustc.resolve()
    env = dict(os.environ, RUSTC=str(rustc))
    commit = subprocess.check_output(['git', '-C', repo, 'rev-parse', args.rev + '^{commit}'], text=True).strip()
    with tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR')) as tmp:
        root = Path(tmp)
        paths = ['README.md', 'docs', 'crates/jcode-docs-data']
        archive = root / 'inputs.tar'
        subprocess.run(['git', '-C', repo, 'archive', '-o', archive, commit, *paths], check=True)
        subprocess.run(['tar', '-xf', archive, '-C', root], check=True)
        pkg = root / paths[-1]
        source = pkg / 'src/lib.rs'
        initial = source.read_bytes()
        ignored = root / 'docs/.justrust-unconsumed-probe'
        if ignored.exists():
            raise ValueError('probe filename already exists')
        rows = []
        for method in ['mtime', 'content']:
            options = [] if method == 'mtime' else ['-Zchecksum-freshness', '--config', 'build.fingerprint="content"']
            cmd = [cargo, *options, 'build', '--target-dir', root / method, '--message-format=json']
            source.write_bytes(initial)
            ignored.touch()
            subprocess.run(cmd, cwd=pkg, env=env, stdout=subprocess.DEVNULL, check=True)
            for edit in ['noop', 'retouch', 'content', 'ignored-doc']:
                times, fresh = [], []
                for i in range(args.samples):
                    if edit == 'retouch':
                        source.touch()
                    elif edit == 'content':
                        source.write_bytes(initial + f'\nconst _: usize = {i};\n'.encode())
                    elif edit == 'ignored-doc':
                        ignored.touch()
                    start = time.perf_counter()
                    output = subprocess.check_output(cmd, cwd=pkg, env=env)
                    times.append(time.perf_counter() - start)
                    [artifact] = [x for x in map(json.loads, output.splitlines()) if x['reason'] == 'compiler-artifact' and x['target']['name'] == 'jcode_docs_data']
                    fresh.append(artifact['fresh'])
                expected = edit == 'noop' or (edit == 'retouch' and method == 'content')
                if edit != 'ignored-doc' and any(value != expected for value in fresh):
                    raise RuntimeError(f'{method}/{edit}: unexpected fresh flags {fresh}')
                rows.append(dict(method=method, edit=edit, wall=times, median=statistics.median(times), fresh=fresh))
    return dict(revision=commit, cargo_sha256=hashlib.sha256(cargo.read_bytes()).hexdigest(), rustc_version=subprocess.check_output([rustc, '-vV'], text=True), rows=rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path.home() / 'jcode')
    parser.add_argument('--rev', required=True)
    parser.add_argument('--cargo', required=True, type=Path)
    parser.add_argument('--rustc', required=True, type=Path)
    parser.add_argument('--samples', type=int, default=5)
    args = parser.parse_args()
    if args.samples < 1:
        parser.error('--samples must be positive')
    print(json.dumps(run(args), indent=2))
