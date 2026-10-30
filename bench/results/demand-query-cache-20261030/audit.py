#!/usr/bin/env python3
"""Audit retained demand-cache evidence offline. Never execute native code."""
import hashlib
import base64
import importlib.util
import json
from pathlib import Path
import tarfile
import tempfile

if not __debug__:
    raise RuntimeError('Run without -O so evidence checks execute')
HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('retained_demand_harness', HERE.parents[2] / 'bench/reuse-demand-cache-accept.py')
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)
COMPILER = '570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144'
DRIVER = 'ed21691a9db267ffb71cc3b8975a8cd89768397a7a7f82c9c839083485424efd'


def cache_of(command):
    return next(arg for arg in command if arg.startswith('incremental='))


def audit_semantics(report):
    assert report['verdict'] == report['semantic_verdict'] == report['mechanism_verdict'] == 'passed'
    assert report['flag'] == D.FLAG and report['verify_ich'] is True
    assert report['mechanism_required'] is True
    assert report['diagnostics'] == 'exact-bytes-no-normalization'
    assert report['identity_before'] == report['identity_after']
    assert report['identity_before']['sha256'] == COMPILER
    assert [d['sha256'] for d in report['identity_before']['drivers']] == [DRIVER]
    assert report['planned_cases'] == report['completed_cases'] == 32
    assert len(report['observations']) == 32
    assert len(D.cases()) == 32
    assert set(report['feature_probe']) == {'no', 'yes'}
    assert all(value['status'] == 0 for value in report['feature_probe'].values())
    persistent = {}
    checked = 0
    dependencies = 0
    for index, (observation, fixture) in enumerate(zip(report['observations'], D.cases())):
        assert observation['fixture'] == fixture
        modes = observation['modes']
        assert set(modes) == {name for name, _, _ in D.schedule(index)}
        reference = modes['fresh-no']
        for name, enabled, cache in D.schedule(index):
            result = modes[name]
            D.H.validate_expected(result, fixture['expected'], name)
            for component in ('dependency', 'compile', 'runtime'):
                if component in reference:
                    D.H.compare(reference[component], result[component], f'{index}/{name}/{component}')
            for component in ('dependency', 'compile'):
                if component not in result:
                    continue
                if component == 'dependency':
                    assert result[component]['status'] == 0
                command = result[component]['command']
                original = reference[component]['command']
                assert '-Zincremental-verify-ich' in command
                assert f'{D.FLAG}={"yes" if enabled else "no"}' in command
                assert '--edition=2024' in command
                assert command[1] == original[1]
                assert command[command.index('-o') + 1] == original[original.index('-o') + 1]
                suffix = 'dependency' if component == 'dependency' else 'binary'
                assert cache_of(command).endswith(f'/incremental/{cache}/{suffix}')
                if not name.startswith('fresh-'):
                    key = (cache, component)
                    assert persistent.setdefault(key, cache_of(command)) == cache_of(command)
                dependencies += component == 'dependency'
            checked += 1
    assert checked == 288
    return checked, dependencies


def audit_mechanism(report, tree=None):
    evidence = report['mechanism_evidence']
    assert evidence['marker_schema'] == list(D.MARKERS)
    observations = evidence['observations']
    assert len(observations) == 10
    expected_names = ('cold', 'unchanged', 'demand-narrow', 'demand-narrow-unchanged', 'demand-wide-return')
    caches = {False: set(), True: set()}
    paths = set()
    raw_counts = {}
    for index, observation in enumerate(observations):
        name = expected_names[index // 2]
        enabled = bool(index % 2)
        assert (observation['name'], observation['enabled']) == (name, enabled)
        expected_source = D.NARROW if name.startswith('demand-narrow') else D.DEMAND
        assert observation['fixture']['source'] == expected_source
        expected = '8\n' if name.startswith('demand-narrow') else '19\n'
        assert observation['fixture']['expected'] == expected
        result = observation['result']
        D.H.validate_expected(result, expected, name)
        if enabled:
            for component in ('compile', 'runtime'):
                D.H.compare(observations[index - 1]['result'][component], result[component], name)
        command = result['compile']['command']
        assert f'{D.FLAG}={"yes" if enabled else "no"}' in command
        assert '-Zincremental-verify-ich' in command
        assert '-Zself-profile-events=default,function-args' in command
        caches[enabled].add(cache_of(command))
        paths.add(observation['profile']['path'])
        raw_counts[f'{name}/{"yes" if enabled else "no"}'] = observation['profile']['markers']
        if tree is not None:
            directory = tree / 'acceptance/mechanism-profiles' / name / ('yes' if enabled else 'no')
            actual = D.profile_evidence(directory)
            for field in ('sha256', 'bytes', 'markers', 'records', 'unresolved_records', 'metadata'):
                assert actual[field] == observation['profile'][field], (name, enabled, field)
    assert len(paths) == 10
    assert len(caches[False]) == len(caches[True]) == 1
    assert caches[False].isdisjoint(caches[True])
    proof = D.validate_mechanism(observations)
    assert evidence['proof'] == proof
    return proof, raw_counts


def main():
    with tempfile.TemporaryDirectory() as temp, tarfile.open(HERE / 'native-evidence.tar.gz') as archive:
        tree = Path(temp)
        for member in archive.getmembers():
            path = Path(member.name)
            assert not path.is_absolute() and '..' not in path.parts
            if member.isdir():
                continue
            assert member.isfile()
            target = tree / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(archive.extractfile(member).read())
        report = json.loads((tree / 'acceptance/acceptance.json').read_text())
        version = report['identity_before']['version']
        assert version['status'] == 0
        assert (tree / 'compiler-version.txt').read_bytes() == base64.b64decode(version['stdout_b64'])
        modes, dependencies = audit_semantics(report)
        proof, counts = audit_mechanism(report, tree)
        before = (tree / 'identity-before.sha256').read_text().splitlines()
        assert {line.split()[0] for line in before} == {COMPILER, DRIVER}
        assert (tree / 'identity-after.log').read_text().splitlines() == [line.split(maxsplit=1)[1].strip() + ': OK' for line in before]
        assert (tree / 'acceptance.status').read_text().strip() == '0'
        assert (tree / 'build.status').read_text().strip() == '0'
        cli = json.loads((tree / 'acceptance.log').read_text())
        assert cli == {key: report[key] for key in ('verdict', 'semantic_verdict', 'mechanism_verdict', 'completed_cases')}
        summary = dict(verdict='passed', semantic_modes=modes, dependency_compiles=dependencies,
                       native_profiles_reparsed=10, compiler_sha256=COMPILER, driver_sha256=DRIVER,
                       mechanism_proof=proof, mechanism_marker_counts=counts,
                       archive_sha256=hashlib.sha256((HERE / 'native-evidence.tar.gz').read_bytes()).hexdigest())
        print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
