#!/usr/bin/env python3
"""Portable, stdlib-only replay of frozen receipts. Does not run any native tool."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
ENGINE = '005c6198fddf1a7a77b6fda6d0be27601a0a48a7f2279deac460ebf5d4898edd'
ORIGINAL = '1197df17843a18b1cf5af78b22dc805d15bfe45056eefe1a04113599e681e3e0'
PREREQUISITE = 'ff9057b0ba5a7ce56ecfe822a6e1e6d39b28de74a74a435a1bb64862a9af4316'
# Fixed independent identity of retained bytes. Re-signing mutable inventories cannot
# silently replace exact historical nonces, diagnostics, source, tool or TC identities.
EVIDENCE_SHA = 'c7452457085be8dfbfc5b1418bb3da828a709dd2e05e6c4f92e682c166df3bda'
SAFE_ENV = frozenset('BLAZE_BACKEND BLAZE_CACHE BLAZE_HOME BLAZE_OPT CARGO_HOME CARGO_TARGET_DIR CARGO_PROFILE_DEV_DEBUG CARGO_PROFILE_DEV_INCREMENTAL CARGO_PROFILE_DEV_OPT_LEVEL CARGO_PROFILE_TEST_DEBUG CARGO_PROFILE_TEST_INCREMENTAL CARGO_PROFILE_TEST_OPT_LEVEL HOME RA_JIT_CACHE RA_JIT_HOME RUSTUP_HOME XDG_CACHE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_RUNTIME_DIR RA_JIT_DEMAND_STD RA_JIT_STATS RA_JIT_CACHE_MISS_STATS RA_JIT_WHOLE_RELOAD JR_NATIVE_CHOICE NATIVE_NONCE RUSTFLAGS RUSTDOCFLAGS RUSTUP_TOOLCHAIN'.split())
EXPECTED = {
    'native-link-reload-v56-original': ([True, False, True], [11, 11, 11], ORIGINAL),
    'native-link-reload-v56-candidate': ([True, True, False], [11, 22, 22], '2fb1170b4a550f541b3ebd1e2e1172ec262addb3c0179555ab31de1e4180a89f'),
    'native-link-reload-v58-candidate': ([True]*3, [11, 22, 11], ENGINE),
    'native-file-reload-v58-original': ([True, False, True], [11, 11, 11], ORIGINAL),
    'native-file-reload-v58-prerequisite': ([True, True, False], [11, 33, 33], PREREQUISITE),
    'native-file-reload-v58-candidate': ([True]*3, [11, 33, 11], ENGINE),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def read(root, name):
    p = root/name
    require(not p.is_symlink() and p.resolve().is_relative_to(root.resolve()), 'external path: '+name)
    return p.read_bytes()


def load(root, name):
    return json.loads(read(root, name))


def clean_env(value):
    if isinstance(value, dict):
        for key, child in value.items():
            if key.lower() in ('env', 'environment', 'environ', 'ambient_env') and isinstance(child, dict):
                require(set(child) <= SAFE_ENV, 'unsafe environment entries')
            clean_env(child)
    elif isinstance(value, list):
        for child in value:
            clean_env(child)


def replay_patch(root, version):
    base = f'frozen/v{version}'
    lines = read(root, base+'/candidate.patch').decode().splitlines(keepends=True)
    i = 0
    touched = []
    while i < len(lines):
        if not lines[i].startswith('--- '):
            i += 1
            continue
        old = lines[i][4:].strip().split('\t')[0]
        new = lines[i+1][4:].strip().split('\t')[0]
        require(new.startswith('b/'), 'patch new path')
        name = new[2:]
        original = [] if old == '/dev/null' else read(root, base+'/originals/'+old[2:]).decode().splitlines(keepends=True)
        out, cursor = [], 0
        i += 2
        while i < len(lines) and lines[i].startswith('@@ '):
            match = re.match(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@', lines[i])
            require(match is not None, 'invalid patch hunk')
            start, old_count, new_start, new_count = map(int, (match[1], match[2] or 1, match[3], match[4] or 1))
            offset = start-1 if start else 0
            require(offset >= cursor, 'overlapping patch hunks')
            out.extend(original[cursor:offset])
            cursor = offset
            require(len(out) == (new_start-1 if new_start else 0), 'patch output offset')
            consumed = added = 0
            i += 1
            while i < len(lines) and lines[i][:1] in (' ', '+', '-') and not lines[i].startswith('--- '):
                line = lines[i]
                if line[0] in ' -':
                    require(cursor < len(original) and original[cursor] == line[1:], 'patch original context')
                    cursor += 1
                    consumed += 1
                if line[0] in ' +':
                    out.append(line[1:])
                    added += 1
                i += 1
            require((consumed, added) == (old_count, new_count), 'patch hunk counts')
        out.extend(original[cursor:])
        require(''.join(out).encode() == read(root, base+'/candidates/'+name), 'patch replay mismatch '+name)
        touched.append(name)
    require(touched, 'empty patch')
    # Also prove candidates not represented by the patch are unchanged.
    for p in (root/base/'candidates').rglob('*'):
        if p.is_file():
            name = str(p.relative_to(root/base/'candidates'))
            if name not in touched:
                require(read(root, base+'/originals/'+name) == p.read_bytes(), 'unpatched candidate changed')
    for line in read(root, base+'/SHA256SUMS').decode().splitlines():
        digest, name = line.split(None, 1)
        require(sha(read(root, base+'/'+name.strip())) == digest, 'frozen inventory '+name)
    return len(touched)


def raw_row(root, name, row):
    require(row['error'] is None and row['raw_error'] is None and not row['timed_out'], 'runtime transport error')
    texts = {}
    for stream in ('stdout', 'stderr'):
        receipt = row['raw'][stream]
        filename = Path(receipt['path']).name
        data = read(root, f'runtime/{name}/{filename}')
        require(sha(data) == receipt['sha256'] and len(data) == receipt['bytes'], 'raw binding '+name+'/'+filename)
        texts[stream] = data.decode()
    status = row['status_receipt']
    require(status['status'] == 0 and status['error'] is None and not status['timed_out'], 'status receipt failure')
    prefix = Path(row['raw']['stdout']['path']).name.removesuffix('.stdout')
    text = read(root, f'runtime/{name}/{prefix}-status.stdout').decode()
    require('daemon: pid ' in text and 'worker: pid ' in text and 'store:' in text, 'missing native status identities')
    require(row['command'][-1] in ('run', 'summary::tests') or 'test' in row['command'], 'unexpected native command')
    return texts['stdout'], texts['stderr']


def identities(state):
    require(state['unchanged'] is True, 'source/tool identities changed')
    if 'tool_before' in state:
        require(state['tool_before'] == state['tool_after'], 'tool identity changed')
        require(state['toolchain_before'] == state['toolchain_after'], 'TC identity changed')
    else:
        require(state['tools'] == state['tools_after'] and state['toolchain'] == state['toolchain_after'], 'manifest identities changed')


def validate(root=HERE, seal=True, anchored=True):
    root = root.resolve()
    if seal:
        inventory = load(root, 'SHA256.json')
        actual = {str(p.relative_to(root)) for p in root.rglob('*') if p.is_file() and '__pycache__' not in p.parts and p.name != 'SHA256.json'}
        require(actual == set(inventory), 'bundle inventory set')
        for name, digest in inventory.items():
            require(sha(read(root, name)) == digest, 'bundle hash '+name)
    privacy = load(root, 'privacy.json')
    require(set(privacy['allowlist']) == SAFE_ENV, 'allowlist drift')
    retained = {}
    for p in privacy['files']:
        data = read(root, p['published'])
        require(sha(data) == p['published_sha256'] and len(data) == p['published_bytes'], 'privacy derivative binding')
        require(p['original_unchanged_verified'] is True, 'private originals verification')
        for env in p['environment']:
            require(env['omitted_entries'] >= 0 and env['original_entries'] == env['published_entries']+env['omitted_entries'], 'privacy omission arithmetic')
        if p['published'].endswith('.json'):
            clean_env(json.loads(data))
        elif p['published'].endswith('.jsonl'):
            for line in data.splitlines():
                clean_env(json.loads(line))
        retained[p['published']] = sha(data)
    patches = sum(replay_patch(root, v) for v in ('55', '56', '57', '58', '58-final-doc'))
    rows = 0
    for name, (accepted, observed, tool) in EXPECTED.items():
        state = load(root, 'runtime/'+name+'/state.json')
        identities(state)
        require(state['tool_before'] == tool, 'native engine identity')
        require([r['accepted'] for r in state['rows']] == accepted, 'failure history acceptance')
        require(state['status'] == ('passed' if all(accepted) else 'failed'), 'failure history status')
        require(len(state['rows']) == 3, 'native row count')
        for i, row in enumerate(state['rows']):
            out, err = raw_row(root, name, row)
            expected = [11, 33 if 'file-' in name else 22, 11][i]
            require(row['expected'] == expected, 'native expected value')
            require(row['status'] == 0, 'native exit')
            require(out == f'NATIVE_{row["nonce"]}_{observed[i]}\n', 'native nonce/value semantics')
            require(row['accepted'] == (observed[i] == expected), 'native acceptance semantics')
            if 'source_at_call' in row:
                want = sha(f'unsigned int native_value(void){{return {expected};}}\n'.encode())
                require(row['source_at_call']['a.c'] == want, 'C source at call')
        for path, digest in state['source_after'].items():
            require(sha(read(root, 'runtime/'+name+'/source/'+path)) == digest, 'source byte identity')
        require(all(state['source_after'].get(k) == v for k, v in state['source_before'].items()), 'restored source mismatch')
        rows += 3
    name = 'manifest-cfg-reload-v58'
    state = load(root, 'runtime/'+name+'/state.json')
    identities(state)
    require(state['status'] == 'passed' and len(state['rows']) == 9, 'manifest nine states')
    require(state['tools'] == dict(original=ORIGINAL, off=ENGINE, on=ENGINE), 'manifest tools')
    for i, row in enumerate(state['rows']):
        require((row['mode'], row['phase']) == (('original', 'off', 'on')[i//3], ('valid', 'manifest-error', 'recovery')[i%3]), 'manifest phase order')
        out, err = raw_row(root, name, row)
        require(row['accepted'] and row['rust_before'] == row['rust_after'], 'manifest source acceptance')
        rust = read(root, f'runtime/{name}/{row["mode"]}/src/main.rs')
        require(sha(rust) == row['rust_before'], 'manifest source bytes')
        if row['phase'] == 'manifest-error':
            require(row['status'] == 101 and 'E0308' in err and row['literal'] in err and 'GEN_OK_' not in out+err, 'manifest diagnostic semantics')
        else:
            require(row['status'] == 0 and out == 'GEN_OK_'+row['nonce']+'\n', 'manifest runtime nonce')
    rows += 9
    name = 'native-reload-v58-seven-state'
    state = load(root, 'runtime/'+name+'/state.json')
    identities(state)
    require(state['tool_before'] == ENGINE and state['startup_controls']['RA_JIT_WHOLE_RELOAD'] == '0', 'default correctness only')
    phases = ['cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery']
    require(state['status'] == 'passed' and [r['phase'] for r in state['rows']] == phases, 'seven-state phase order')
    for row in state['rows']:
        out, err = raw_row(root, name, row)
        require(row['accepted'] and row['input_unchanged'] and row['failures'] == [], 'seven-state acceptance')
        if row['phase'] == 'type-error':
            require(row['status'] == 101 and 'E0308' in err and 'JR_MATRIX_TYPE_'+row['nonce'] in err and not out and row['parsed'] is None, 'type diagnostic semantics')
        elif row['phase'] == 'assert-failure':
            require(row['status'] == 101 and 'JR_MATRIX_RUNTIME_'+row['nonce'] in out and '3 passed; 1 failed;' in out and row['parsed']['failed'] == 1, 'assert failure semantics')
        else:
            require(row['status'] == 0 and '4 passed; 0 failed;' in out and row['parsed']['passed'] == 4 and row['parsed']['failed'] == 0, 'libtest semantics')
    rows += 7
    gates = []
    for version in (56, 57, 58):
        for kind in ('build', 'unit'):
            base = f'gates/candidate-{kind}-v{version}'
            if kind == 'build':
                for gate in ('check', 'compile'):
                    require(read(root, base+'/'+gate+'.exit') == b'0\n', 'build exit')
                sources = read(root, base+'/all-source-before.sha256').decode().splitlines()
                require(len(sources) == 3757, 'source3757 identity count')
                if version == 58:
                    for file in ('main.rs', 'inputs.rs'):
                        digest = sha(read(root, 'frozen/v58/candidates/toolchain/resident/crates/ra-jit/src/'+file))
                        require(digest+'  crates/ra-jit/src/'+file in sources, 'candidate/build source identity')
                    require(read(root, base+'/engine-identity.sha256').decode().split()[0] == ENGINE, 'launcher hash')
                    after = read(root, 'gates/candidate-unit-v58/all-source-after.log').decode().splitlines()
                    require(after == [line.split(None, 1)[1]+': OK' for line in sources], 'full source restoration')
                    require(read(root, 'gates/candidate-unit-v58/lock-after.log') == b'Cargo.lock: OK\n', 'root lock restoration')
            else:
                require(read(root, base+'/codegen-native.exit') == b'0\n', 'codegen exit')
                require(read(root, base+'/full-native.exit') == (b'0\n' if version == 58 else b'101\n'), 'historical full test exit')
            for path in sorted((root/f'journals/candidate-{kind}-v{version}').glob('*/summary.json')):
                relative = str(path.relative_to(root))
                summary = load(root, relative)
                meta = load(root, str(path.parent.relative_to(root))+'/meta.json')
                require(meta['id'] == summary['id'] == path.parent.name, 'journal identity')
                output = ''.join(json.loads(line)['l']+'\n' for line in read(root, str(path.parent.relative_to(root))+'/output.jsonl').decode().splitlines())
                tests = summary['tests']
                if kind == 'unit':
                    codegen = any('hir-ty-codegen' == arg for arg in meta['args'])
                    expected = (53, 0, 0) if codegen else ((364, 0, 0) if version == 58 else (361, 1, 101))
                    require((tests['passed'], tests['failed'], summary['exit']) == expected, 'journal gate counts')
                    require(f'{expected[0]} passed; {expected[1]} failed;' in output, 'full journal libtest result')
                else:
                    require(summary['exit'] == 0, 'journal build result')
                gates.append(dict(id=summary['id'], version=version, kind=kind, passed=tests['passed'], failed=tests['failed'], exit=summary['exit']))
    for choice, immediate in (('a', '$0xb,%eax'), ('b', '$0x16,%eax')):
        require(immediate in read(root, f'runtime/native-link-reload-v56-original/{choice}-archive.objdump.txt').decode(), 'actual A/B objdump')
    artifacts = load(root, 'native-artifact-identities.json')
    for choice in ('a', 'b'):
        require(any('/nl560/' in f['path'] and '/out/'+choice+'/libnativechoice.a' in f['path'] and f['unchanged_verified'] for f in artifacts['files']), 'actual original archive hash')
    cleanup = load(root, 'cleanup/completed-native-reload-v58-stop/receipt.json')
    require(cleanup['stores_deleted'] is False and len(cleanup['rows']) == 1 and cleanup['rows'][0]['status'] == 0, 'cleanup preserves stores')
    regression = read(root, 'provenance/v58-bench-regression.log').decode()
    require(re.search(r'Ran 733 tests in .*\n\nOK\s*$', regression), '733 regression result')
    retained['native-artifact-identities.json'] = sha(read(root, 'native-artifact-identities.json'))
    # Privacy receipt itself binds original private SHA and omission counts too.
    retained['privacy.json'] = sha(read(root, 'privacy.json'))
    evidence_sha = sha(canonical(retained))
    if anchored:
        require(evidence_sha == EVIDENCE_SHA, 'exact historical semantic identity')
    return dict(status='passed', runtime_rows=rows, patches_replayed=patches, gates=gates, evidence_sha256=evidence_sha, whole_generation_reuse='UNIMPLEMENTED', speed_claim=False, scope='Default native reload correctness only. No native jobs executed.')


def audit(root):
    root = root.resolve()
    def hook(event, args):
        if event == 'open':
            path, mode, flags = args
            require(not isinstance(path, int), 'unexpected descriptor open')
            require(Path(path).resolve().is_relative_to(root), 'audit external read')
            require(not (isinstance(mode, str) and any(c in mode for c in 'wax+')), 'audit write')
            require(not flags & (1 | 2 | 64 | 512 | 1024), 'audit write flags')
        if event in ('os.listdir', 'os.scandir'):
            require(Path(args[0]).resolve().is_relative_to(root), 'audit external directory')
        require(event not in ('os.remove', 'os.rename', 'os.rmdir', 'os.mkdir', 'os.link', 'os.symlink', 'os.truncate'), 'audit mutation denied')
        require(not event.startswith(('subprocess.', 'socket.', 'os.exec', 'os.spawn', 'os.fork')) and event != 'os.system', 'audit native/network denied')
    sys.addaudithook(hook)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--audit', action='store_true', help='Reject reads outside bundle, writes, native execution and network')
    args = parser.parse_args()
    if args.audit:
        audit(HERE)
    try:
        result = validate()
        result['audit_enabled'] = args.audit
        print(json.dumps(result, indent=2, sort_keys=True))
    except (ValueError, KeyError, OSError) as error:
        print('INVALID: '+str(error), file=sys.stderr)
        sys.exit(1)
