#!/usr/bin/env python3
"""One-time allowlisted receipt collector, not a workload runner."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import io
import tarfile

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
RAW = Path('/home/jeremy/re01')
ENGINE = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
ENTRY = Path('/home/jeremy/.jcode/scratch/re-entry-20261030')

def digest(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()

def copy(src, dst):
    assert src.is_file() and not src.is_symlink(), src
    data = src.read_bytes()
    assert not data.startswith(b'\x7fELF'), src
    dst = ROOT / dst
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dst)

for name in ('metadata.json', 'records.json', 'summary.json', 'source.original'):
    copy(RAW / name, 'runtime/' + name)
for stack in ('reference', 'engine'):
    for phase, count in [('version', 1), ('prepare', 1), ('type-error', 1), ('type-recovery', 1), ('assert-failure', 1), ('assert-recovery', 1), ('edit', 5)]:
        for rep in range(count):
            for suffix in ('stdout', 'stderr'):
                name = f'{stack}-{phase}-{rep}.{suffix}'
                copy(RAW / name, 'runtime/' + name)
meta = json.loads((RAW / 'metadata.json').read_text())
assert len(meta['source_before']) == 63
for name, sha in meta['source_before'].items():
    src = Path('/home/jeremy/.jcode/scratch/ba30/source') / name
    assert digest(src) == sha, name
    copy(src, 'source/' + name)
copy(ENTRY / 'acceptance.json', 'entry/acceptance.json')
for row in json.loads((ENTRY / 'acceptance.json').read_text())['rows']:
    for suffix in ('stdout', 'stderr'):
        name = row['case'] + '.' + suffix
        copy(ENTRY / name, 'entry/' + name)
for directory, names in {
    'baseline-build': ['rustc-version.txt', 'tool-identities.sha256', 'build.log', 'exit', 'engine-version.txt'],
    'candidate-build-v2': ['source-before.sha256', 'build.log', 'compile-exit', 'source-after.log', 'engine-version.txt', 'engine-identity.sha256', 'exit'],
}.items():
    for name in names:
        copy(ENGINE / directory / name, 'build/' + directory + '/' + name)
for name in ('build-baseline.sh', 'build-candidate-v2.sh', 'run-runtime-v2.sh', 'runtime-v2.log', 'runtime-v2.exit'):
    copy(ENGINE / name, 'build/' + name)
for name in ('resident_engine_compare.py', 'resident_entry_accept.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py'):
    copy(REPO / 'bench' / name, 'harness/' + name)
for name in ('LICENSE-APACHE', 'LICENSE-MIT'):
    copy(REPO / 'toolchain/resident' / name, 'provenance/' + name)
copy(REPO / 'toolchain/sources.json', 'provenance/toolchain-sources.json')
copy(Path(meta['toolchain_before']['path']) / 'lib/rustlib/multirust-channel-manifest.toml', 'provenance/channel-manifest.toml')
# Exact candidate commit predates unrelated concurrent working-tree changes.
candidate_commit = '20f9102027d553bc72c003927964b4ca72bc6574'
archive = subprocess.check_output(['git', 'archive', candidate_commit + ':toolchain/resident'], cwd=REPO)
with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
    candidate = {m.name: tar.extractfile(m).read() for m in tar.getmembers() if m.isfile()}
(ROOT / 'provenance/candidate-commit.txt').write_text(candidate_commit + '\n')
# Full compiler source identities only, never the compiler source tree or artifacts.
for label, tree in [('baseline', ENGINE / 'source')]:
    files = {}
    for path in sorted(tree.rglob('*')):
        rel = path.relative_to(tree)
        if any(p in ('.git', 'target', 'node_modules', '.wrangler') for p in rel.parts):
            continue
        if path.is_file() and not path.is_symlink():
            files[str(rel)] = digest(path)
    (ROOT / 'provenance' / (label + '-source-manifest.json')).write_text(json.dumps(files, indent=2) + '\n')
files = {name: hashlib.sha256(data).hexdigest() for name, data in sorted(candidate.items())}
(ROOT / 'provenance/candidate-source-manifest.json').write_text(json.dumps(files, indent=2) + '\n')
# Record the exact source-key input identities, not a claim to reconstruct source bytes.
identity = []
for name, data in sorted(candidate.items()):
    parts = Path(name).parts
    compiled = parts[0] in ('crates', 'vendor') and not any(p.startswith('.') or p in ('target', 'node_modules') for p in parts[1:-1])
    if compiled or name in ('Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'harness/toolchain'):
        identity.append({'path': name, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
(ROOT / 'provenance/candidate-build-inputs.json').write_text(json.dumps(identity, indent=2) + '\n')
for label, cwd, args in [
    ('workload-tree', REPO, ['ls-tree', '-r', 'f97fcc2']),
    ('baseline-head', ENGINE / 'source', ['rev-parse', 'HEAD']),
    ('baseline-status', ENGINE / 'source', ['status', '--porcelain']),
]:
    (ROOT / 'provenance' / (label + '.txt')).write_bytes(subprocess.check_output(['git', *args], cwd=cwd))
print('Copied allowlisted evidence and 63 verified source files')
