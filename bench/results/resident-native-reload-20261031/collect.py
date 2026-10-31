#!/usr/bin/env python3
"""Retain existing receipts only. Never execute native jobs. Private inputs are read-only."""
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
REPO = Path('/home/jeremy/justrust')
# Deliberately explicit, not prefix-based. Never publish arbitrary ambient env.
SAFE_ENV = frozenset('BLAZE_BACKEND BLAZE_CACHE BLAZE_HOME BLAZE_OPT CARGO_HOME CARGO_TARGET_DIR CARGO_PROFILE_DEV_DEBUG CARGO_PROFILE_DEV_INCREMENTAL CARGO_PROFILE_DEV_OPT_LEVEL CARGO_PROFILE_TEST_DEBUG CARGO_PROFILE_TEST_INCREMENTAL CARGO_PROFILE_TEST_OPT_LEVEL HOME RA_JIT_CACHE RA_JIT_HOME RUSTUP_HOME XDG_CACHE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_RUNTIME_DIR RA_JIT_DEMAND_STD RA_JIT_STATS RA_JIT_CACHE_MISS_STATS RA_JIT_WHOLE_RELOAD JR_NATIVE_CHOICE NATIVE_NONCE RUSTFLAGS RUSTDOCFLAGS RUSTUP_TOOLCHAIN'.split())
SESSIONS = ['native-link-reload-v56-original', 'native-link-reload-v56-candidate', 'native-link-reload-v58-candidate', 'native-file-reload-v58-original', 'native-file-reload-v58-prerequisite', 'native-file-reload-v58-candidate', 'manifest-cfg-reload-v58', 'native-reload-v58-seven-state']


def sha(b):
    return hashlib.sha256(b).hexdigest()


def emit(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')


def sanitize(value, counts, pointer=''):
    if isinstance(value, dict):
        result = {}
        for key, child in value.items():
            if key.lower() in ('env', 'environment', 'environ', 'ambient_env') and isinstance(child, dict):
                safe = {k: v for k, v in child.items() if k in SAFE_ENV}
                counts.append(dict(pointer=pointer+'/'+key, original_entries=len(child), published_entries=len(safe), omitted_entries=len(child)-len(safe)))
                result[key] = safe
            else:
                result[key] = sanitize(child, counts, pointer+'/'+key)
        return result
    if isinstance(value, list):
        return [sanitize(v, counts, pointer+'/'+str(i)) for i, v in enumerate(value)]
    return value


def main():
    provenance = []
    def retain(src, relative):
        assert src.is_file() and not src.is_symlink(), src
        original = src.read_bytes()
        assert b'\x00' not in original and not original.startswith(b'\x7fELF'), src
        counts = []
        if src.suffix == '.json':
            data = (json.dumps(sanitize(json.loads(original), counts), indent=2, sort_keys=True)+'\n').encode()
        elif src.suffix == '.jsonl':
            data = ''.join(json.dumps(sanitize(json.loads(line), counts), sort_keys=True)+'\n' for line in original.decode().splitlines() if line).encode()
        else:
            data = original
        dest = HERE / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(data)
        assert sha(src.read_bytes()) == sha(original), 'private original changed'
        provenance.append(dict(source=str(src), published=relative, original_sha256=sha(original), published_sha256=sha(data), original_bytes=len(original), published_bytes=len(data), environment=counts, original_unchanged_verified=True))
    def tree(source, target):
        for p in sorted(source.rglob('*')):
            if p.is_file():
                retain(p, str(Path(target)/p.relative_to(source)))
    for v in ('55', '56', '57', '58', '58-final-doc'):
        tree(ROOT/f'whole-generation-reuse-v{v}', f'frozen/v{v}')
    for n in SESSIONS:
        tree(ROOT/n, f'runtime/{n}')
    for v in (56, 57, 58):
        retain(ROOT/f'build-test-candidate-v{v}.sh', f'provenance/build-test-candidate-v{v}.sh')
        for kind in ('build', 'unit'):
            n = f'candidate-{kind}-v{v}'
            for p in sorted((ROOT/n).iterdir()):
                if p.is_file():
                    retain(p, f'gates/{n}/{p.name}')
            for run in sorted((ROOT/n/'justrust-home/runs').glob('*')):
                for name in ('meta.json', 'output.jsonl', 'summary.json'):
                    p = run/name
                    if p.is_file():
                        retain(p, f'journals/{n}/{run.name}/{name}')
    for name in ('probe-native-link-reload-v56.py', 'probe-native-file-reload-v58.py', 'probe-manifest-cfg-reload-v55.py', 'probe-whole-generation-reuse-v55.py', 'cleanup-completed-native-reload-v58.py', 'v58-bench-regression.log'):
        retain(ROOT/name, 'provenance/'+name)
    tree(ROOT/'completed-native-reload-v58-stop', 'cleanup/completed-native-reload-v58-stop')
    for name in ('resident_engine_compare.py', 'resident_state_matrix.py', 'resident_workload_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py'):
        retain(REPO/'bench'/name, 'provenance/'+name)
    for name in ('LICENSE-APACHE', 'LICENSE-MIT'):
        retain(REPO/'toolchain/resident'/name, 'licenses/'+name)
    retain(REPO/'LICENSE', 'licenses/justrust-LICENSE')
    # Existing archive/object bytes are hashed in place, never copied or executed.
    archives = []
    for private in ('nl560', 'nl561', 'nl582', 'nf580', 'nf581', 'nf582'):
        base = Path('/home/jeremy')/private/'engine/target'
        for p in sorted(base.rglob('*')):
            if p.is_file() and p.name in ('libnativechoice.a', 'native.o'):
                data = p.read_bytes()
                archives.append(dict(path=str(p), bytes=len(data), sha256=sha(data), unchanged_verified=sha(p.read_bytes()) == sha(data)))
    emit(HERE/'native-artifact-identities.json', dict(scope='Post-run read-only hashes of preserved stores, not binaries, not archive re-execution. Original A/B disassembly captured separately at runtime/native-link-reload-v56-original.', files=archives))
    emit(HERE/'privacy.json', dict(policy='Explicit safe environment allowlist only. Omitted names and values are not published. Private originals never modified. SHA identities bind private originals, not a public claim to reconstruct omitted values.', allowlist=sorted(SAFE_ENV), files=provenance))
    print(json.dumps(dict(files=len(provenance), environment_maps=sum(len(p['environment']) for p in provenance), omissions=sum(e['omitted_entries'] for p in provenance for e in p['environment']))))


if __name__ == '__main__':
    main()
