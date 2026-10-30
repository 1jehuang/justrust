#!/usr/bin/env python3
"""Portable retained-evidence audit. Never invokes a compiler or fixture binary."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
spec = importlib.util.spec_from_file_location('tt_accept_audit_harness', ROOT / 'bench/reuse-declarative-macro-accept.py')
H = importlib.util.module_from_spec(spec)
spec.loader.exec_module(H)
if not __debug__:
    raise RuntimeError('Run audit without -O so all evidence assertions execute')
DRIVER = 'ac116b9398e1f5c677e73952b8719be9d9e47190fe61fbaeba9b7cf270739595'
COMPILER = '570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144'


def audit_semantics(report):
    assert report['identity_before'] == report['identity_after']
    assert report['semantic_verdict'] == 'passed'
    assert report['planned_cases'] == report['completed_cases'] == 51
    assert report['verify_ich'] is True
    fixtures = H.cases() + H.tt_cases()
    assert len(report['observations']) == len(fixtures)
    checked = 0
    for index, (observation, fixture) in enumerate(zip(report['observations'], fixtures)):
        assert observation['fixture'] == fixture
        assert 'unsupported' not in observation
        modes = observation['modes']
        assert set(modes) == {name for name, _, _ in H.schedule(index)}
        reference = modes['fresh-no']
        for mode, enabled, cache in H.schedule(index):
            result = modes[mode]
            H.validate_expected(result, fixture['expected'], mode)
            command = result['compile']['command']
            assert '-Zincremental-verify-ich' in command
            assert f'{H.FLAG}={"yes" if enabled else "no"}' in command
            assert any(arg.endswith(f'/incremental/{cache}/binary') for arg in command)
            assert command[1] == reference['compile']['command'][1]
            assert command[command.index('-o') + 1] == reference['compile']['command'][reference['compile']['command'].index('-o') + 1]
            for component in ('dependency', 'compile', 'runtime'):
                if component in reference:
                    H.compare(reference[component], result[component], f'{index}/{mode}/{component}')
            checked += 1
    assert checked == 357
    return checked


def audit_proof(report, key, tree=None, negative=False):
    evidence = report[key]
    assert evidence['marker_schema'] == list(H.MARKERS)
    assert evidence['profile_events'] == 'default,function-args'
    observations = evidence['observations']
    assert [o['name'] for o in observations] == ['off-before', 'cold-yes', 'warm-yes', 'body-edit-yes', 'off-after']
    first = observations[0]['result']
    paths = set()
    caches = []
    source = H.TT if key == 'tt_evidence' else H.LITERAL
    for observation in observations:
        edited = observation['name'] in ('body-edit-yes', 'off-after')
        expected_source = source.replace('{ 1 }', '{ 2 }').replace('!= 43', '!= 44') if edited else source
        assert observation['fixture']['source'] == expected_source
        result = observation['result']
        command = result['compile']['command']
        enabled = observation['name'] not in ('off-before', 'off-after')
        assert f'{H.FLAG}={"yes" if enabled else "no"}' in command
        assert '-Zincremental-verify-ich' in command
        assert '-Zself-profile-events=default,function-args' in command
        caches.append(next(arg for arg in command if arg.startswith('incremental=')))
        H.validate_expected(result, '', observation['name'])
        for component in ('compile', 'runtime'):
            H.compare(first[component], result[component], observation['name'])
        profile = observation['profile']
        paths.add(profile['path'])
        if tree is not None:
            profile_dir = 'tt-profiles' if key == 'tt_evidence' else 'native-profiles'
            directory = tree / 'acceptance-tt' / profile_dir / observation['name']
            actual = H.profile_evidence(directory)
            for field in ('sha256', 'bytes', 'markers', 'records', 'unresolved_records', 'metadata'):
                assert actual[field] == profile[field], (key, observation['name'], field)
    assert len(paths) == 5
    assert len(set(caches[1:])) == 1 and caches[0] != caches[1]
    if negative:
        cold = observations[1]['profile']['markers']
        assert cold[H.MARKERS[0]] == cold[H.MARKERS[2]] == cold[H.MARKERS[3]] == cold[H.MARKERS[4]] == 0
        assert cold[H.MARKERS[1]] > 0
        assert 'proof' not in evidence
        try:
            H.validate_native_markers(observations, 'tt')
        except AssertionError as error:
            assert str(error) == 'Cold tt fixture did not positively exercise native eligible/provider/success paths'
        else:
            raise AssertionError('Literal-only negative unexpectedly passed TT proof')
        return
    proof = H.validate_native_markers(observations, 'tt' if key == 'tt_evidence' else 'literal')
    assert proof['inferred_provider_avoided'] == evidence['proof']['inferred_provider_avoided']
    return proof['inferred_provider_avoided']


def main():
    with tempfile.TemporaryDirectory() as temp, tarfile.open(HERE / 'native-evidence.tar.gz') as archive:
        tree = Path(temp)
        # Explicit regular-file copying avoids tar extraction links and traversal.
        for member in archive.getmembers():
            path = Path(member.name)
            assert not path.is_absolute() and '..' not in path.parts
            if member.isdir():
                continue
            assert member.isfile()
            target = tree / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(archive.extractfile(member).read())
        positive = json.loads((tree / 'acceptance-tt/acceptance.json').read_text())
        negative = json.loads((tree / 'tt-literal-negative/acceptance.json').read_text())
        assert positive['verdict'] == positive['native_hit_verdict'] == positive['tt_hit_verdict'] == 'passed'
        assert positive['native_hit_required'] and positive['tt_hit_required']
        identity = positive['identity_before']
        assert identity['sha256'] == COMPILER
        assert [d['sha256'] for d in identity['drivers']] == [DRIVER]
        before = (tree / 'identity-tt-before.sha256').read_text().splitlines()
        assert {line.split()[0] for line in before} == {COMPILER, DRIVER}
        assert (tree / 'identity-tt-after.log').read_text().splitlines() == [line.split(maxsplit=1)[1].strip() + ': OK' for line in before]
        assert negative['verdict'] == negative['tt_hit_verdict'] == 'failed'
        assert negative['native_hit_verdict'] == 'passed'
        assert negative['error'] == 'Cold tt fixture did not positively exercise native eligible/provider/success paths'
        assert negative['identity_before']['drivers'][0]['sha256'] != DRIVER
        assert 'Cold tt fixture' in (tree / 'tt-literal-negative.log').read_text()
        audit_semantics(negative)
        audit_proof(negative, 'native_evidence')
        audit_proof(negative, 'tt_evidence', negative=True)
        summary = dict(verdict='passed', semantic_modes=audit_semantics(positive),
                       negative_semantic_modes=357, native_profiles_reparsed=10,
                       compiler_sha256=COMPILER, driver_sha256=DRIVER,
                       literal_provider_avoided=audit_proof(positive, 'native_evidence', tree),
                       tt_provider_avoided=audit_proof(positive, 'tt_evidence', tree),
                       archive_sha256=hashlib.sha256((HERE / 'native-evidence.tar.gz').read_bytes()).hexdigest())
        print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
