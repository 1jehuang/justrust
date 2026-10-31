#!/usr/bin/env python3
"""Portable offline replay of frozen native-input metadata evidence. No native execution."""
import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import re
import statistics
import sys

HERE = Path(__file__).resolve().parent
COMMIT = 'a59351f652c9b266b7f29734f2218cbb19a9aaad'
TC_SHA = '8bdbccdd1ca3fd7f2935cfb5667047eb4c46824928823e96311d8eb5ec0a6371'
ENGINE = '767491d3f56ba67ae4c87d7275442cb9b6fae62cfa06f4538c5e3c565dbda301'
REFERENCE = 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263'
OLD_ENGINE = '89c9445128b4ece98e9b99b0d399ac102bc68185d52ff96098d944f6f4627a20'
SOURCES = {'codegen-mod.rs': 'ee9fa1b2298f22fc2c1e062b3692383a7bb3e2df3f5fbeee9070270b3368c98c',
           'main.rs': '3ef8143a8f6294335168f9a987d3266ce347f06209e1e3c3f209b72d9c6447fd',
           'doc.md': '5003298263d7ac917b7103547eb629e2435d0842a1f6787b0dcd41fd421c5f2a'}
HELPERS = {'blaze_edit_compare': '9c451fdb939d3a8c024ddf7e2b8c2fd3a8badc7c164a429415a7c636eb8c41a8',
           'blaze_runtime_compare': 'a7e2af863e2b1ea2878ae5c66a0f01aab1ad7fb9efe2e1027254a1269eefb568',
           'resident_engine_compare': '7da5b024344f662743b81043f4c200a23b3a49f61356e933b48257b2860c5a80'}
JOURNALS = [('candidate-build-v35', 'check', '20261030-182508083-573757', 101, 0),
            ('candidate-build-v36', 'check', '20261030-182658166-583351', 0, 0),
            ('candidate-build-v36', 'compile', '20261030-182713472-585395', 0, 0),
            ('candidate-unit-v36', 'codegen-native', '20261030-182838650-595329', 0, 48),
            ('candidate-unit-v36', 'full-native', '20261030-182858112-597752', 0, 360)]
SCRIPTS = {'build-test-candidate-v35.sh': '61e728fea26788adb03a22056c1480ada455156834fc68bff30636f866a5d507',
           'build-test-candidate-v36.sh': '28d49a5363e10cca3c8880f487e4dc09ab1376f8effe43e2d24cd828fa390f62',
           'probe-demand-inventory-v36.py': '24bd5e07427f4bfc6e6b4eea3fa4146fe9a3cc2a01bff2d904b2071bc8301a98'}
ENV_ALLOW = set('HOME CARGO_HOME CARGO_TARGET_DIR RUSTUP_HOME BLAZE_HOME BLAZE_CACHE BLAZE_BACKEND BLAZE_OPT BLAZE_REMOTE BLAZE_PROC_MACRO_SERVER RA_JIT_HOME RA_JIT_CACHE RA_JIT_REMOTE RA_JIT_PROC_MACRO_SERVER JR_ENGINE_HOME XDG_CACHE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_RUNTIME_DIR JCODE_SCRATCH_DIR JCODE_SESSION_ID RA_JIT_DEMAND_STD RA_JIT_DEMAND_INVENTORY'.split())
ENV_ALLOW |= {f'CARGO_PROFILE_{profile}_{key}' for profile in ('DEV', 'TEST') for key in ('DEBUG', 'INCREMENTAL', 'OPT_LEVEL')}
COUNTS = [6998, 0, 48841, 0, 0, 2, 26533]
PATTERN = re.compile(r'demand inventory: (\d+) compiled roots, (\d+) skipped roots, (\d+) declared instances, (\d+) found instances, (\d+) rejected instances, (\d+) kept statics, (\d+) data definitions \(full round\)')


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return sha(json.dumps(value, sort_keys=True, separators=(',', ':')).encode())


def load(path):
    def unique(items):
        value = {}
        for k, v in items:
            require(k not in value, 'duplicate JSON key')
            value[k] = v
        return value
    return json.loads(path.read_text(), object_pairs_hook=unique)


def safe(root, name):
    p = Path(name)
    require(not p.is_absolute() and p.parts and '..' not in p.parts, 'unsafe path')
    require(not any((root / Path(*p.parts[:n])).is_symlink() for n in range(1, len(p.parts) + 1)), 'symlink')
    return root / p


def files(root):
    result = {}
    for p in root.rglob('*'):
        require(not p.is_symlink(), 'symlink')
        if not p.is_file():
            require(p.is_dir(), 'special file')
            continue
        if '__pycache__' in p.parts:
            continue
        name = p.relative_to(root).as_posix()
        if name in ('SHA256SUMS', 'validation.json'):
            continue
        data = p.read_bytes()
        require(not data.startswith(b'\x7fELF'), 'binary excluded')
        require(not {'target', '.git', '.wrangler', 'justrust-home'} & set(p.relative_to(root).parts), 'private cache/build directory')
        result[name] = sha(data)
    return result


def seal(root):
    (root / 'SHA256SUMS').write_text(''.join(f'{digest}  {name}\n' for name, digest in sorted(files(root).items())))


def manifest(root):
    actual = {}
    for line in (root / 'SHA256SUMS').read_text().splitlines():
        require(re.fullmatch(r'[0-9a-f]{64}  .+', line), 'invalid checksum entry')
        digest, name = line.split('  ', 1)
        safe(root, name)
        require(name not in actual, 'duplicate checksum entry')
        actual[name] = digest
    require(actual == files(root), 'hash inventory mismatch')


def modules(root):
    for name, digest in HELPERS.items():
        p = root / 'provenance' / (name + '.py')
        require(sha(p.read_bytes()) == digest, 'frozen helper drift')
        spec = importlib.util.spec_from_file_location(name, p)
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    return sys.modules['resident_engine_compare']


def apply_unified(original, patch):
    """Replay one ordinary textual unified diff without Git or a checkout."""
    lines, out, cursor = original.splitlines(keepends=True), [], 0
    p = patch.splitlines(keepends=True)
    i = 0
    while i < len(p):
        m = re.match(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@', p[i])
        if not m:
            i += 1
            continue
        start = max(0, int(m[1]) - 1)
        require(start >= cursor, 'overlapping patch')
        out.extend(lines[cursor:start])
        cursor = start
        old_count = new_count = 0
        i += 1
        while i < len(p) and p[i][:1] in (' ', '+', '-') and not p[i].startswith(('--- ', '+++ ')):
            mark, text = p[i][0], p[i][1:]
            if mark in (' ', '-'):
                require(cursor < len(lines) and lines[cursor] == text, 'patch context mismatch')
                cursor += 1
                old_count += 1
            if mark in (' ', '+'):
                out.append(text)
                new_count += 1
            i += 1
        require(old_count == int(m[2] or 1) and new_count == int(m[4] or 1), 'patch hunk size')
    out.extend(lines[cursor:])
    return ''.join(out)


def source_audit(root):
    require((root / 'accepted/commit.txt').read_text().strip() == COMMIT, 'accepted commit')
    for name, digest in SOURCES.items():
        require(sha((root / 'accepted' / name).read_bytes()) == digest, 'accepted source/doc identity')
        if name != 'doc.md':
            require((root / 'accepted' / name).read_bytes() == (root / 'frozen/v36' / name).read_bytes(), 'v36 accepted byte binding')
    history = root / 'frozen/v34-history'
    for line in (history / 'SHA256SUMS').read_text().splitlines():
        digest, name = line.split('  ', 1)
        require(name.startswith('demand-library-artifact-v34/'), 'historical receipt path prefix')
        require(sha(safe(history, name.removeprefix('demand-library-artifact-v34/')).read_bytes()) == digest, 'v34 historical frozen manifest')
    v35 = root / 'frozen/v35'
    for line in (v35 / 'SHA256SUMS').read_text().splitlines():
        digest, name = line.split('  ', 1)
        require(sha(safe(v35, name).read_bytes()) == digest, 'v35 original frozen manifest')
    mapping = {'codegen-mod.rs': 'toolchain/resident/crates/hir-ty-codegen/mod.rs',
               'main.rs': 'toolchain/resident/crates/ra-jit/src/main.rs',
               'doc.md': 'docs/resident-demand-library-artifact.md'}
    for line in (v35 / 'root-freeze/SHA256SUMS').read_text().splitlines():
        digest, path = line.split('  ', 1)
        require(path in mapping.values(), 'root freeze bounded path')
        name = next(n for n, p in mapping.items() if p == path)
        require(sha((v35 / 'root-freeze' / name).read_bytes()) == digest, 'root freeze manifest')
    for name, path in mapping.items():
        require((v35 / 'root-freeze' / name).read_bytes() == (v35 / 'after' / path).read_bytes(), 'root-owned freeze differs from worker candidate')
    patch = (v35 / 'change.patch').read_text()
    parts = re.split(r'(?=^--- a/)', patch, flags=re.M)
    for part in filter(None, parts):
        path = part.splitlines()[0].removeprefix('--- a/')
        require(path in mapping.values(), 'out-of-scope worker patch')
        require(apply_unified((v35 / 'before' / path).read_text(), part) == (v35 / 'after' / path).read_text(), 'worker patch full-byte replay')
    require((v35 / 'before/toolchain/resident/crates/hir-ty-codegen/units.rs').read_bytes() == (v35 / 'after/toolchain/resident/crates/hir-ty-codegen/units.rs').read_bytes(), 'units changed')
    accepted_patch = (root / 'accepted/change.patch').read_text()
    require(accepted_patch.startswith('commit ' + COMMIT + '\n'), 'gitshow commit binding')
    sections = re.split(r'(?=^diff --git )', accepted_patch, flags=re.M)[1:]
    require(len(sections) == 3, 'accepted three-path scope')
    for section in sections:
        path = re.search(r'^\+\+\+ b/(.+)$', section, re.M).group(1)
        require(path in mapping.values(), 'out-of-scope accepted patch')
        name = next(n for n, p in mapping.items() if p == path)
        original = '' if '--- /dev/null\n' in section else (v35 / 'before' / path).read_text()
        require(apply_unified(original, section) == (root / 'accepted' / name).read_text(), 'gitshow full-byte replay')
    for name, digest in SCRIPTS.items():
        require(sha((root / 'provenance' / name).read_bytes()) == digest, 'frozen launcher/build script drift')
    old = (v35 / 'root-freeze/codegen-mod.rs').read_text()
    new = (root / 'accepted/codegen-mod.rs').read_text()
    a = old.index('    fn demand_inventory_data_capture_does_not_change_native_output_or_errors()')
    end = old.index('\n    #[test]', a)
    aa = new.index('    fn demand_inventory_data_capture_does_not_change_native_output_or_errors()')
    ee = new.index('\n    #[test]', aa)
    require(old[:a] == new[:aa] and old[end:] == new[ee:], 'repair exceeds fixture')
    fixture = new[aa:ee]
    for exact in ('target().unwrap().clone()', 'ModuleRelocTarget::User { namespace, index } => (*namespace, *index)',
                  '(1, local.as_u32())', '(0, function.as_u32())', 'for description in [&captured[1].1, &pointers]'):
        require(exact in fixture, 'exact relocation/Arc repair absent')
    require('value == Some(std::ffi::OsStr::new("1"))' in new, 'default-off exact opt-in')
    require(new.index('let inventory = cg.inventory(roots, &compiled, &skipped, denied);') < new.index('let (object, apart) = cg.finish()?;'), 'capture not pre-finalization')
    require('inventory: None,' in new and 'no full-round coverage claimed' in new, 'patch coverage unavailable')
    helper = new[new.index('fn define_inventory_data('):new.index('/// What a call to an intrinsic turned into.')]
    require(helper.index('module.define_data(id, data)') < helper.index('inventory.push((id, data.clone()))'), 'record only successful definitions')


def check_identity(value, source, toolchain, tool=None):
    for kind, expected in [('source', source), ('toolchain', toolchain)]:
        require(value[kind + '_before'] == value[kind + '_after'] == expected, kind + ' identity unchanged')
    require(canonical(toolchain) == TC_SHA and len(toolchain['files']) == 4020, 'frozen TC commitment')
    require(value['unchanged'] is True, 'unchanged gate')
    if tool:
        require(value['tool_before'] == value['tool_after'] == tool, 'native tool identity unchanged')


def gate_audit(root):
    source_maps = []
    for v in (35, 36):
        directory = root / f'gates/candidate-build-v{v}'
        mapping = {}
        for line in (directory / 'all-source-before.sha256').read_text().splitlines():
            digest, name = line.split('  ', 1)
            require(name not in mapping and len(digest) == 64, 'source receipt entry')
            mapping[name] = digest
        require(len(mapping) == 3757, 'all 3757 source identities')
        for name, path in [('codegen-mod.rs', 'crates/hir-ty-codegen/mod.rs'), ('main.rs', 'crates/ra-jit/src/main.rs')]:
            frozen = root / ('frozen/v35/root-freeze' if v == 35 else 'frozen/v36') / name
            require(mapping[path] == sha(frozen.read_bytes()), 'compiled frozen source identity')
        source_maps.append(mapping)
    changed = {k for k in source_maps[0] if source_maps[0][k] != source_maps[1][k]}
    require(changed == {'crates/hir-ty-codegen/mod.rs'}, 'only fixture changed between native checks')
    after = (root / 'gates/candidate-unit-v36/all-source-after.log').read_text().splitlines()
    require(after == [name + ': OK' for name in source_maps[1]], 'all-source before/after gate')
    require((root / 'gates/candidate-unit-v36/lock-after.log').read_text() == 'Cargo.lock: OK\n', 'lock after')
    require((root / 'gates/candidate-build-v35/lock-before.sha256').read_bytes() == (root / 'gates/candidate-build-v36/lock-before.sha256').read_bytes(), 'lock before identity')
    for group, gate, run, status, count in JOURNALS:
        directory = root / 'journals' / run
        meta, summary = [load(directory / n) for n in ('meta.json', 'summary.json')]
        output = [json.loads(l) for l in (directory / 'output.jsonl').read_text().splitlines()]
        text = '\n'.join(r['l'] for r in output)
        require(meta['id'] == summary['id'] == run and meta['args'] == summary['args'], 'journal identity/args')
        expected_args = (['check', '--release', '--locked', '-p', 'ra-jit', '--tests', '-p', 'hir-ty-codegen', '-j', '6'] if gate == 'check' else
                         ['build', '--release', '--locked', '-p', 'ra-jit', '--bin', 'jr-engine', '-j', '6'] if gate == 'compile' else
                         ['test', '--release', '--locked', '-p', 'hir-ty-codegen', '-j', '6'] if count == 48 else
                         ['test', '--release', '--locked', '-p', 'ra-jit', '--bin', 'jr-engine', '-j', '6'])
        require(meta['args'] == expected_args, 'actual native gate command')
        require(summary['exit'] == status and summary['tests']['passed'] == count, 'journal status/count')
        require(int((root / 'gates' / group / (gate + '.exit')).read_text()) == status, 'exit receipt')
        require(summary['diagnostics']['compile_errors'] == (3 if status else 0), 'actual compiler errors')
        if status:
            require(text.count('error[E0308]') == 1 and text.count('error[E0369]') == 2, 'three fixture compiler errors retained')
        if count:
            require(re.search(r'test result: ok\. ' + str(count) + r' passed; 0 failed;', text), 'raw full native test total')
            tests = re.findall(r'^test (\S+) \.\.\. ok', text, re.M)
            require(len(set(tests)) == count, 'full passing names in raw journal')
            new_tests = (['tests::demand_inventory_requires_the_exact_separate_opt_in', 'tests::demand_inventory_data_capture_does_not_change_native_output_or_errors'] if count == 48 else
                         ['tests::demand_inventory_preserves_root_instance_static_and_refusal_identities'])
            require(set(new_tests) <= set(tests), 'new inventory fixtures actually executed')
        slim = load(root / 'slim' / (run + '.json'))
        require(all(slim[k] == summary[k] for k in ('id', 'args', 'subcommand', 'start', 'wall', 'exit', 'tests', 'route') if k in summary), 'slim export mismatch')
    require(ENGINE in (root / 'gates/candidate-build-v36/engine-identity.sha256').read_text(), 'built engine identity')
    version = (root / 'gates/candidate-build-v36/engine-version.txt').read_text().strip()
    require(version.endswith('build resident-v1-11eee3f9ddd82894df5699baa288843febacff7987f3ab821b193720335599e5)'), 'build identity')
    require('Ran 733 tests' in (root / 'outer/32891831af.output').read_text() and '[stderr] OK' in (root / 'outer/32891831af.output').read_text(), '733 Python root raw receipt')
    return version


def raw(directory, prefix):
    return '\n'.join((directory / (prefix + '.' + suffix)).read_text() for suffix in ('stdout', 'stderr'))


def valid_process(row):
    require(row['error'] is None and row['timed_out'] is False, 'process launch/timeout')
    elapsed = row['elapsed_seconds']
    require(isinstance(elapsed, (int, float)) and math.isfinite(elapsed) and elapsed > 0, 'elapsed time')
    require(row['noise']['contamination'] == 'unknown', 'noise must remain unknown')


def probe_audit(root, h, source):
    directory = root / 'probe'
    state = load(directory / 'state.json')
    check_identity(state, source, state['toolchain_before'], ENGINE)
    require(state['status'] == 'passed' and len(state['rows']) == 2, 'two-call native diagnostic')
    for row, mode in zip(state['rows'], ('off', 'on')):
        valid_process(row)
        require(row['mode'] == mode and row['accepted'] is True and row['status'] == 0, 'probe success')
        require(row['startup_controls'] == {'RA_JIT_DEMAND_STD': '1', 'RA_JIT_DEMAND_INVENTORY': str(int(mode == 'on'))}, 'explicit separate startup controls')
        require(Path(row['command'][0]).name == 'jr-engine' and '/candidate-v36-bin/' in row['command'][0] and row['command'][1:] == ['test', *h.FILTER], 'actual native cold command')
        text = raw(directory, mode)
        parsed = h.runtime.test_results(text)
        require(h.valid_tests(parsed) and parsed['passed'] == 4, 'four actual tests per cold call')
        logs = '\n'.join(p.read_text() for p in sorted(directory.glob(mode + '-worker-*.log')))
        counts = [list(map(int, m.groups())) for m in PATTERN.finditer(logs)]
        require(counts == row['inventory_counts'] == ([COUNTS] if mode == 'on' else []), 'native inventory counts/full round')
        require(mode == 'on' or 'demand inventory:' not in logs, 'off inventory absence')
        require('link:' in logs and 'exit status: 0' in logs and '1 rounds' in logs, 'real full native link/round')
    return state['toolchain_before']


def runtime_audit(root, name, h, source, tc, version):
    directory = root / 'runtime' / name
    meta, rows, summary, suite = [load(directory / (n + '.json')) for n in ('metadata', 'records', 'summary', 'suite')]
    check_identity(meta, source, tc)
    require(meta['status'] == 'complete' and meta['source_restored'] is True and meta['toolchain_links_valid'] is True, 'runtime completion/restoration')
    require(meta['tools_before'] == meta['tools_after'], 'runtime tool unchanged')
    expected_tools = {'reference': REFERENCE, 'engine': OLD_ENGINE if name == 'rsb21' else ENGINE}
    require({s: v['sha256'] for s, v in meta['tools_before'].items()} == expected_tools, 'runtime exact tools')
    require(len(rows) == 22, 'all 22 runtime rows')
    require(suite['metadata'] == meta, 'suite metadata matches raw metadata')
    for filename, digest in suite['artifacts_sha256'].items():
        require(sha(safe(directory, filename).read_bytes()) == digest, 'runtime suite raw artifact hash')
    original = (directory / 'source.original').read_bytes()
    require(sha(original) == source['src/summary.rs'], 'runtime original source')
    require(original == (root / 'input/source63/src/summary.rs').read_bytes(), 'source63 original binding')
    keys = set()
    samples = {'reference': [], 'engine': []}
    for row in rows:
        valid_process(row)
        stack, phase, rep = row['stack'], row['phase'], row['replicate']
        key = (stack, phase, rep)
        require(key not in keys, 'duplicate runtime row')
        keys.add(key)
        text = raw(directory, f'{stack}-{phase}-{rep}')
        parsed = h.runtime.test_results(text)
        require(json.loads(json.dumps(parsed)) == row['parsed'], 'runtime parsed raw parity')
        contents = h.runtime.edit_bytes(original, rep) if phase == 'edit' else h.runtime.edit_bytes(original, kind={'type-error': 'type-error', 'assert-failure': 'test-failure'}[phase]) if phase in ('type-error', 'assert-failure') else original
        require(sha(contents) == row['source_sha256'], 'exact runtime source edit identity')
        cmd = [meta['tools_before'][stack]['path']] + (['--version'] if phase == 'version' else (['cargo'] if stack == 'reference' else []) + ['test', *h.FILTER])
        require(row['command'] == cmd, 'runtime direct native/reference command')
        if phase == 'version':
            require(row['status'] == 0 and text == meta['versions'][stack]['text'], 'raw version receipt')
            if stack == 'engine' and name == 'rsa27':
                require(text.strip() == version, 'built engine runtime version')
        elif phase == 'type-error':
            require(row['status'] == 101 and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text and row['gate_passed'] is True and parsed is None, 'actual type error')
        elif phase == 'assert-failure':
            require(row['status'] == 101 and 'JR_RUNTIME_FAILURE' in text and row['gate_passed'] is True and h.valid_tests(parsed, failing=True), 'actual assertion failure')
        else:
            require(row['status'] == 0 and h.valid_tests(parsed), 'runtime success/recovery')
            if phase == 'edit':
                require(row['accepted'] is True and re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text) == [f'JR_RUNTIME_EXECUTED_{rep + 1}'], 'executed edit marker')
                samples[stack].append(row['elapsed_seconds'])
        for k, v in row['environment'].items():
            if k in meta['environments'][stack]:
                require(v == meta['environments'][stack][k], 'row launcher environment mismatch')
    expected = {(stack, phase, 0) for stack in samples for phase in ('version', 'prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')}
    expected |= {(stack, 'edit', rep) for stack in samples for rep in range(5)}
    require(keys == expected, 'full runtime error/recovery/edit coverage')
    for stack, values in samples.items():
        require(len(values) == 5 and values == suite['measurements'][stack]['samples_seconds'], 'runtime samples join')
        require(statistics.median(values) == summary['medians_seconds'][stack] == suite['measurements'][stack]['median_seconds'], 'runtime median replay')
    require(summary['source_built_parity'] is True and summary['excluded'] == [], 'runtime parity no exclusions')
    return summary['medians_seconds']


def safe_environments(value):
    if isinstance(value, list):
        for item in value:
            safe_environments(item)
    elif isinstance(value, dict):
        for key, item in value.items():
            if key in ('env', 'environment', 'environments', 'startup_controls') and isinstance(item, dict):
                if all(isinstance(v, str) for v in item.values()):
                    require(set(item) <= ENV_ALLOW, 'unexpected environment key')
                    for control in ('RA_JIT_DEMAND_STD', 'RA_JIT_DEMAND_INVENTORY'):
                        require(control not in item or item[control] in ('0', '1'), 'invalid startup control')
                else:
                    for child in item.values():
                        require(isinstance(child, dict), 'environment receipt shape')
                        safe_environments({'environment': child})
            else:
                safe_environments(item)


def validate(root):
    root = root.resolve()
    manifest(root)
    for p in root.rglob('*.json'):
        if p.name != 'validation.json':
            safe_environments(load(p))
    source_audit(root)
    h = modules(root)
    entries = load(root / 'input/tree.json')
    require(sha((root / 'input/tree.json').read_bytes()) == 'a36bc6039b80b1f03838e4c6ccf20ac96641fd46bc2b48050d24848c37bd1574', 'pinned tree')
    source = {e['path']: e['sha256'] for e in entries}
    require(len(source) == 63, '63 frozen source inputs')
    for e in entries:
        data = safe(root / 'input/source63', e['path']).read_bytes()
        require(sha(data) == e['sha256'] and hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() == e['git_blob'], 'source63 blob/hash')
    version = gate_audit(root)
    tc = probe_audit(root, h, source)
    medians = {name: runtime_audit(root, name, h, source, tc, version) for name in ('rsb21', 'rsa27')}
    before, after = [medians[n] for n in ('rsb21', 'rsa27')]
    normalized = ((after['engine'] / after['reference']) / (before['engine'] / before['reference']) - 1) * 100
    require(abs(normalized + 4.92) < .01 and abs(normalized) < 10, 'descriptive normalized result/floor')
    return {'status': 'passed', 'scope': 'stage1 native input metadata before finish only',
            'native_calls_during_replay': 0, 'compile_errors_v35': 3, 'codegen_tests_v36': 48, 'resident_tests_v36': 360,
            'cold_calls': 2, 'tests_per_cold_call': 4, 'inventory_counts': COUNTS, 'runtime_rows_per_session': 22,
            'runtime_medians_seconds': medians, 'reference_normalized_percent': normalized,
            'noise': 'unknown', 'speed_claim': False, 'zero_semantic_overhead_claim': False,
            'unavailable': ['patch inventory', 'unwind/backend finalization data', 'exact final closure', 'stable cache keys', 'persistent artifact reuse']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=HERE)
    parser.add_argument('--seal', action='store_true')
    args = parser.parse_args()
    if args.seal:
        seal(args.root)
    print(json.dumps(validate(args.root), indent=2))

if __name__ == '__main__':
    main()
