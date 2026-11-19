#!/usr/bin/env python3
"""Persistent scratch fixtures for one paired warm edit per repository."""
import os
from pathlib import Path
import re
import subprocess as sp
import tempfile
import time
import uuid

import readme_uv as uv

root = uv.scratch_root() / 'warm-three'
rows = [
    ('jcode', Path.home() / 'jcode', '557173275060be087a3763e5f448f124371b54c6', 'crates/jcode-tui/src/tui/jarvis.rs', '        (enabled, "--jarvis"),', '        (std::hint::black_box(enabled), "--jarvis"),', 'test -p jcode-tui --lib jarvis'),
    ('ripgrep', root / 'ripgrep', '4649aa9700619f94cf9c66876e9549d83420e16c', 'crates/ignore/src/gitignore.rs', '    pub fn is_empty(&self) -> bool {', '    pub fn is_empty(&self) -> bool {\n        std::hint::black_box(());', 'test -p ignore --lib --locked'),
    ('uv', uv.DEFAULT_SOURCE, uv.PIN, uv.EDIT_FILE, uv.EDIT_ANCHOR, uv.EDIT_ANCHOR + '        std::hint::black_box(());\n', 'check -p uv-resolver --locked'),
]


def run():
    root.mkdir(parents=True, exist_ok=True)
    out = Path(tempfile.mkdtemp(dir=root, prefix='results-'))
    print(out, flush=True)
    for name, source, pin, file, anchor, replacement, args in rows:
        try:
            if name == 'ripgrep' and not source.exists():
                sp.run(['git', 'clone', '--depth=1', '-b', '14.1.1', 'https://github.com/BurntSushi/ripgrep', source], check=True)
            checkout = root / (name + '-' + pin[:12])
            if not checkout.exists():
                with tempfile.NamedTemporaryFile(dir=root) as archive:
                    sp.run(['git', '-C', source, 'archive', pin], stdout=archive, check=True)
                    uv.extract_checkout(archive.name, checkout)
            p = checkout / file
            original = sp.check_output(['git', 'show', ':' + file], cwd=checkout)
            if p.read_bytes() != original or original.count(anchor.encode()) != 1:
                raise ValueError('unexpected fixture contents')
            counts = []
            for edited in (False, True):
                if edited:
                    p.write_bytes(original.replace(anchor.encode(), replacement.encode()))
                try:
                    for stack in ('stock', 'remote'):
                        rid = 'warm-three-' + uuid.uuid4().hex
                        cmd = args.split()
                        cmd = ['/usr/bin/cargo', *cmd] if stack == 'stock' else ['justrust', cmd[0], '--remote', *cmd[1:]]
                        env = uv.sample_environment(stack, checkout / 'target', rid)
                        log = out / f'{name}-{edited}-{stack}.log'
                        t = time.monotonic()
                        with log.open('w') as f:
                            rc = sp.run(cmd, cwd=checkout, env=env, stdout=f, stderr=sp.STDOUT).returncode
                        elapsed = time.monotonic() - t
                        print(name, 'measured' if edited else 'warmup', stack, round(elapsed, 2), 'exit', rc, rid if stack == 'remote' else '', flush=True)
                        if rc:
                            raise RuntimeError(log)
                        if stack == 'remote':
                            summary, _ = uv.read_evidence(Path(os.environ.get('JUSTRUST_HOME', Path.home()/'.justrust')), rid)
                            uv.validate_summary(stack, summary, rid)
                        if args.startswith('test'):
                            matches = re.findall(r'test result: ok\. (\d+) passed', log.read_text())
                            counts.append(summary['tests']['passed'] if stack == 'remote' else sum(map(int, matches)))
                finally:
                    if edited:
                        p.write_bytes(original)
            if counts and (not min(counts) or len(set(counts)) != 1):
                raise ValueError(f'unverified test counts: {counts}')
            print(name, 'pair complete', counts, flush=True)
        except Exception as error:
            print(name, 'incomplete:', repr(error), flush=True)


if __name__ == '__main__':
    run()
