#!/usr/bin/env python3
"""Offline release-build and native acceptance audit, no native invocations."""
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import tomllib

COMPILER = '64689a080e88a1aece4307ee5012a59bc0e89d54d3ec208dd16cb3102c60156d'
DRIVER = '961a7344a71a7b1997557fa4167cc44305daa341ea2aee920ff88ee1e709ea43'
SOURCE_HASHES = {
    'compiler/rustc_monomorphize/src/partitioning.rs': 'fd269dc9cb3dc92599839ca127f799921f613088fc50a9fbd05ebbba5a56134f',
    'compiler/rustc_session/src/options.rs': '6301a8c55991317045183d0e1ae2673dbab48b13bf08f9b68686f3922e62ff6b',
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def audit(root):
    release = root / 'release-build'
    for name in ('build.status', 'build-freeze.status', 'acceptance.status'):
        assert (release / name).read_text() == '0\n'
    assert 'Build completed successfully' in (release / 'build.log').read_text()
    assert 'profile [optimized + debuginfo]' in (release / 'build.log').read_text()
    config = tomllib.loads((release / 'bootstrap-release.toml').read_text())
    assert config['profile'] == 'compiler'
    rust = config['rust']
    assert rust['incremental'] is False and rust['lto'] == 'thin' and rust['codegen-units'] == 1
    assert rust['debug-assertions'] is False and rust['debug-logging'] is False
    assert rust['download-rustc'] is False and rust['codegen-backends'] == ['llvm', 'cranelift']
    assert config['build']['profiler'] is True
    assert not any('pgo' in k or 'bolt' in k or 'profile-generate' in k or 'profile-use' in k for k in rust)
    expected_sources = ''.join(f'{sha}  {name}\n' for name, sha in SOURCE_HASHES.items())
    assert (release / 'source-before.sha256').read_text() == expected_sources
    assert (release / 'source-after.log').read_text() == ''.join(name + ': OK\n' for name in SOURCE_HASHES)
    for name, sha in SOURCE_HASHES.items():
        assert digest(release / 'source' / name) == sha
    assert {line.split()[0] for line in (release / 'identity-before.sha256').read_text().splitlines()} == {COMPILER, DRIVER}
    spec = importlib.util.spec_from_file_location('br48_acceptance', root / 'acceptance-runner/reuse-cgu-inline-accept.py')
    c = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(c)
    report = json.loads((release / 'acceptance/acceptance.json').read_text())
    assert report['identity_before'] == report['identity_after']
    identity = report['identity_before']
    assert identity['sha256'] == COMPILER and [d['sha256'] for d in identity['drivers']] == [DRIVER]
    assert identity['version']['status'] == 0 and identity['sysroot']['status'] == 0
    version = base64.b64decode(identity['version']['stdout_b64'], validate=True)
    assert version == (release / 'compiler-version.txt').read_bytes() == (root / 'rustc-version.stdout').read_bytes()
    assert report['verdict'] == report['semantic_verdict'] == report['mechanism_verdict'] == 'passed'
    assert report['scope'] == 'correctness-only-no-speedup-claim'
    assert report['mechanism_required'] and report['verify_ich'] is True
    assert report['flag'] == c.FLAG and report['diagnostics'] == 'exact-bytes-no-normalization'
    assert report['completed_cases'] == report['planned_cases'] == len(report['observations']) == 96
    assert report['planned_main_compiles'] == 672 and report['configurations'] == c.configurations()
    assert len(c.cases()) == 12 and len(c.configurations()) == 8
    assert set(report['feature_probe']) == {'no', 'yes'}
    assert all(row['status'] == 0 for row in report['feature_probe'].values())
    persistent, count, index = {}, 0, 0
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
                        c.H.compare(reference[component], result[component], name)
                command = result['compile']['command']
                assert command[0] == identity['path']
                for flag in ('-Zincremental-verify-ich', '--edition=2024',
                             f'{c.FLAG}={"yes" if enabled else "no"}',
                             f'-Ccodegen-units={config["codegen_units"]}', f'-Copt-level={config["opt_level"]}'):
                    assert flag in command
                original = reference['compile']['command']
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
    evidence = report['mechanism_evidence']
    assert evidence['marker_schema'] == list(c.MARKERS) and evidence['profile_events'] == 'generic-activity'
    observations = evidence['observations']
    assert len(observations) == 4
    caches, paths = {False: set(), True: set()}, set()
    for index, observation in enumerate(observations):
        name, enabled = ('cold' if index < 2 else 'warm'), bool(index % 2)
        assert (observation['name'], observation['enabled']) == (name, enabled)
        assert observation['fixture']['source'] == c.BASE and observation['fixture']['expected'] == c.EXPECTED
        assert observation['config'] == dict(name='mechanism-cgu1', codegen_units=1, incremental=True, opt_level=0)
        result = observation['result']
        c.H.validate_expected(result, c.EXPECTED, name)
        for component in ('compile', 'runtime'):
            c.H.compare(observations[0]['result'][component], result[component], name)
        command = result['compile']['command']
        assert command[0] == identity['path']
        for flag in ('-Zself-profile-events=generic-activity', '-Zincremental-verify-ich',
                     '-Ccodegen-units=1', '-Copt-level=0', f'{c.FLAG}={"yes" if enabled else "no"}'):
            assert flag in command
        caches[enabled].add(next(a for a in command if a.startswith('incremental=')))
        profile = observation['profile']
        paths.add(profile['path'])
        actual = c.profile_evidence(release / 'acceptance/mechanism-profiles' / name / ('yes' if enabled else 'no'))
        for field in ('sha256', 'bytes', 'markers', 'records', 'unresolved_records', 'metadata'):
            assert actual[field] == profile[field], (name, enabled, field)
    assert len(paths) == 4 and len(caches[False]) == len(caches[True]) == 1
    assert caches[False].isdisjoint(caches[True])
    proof = c.validate_mechanism(observations)
    assert proof == evidence['proof']
    assert proof['marker_totals'] == dict(zip(c.MARKERS, (46, 2)))
    assert json.loads((release / 'acceptance.log').read_text()) == dict(
        verdict='passed', semantic_verdict='passed', mechanism_verdict='passed',
        completed_cases=96, planned_main_compiles=672)
    return count
