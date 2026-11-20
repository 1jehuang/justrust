#!/usr/bin/env python3
"""Paired project-side experiment. No Cargo patch or live project edits.

The launcher fingerprints exactly the membership read by this build.rs. Its
private env is scoped to one Cargo invocation. Stock invocations still watch
recursively. `include_str!` remains responsible for actual document bytes.
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

PIN = '27d2a732bb1f9ac852507f27d0611b20876fd7fd'
KEY = 'JCODE_DOCS_MEMBERSHIP_V1'
OLD = '    println!("cargo:rerun-if-changed={}", docs_dir.display());'
NEW = f'''    println!("cargo:rerun-if-env-changed={KEY}");
    if env::var_os("{KEY}").is_none() {{
    {OLD}
    }}'''


def membership(docs):
    # DirEntry::metadata in Rust does not follow entry symlinks. read_dir
    # itself follows a symlink naming the directory. Match both behaviors.
    with os.scandir(os.fsencode(docs)) as entries:
        names = sorted(e.name for e in entries if e.name.endswith(b'.md') and e.name != b'.md'
                       and e.is_file(follow_symlinks=False))
    return hashlib.sha256(b'\0'.join([os.fsencode(docs.resolve(strict=True)), *names])).hexdigest()


def experiment(args, root):
    all_rows = {}
    modes = ('recursive', 'precise')
    for mode in reversed(modes) if args.reverse else modes:
        tree = root / mode
        tree.mkdir()
        archive = tree / 'inputs.tar'
        pkg = tree / 'crates/jcode-docs-data'
        subprocess.run(['git', '-C', args.repo, 'archive', '-o', archive, PIN, 'README.md',
                        'docs', 'crates/jcode-docs-data'], check=True)
        subprocess.run(['tar', '-xf', archive, '-C', tree], check=True)
        script = pkg / 'build.rs'
        if mode == 'precise':
            text = script.read_text()
            assert text.count(OLD) == 1
            script.write_text(text.replace(OLD, NEW))
        docs = tree / 'docs'
        probe = docs / 'justrust-probe.md'
        renamed = probe.with_name('justrust-renamed.md')
        ignored = docs / 'justrust-probe.txt'
        assert not any(p.exists() for p in (probe, renamed, ignored))
        ignored.write_text('initial')
        cmd = [args.cargo, 'build', '--offline', '--message-format=json']
        rows = []

        def build(label, expected, launcher=True):
            start = time.perf_counter()
            env = dict(os.environ, RUSTC=str(args.rustc))
            env['CARGO_TARGET_DIR'] = str(pkg / 'target')
            env.pop(KEY, None)
            if mode == 'precise' and launcher:
                try:
                    env[KEY] = membership(docs)
                except OSError:
                    pass  # Native build.rs reports the original error.
            p = subprocess.run(cmd, cwd=pkg, env=env, capture_output=True)
            elapsed = time.perf_counter() - start
            if label == 'missing':
                assert p.returncode and b'No such file' in p.stderr, p.stderr
                return
            if p.returncode:
                raise RuntimeError(p.stderr.decode())
            artifact, = [r for r in map(json.loads, p.stdout.splitlines())
                         if r['reason'] == 'compiler-artifact' and r['target']['kind'] == ['lib']]
            assert artifact['fresh'] == expected, (mode, label, artifact)
            if label in ('add', 'content', 'rename', 'delete', 'symlink'):
                generated, = (pkg / 'target/debug/build').glob('*/out/jcode_docs.rs')
                listing = generated.read_text()
                for path in (probe, renamed):
                    assert (f'"docs/{path.name}"' in listing) == (path.is_file() and not path.is_symlink())
                if label in ('add', 'content'):
                    assert probe.read_bytes() in Path(artifact['filenames'][0]).read_bytes()
            if label:
                rows.append((label, elapsed))

        build(None, False)
        for i in range(args.samples):
            ignored.write_text(str(i))
            build('ignored', mode == 'precise')
            probe.write_text('justrust-docs-probe-added-086291')
            build('add', False)
            probe.write_text('justrust-docs-probe-changed-946122')
            build('content', False)
            probe.rename(renamed)
            build('rename', False)
            renamed.unlink()
            build('delete', False)
        probe.symlink_to(ignored)
        build('symlink', mode == 'precise')
        probe.unlink()
        docs.rename(tree / 'moved')
        build('missing', False)
        docs.symlink_to(tree / 'moved', target_is_directory=True)
        build('restore', False)
        build(None, mode == 'recursive', launcher=False)
        ignored.write_text('fallback')
        build('fallback', False, launcher=False)
        all_rows[mode] = rows
    for mode, rows in all_rows.items():
        samples = [sum(t for _, t in rows[i:i+5]) for i in range(0, args.samples*5, 5)]
        print(json.dumps(dict(pin=PIN, mode=mode, paired_cycle_median_s=statistics.median(samples),
                              paired_cycle_total_s=sum(samples), rows=rows)))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--repo', type=Path, default=Path.home() / 'jcode')
    p.add_argument('--cargo', type=Path, required=True)
    p.add_argument('--rustc', type=Path, required=True)
    p.add_argument('--reverse', action='store_true', help='measure precise first to check order effects')
    p.add_argument('--samples', type=int, default=7)
    a = p.parse_args()
    if a.samples < 1:
        p.error('samples must be positive')
    with tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR')) as tmp:
        experiment(a, Path(tmp))
