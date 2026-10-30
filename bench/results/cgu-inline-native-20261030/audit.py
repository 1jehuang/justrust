#!/usr/bin/env python3
"""Detached offline audit of native CGU evidence. Never invokes native code or git."""
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import tarfile
import tempfile

if not __debug__:
    raise RuntimeError('Run without -O so all evidence assertions execute')
HERE = Path(__file__).resolve().parent
COMPILER = '570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144'
DRIVER = '29ea709935d751ce13e77995173659006c1e15683e0f0645616d12cd5a3a1665'
SOURCE_HASHES = {
    'toolchain/rustc/compiler/rustc_monomorphize/src/partitioning.rs': 'fd269dc9cb3dc92599839ca127f799921f613088fc50a9fbd05ebbba5a56134f',
    'toolchain/rustc/compiler/rustc_session/src/options.rs': '6301a8c55991317045183d0e1ae2673dbab48b13bf08f9b68686f3922e62ff6b',
}


def extract(archive_path, target):
    with tarfile.open(archive_path) as archive:
        for member in archive.getmembers():
            path = Path(member.name)
            assert not path.is_absolute() and '..' not in path.parts
            if member.isdir():
                continue
            assert member.isfile()
            output = target / path
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(archive.extractfile(member).read())


def load_harness(tree):
    spec = importlib.util.spec_from_file_location('detached_cgu_' + tree.name, tree / 'bench/reuse-cgu-inline-accept.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def identity(report):
    assert report['identity_before'] == report['identity_after']
    value = report['identity_before']
    assert value['sha256'] == COMPILER
    assert [driver['sha256'] for driver in value['drivers']] == [DRIVER]


def audit_semantics(report, c):
    identity(report)
    assert report['semantic_verdict'] == 'passed'
    assert report['verify_ich'] is True and report['flag'] == c.FLAG
    assert report['diagnostics'] == 'exact-bytes-no-normalization'
    assert report['planned_cases'] == report['completed_cases'] == 96
    assert report['planned_main_compiles'] == 672
    assert report['configurations'] == c.configurations()
    assert len(report['observations']) == 96
    assert len(c.cases()) == 12 and len(c.configurations()) == 8
    assert set(report['feature_probe']) == {'no', 'yes'}
    assert all(r['status'] == 0 for r in report['feature_probe'].values())
    persistent = {}
    count = 0
    index = 0
    for config in c.configurations():
        for revision, fixture in enumerate(c.cases()):
            observation = report['observations'][index]
            index += 1
            assert observation['config'] == config and observation['fixture'] == fixture
            modes = observation['modes']
            assert set(modes) == {name for name, _, _ in c.schedule(revision, config)}
            reference = modes['fresh-no']
            for name, enabled, cache in c.schedule(revision, config):
                result = modes[name]
                c.H.validate_expected(result, fixture['expected'], name)
                for component in ('compile', 'runtime'):
                    if component in reference:
                        c.H.compare(reference[component], result[component], config['name'] + '/' + name)
                command = result['compile']['command']
                original = reference['compile']['command']
                for flag in ('-Zincremental-verify-ich', '--edition=2024',
                             f'{c.FLAG}={"yes" if enabled else "no"}',
                             f'-Ccodegen-units={config["codegen_units"]}', f'-Copt-level={config["opt_level"]}'):
                    assert flag in command
                assert command[1] == original[1]
                assert command[command.index('-o') + 1] == original[original.index('-o') + 1]
                caches = [a for a in command if a.startswith('incremental=')]
                assert len(caches) == int(config['incremental'])
                if caches:
                    assert caches[0].endswith('/incremental/' + cache)
                    if not name.startswith('fresh-'):
                        assert persistent.setdefault(cache, caches[0]) == caches[0]
                count += 1
    assert count == 672
    return count


def audit_profiles(evidence, c, tree, prefix, negative=False):
    observations = evidence['observations']
    assert len(observations) == 4
    reference = observations[0]['result']
    caches = {False: set(), True: set()}
    paths = set()
    for index, observation in enumerate(observations):
        name, enabled = ('cold' if index < 2 else 'warm'), bool(index % 2)
        assert (observation['name'], observation['enabled']) == (name, enabled)
        assert observation['fixture']['source'] == c.BASE
        assert observation['fixture']['expected'] == c.EXPECTED
        config = observation['config']
        assert (config['codegen_units'], config['incremental'], config['opt_level']) == (1, True, 0)
        result = observation['result']
        c.H.validate_expected(result, c.EXPECTED, name)
        for component in ('compile', 'runtime'):
            c.H.compare(reference[component], result[component], name)
        command = result['compile']['command']
        for flag in ('-Zself-profile-events=generic-activity', '-Zincremental-verify-ich',
                     '-Ccodegen-units=1', '-Copt-level=0', f'{c.FLAG}={"yes" if enabled else "no"}'):
            assert flag in command
        caches[enabled].add(next(a for a in command if a.startswith('incremental=')))
        profile = observation['profile']
        paths.add(profile['path'])
        actual = c.profile_evidence(tree / prefix / 'mechanism-profiles' / name / ('yes' if enabled else 'no'))
        for field in ('sha256', 'bytes', 'markers', 'records', 'unresolved_records', 'metadata'):
            assert actual[field] == profile[field], (prefix, name, enabled, field)
    assert len(paths) == 4
    assert len(caches[False]) == len(caches[True]) == 1 and caches[False].isdisjoint(caches[True])
    if negative:
        assert 'proof' not in evidence
        assert [o['profile']['markers'][c.MARKERS[1]] for o in observations] == [0, 0, 0, 0]
        assert [o['profile']['markers'][c.MARKERS[0]] for o in observations] == [0, 23, 0, 23]
        try:
            c.validate_mechanism(observations)
        except AssertionError as error:
            assert str(error) == 'No positive native shared traversal and duplicate DFS avoidance evidence'
        else:
            raise AssertionError('Original positive-proof gap unexpectedly passed')
        return None
    proof = c.validate_mechanism(observations)
    assert proof == evidence['proof']
    assert proof['marker_totals'] == dict(zip(c.MARKERS, (46, 2)))
    return proof


def audit(tree, original, repaired):
    old = json.loads((tree / 'acceptance/acceptance.json').read_text())
    new = json.loads((tree / 'acceptance-v2/acceptance.json').read_text())
    focused = json.loads((tree / 'mechanism-repair-v2/focused-evidence.json').read_text())
    assert old['verdict'] == old['mechanism_verdict'] == 'failed'
    assert old['error'] == 'No positive native shared traversal and duplicate DFS avoidance evidence'
    assert new['verdict'] == new['mechanism_verdict'] == 'passed'
    assert old['mechanism_required'] and new['mechanism_required']
    identity(focused)
    assert focused['verdict'] == 'passed'
    assert (tree / 'acceptance.status').read_text().strip() == '1'
    assert (tree / 'acceptance-v2.status').read_text().strip() == '0'
    assert (tree / 'build.status').read_text().strip() == '0'
    assert (tree / 'build-freeze.status').read_text().strip() == '0'
    assert old['error'] in (tree / 'acceptance.log').read_text()
    assert json.loads((tree / 'acceptance-v2.log').read_text())['verdict'] == 'passed'
    assert (tree / 'compiler-version.txt').read_bytes() == base64.b64decode(new['identity_before']['version']['stdout_b64'])
    assert {line.split()[0] for line in (tree / 'identity-before.sha256').read_text().splitlines()} == {COMPILER, DRIVER}
    ready = (tree / 'source-ready').read_text()
    for path, expected in SOURCE_HASHES.items():
        assert hashlib.sha256((tree / 'source' / path).read_bytes()).hexdigest() == expected
        assert f'{Path(path).name} sha256 {expected}' in ready
    audit_profiles(old['mechanism_evidence'], original, tree, 'acceptance', negative=True)
    proof = audit_profiles(new['mechanism_evidence'], repaired, tree, 'acceptance-v2')
    audit_profiles(focused, repaired, tree, 'mechanism-repair-v2')
    return dict(verdict='passed', original_semantic_modes=audit_semantics(old, original),
                repaired_semantic_modes=audit_semantics(new, repaired), raw_profiles_reparsed=12,
                original_mechanism_verdict='failed-as-required', repaired_mechanism_proof=proof,
                compiler_sha256=COMPILER, driver_sha256=DRIVER, native_source_hashes=SOURCE_HASHES,
                source_commit='66e0f9273', original_harness_commit='719c916df', repaired_harness_commit='20e3f2a5e')


def prepare(tree):
    extract(HERE / 'native-evidence.tar.gz', tree)
    extract(HERE / 'harness-original.tar.gz', tree / 'original')
    extract(HERE / 'harness-repaired.tar.gz', tree / 'repaired')
    extract(HERE / 'native-source.tar.gz', tree / 'source')
    return load_harness(tree / 'original'), load_harness(tree / 'repaired')


def main():
    with tempfile.TemporaryDirectory() as temp:
        original, repaired = prepare(Path(temp))
        summary = audit(Path(temp), original, repaired)
        summary['archive_sha256'] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(HERE.glob('*.tar.gz'))}
        print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
