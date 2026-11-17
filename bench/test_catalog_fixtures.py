"""Pinned catalog fixture repairs and real original-assertion acceptance.

Anthropic's picker unions cached API and OAuth scopes, including scopes left
by preceding tests. Removing just ANTHROPIC_API_KEY cannot clear those scopes
or persisted credentials. OpenAI's loaded API-key-shaped credential validates
its fingerprinted scope, not the active OAuth account seeded by the old test.
"""
from pathlib import Path
import difflib
import hashlib
import os
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import suite

PATCH = 'bench/patches/jcode-catalog-fixtures-v1.patch'
ANTHROPIC = 'crates/jcode-base/src/provider/tests/catalog_subscription.rs'
OPENAI = 'crates/jcode-provider-openai-runtime/src/openai_tests/models_state.rs'
INPUTS = (ANTHROPIC, OPENAI)


def original_source(source):
    repo, revision = suite.SNAPSHOTS['desktop']['repos']['jcode']
    return subprocess.check_output(['git', '-C', str(repo), 'show', f'{revision}:{source}'], text=True)


def corrections(source, text):
    if source == ANTHROPIC:
        start = text.index('fn test_anthropic_live_catalog_replaces_static_fallback_list() {\n')
        end = text.index('\n}\n', start)
        body = text[start:end]
        body = body.replace('    let _guard = crate::storage::lock_test_env();\n', '', 1)
        head, rest = body.split('\n', 1)
        body = head + '\n    with_clean_provider_test_env(|| {\n' + ''.join('    ' + line if line.strip() else line for line in rest.splitlines(True)) + '\n    });'
        return text[:start] + body + text[end:]
    seed = '''    jcode_base::provider::populate_account_models(vec![
        "gpt-5.6-sol".to_string(),
        "gpt-5.1-codex".to_string(),
        "gpt-5.1-codex-mini".to_string(),
        "gpt-5.2-codex".to_string(),
    ]);
'''
    assert text.count(seed) == 1
    text = text.replace(seed, '', 1)
    anchor = '    let provider = OpenAIProvider::new(creds);\n'
    assert text.count(anchor) == 1
    scoped = seed.replace('populate_account_models(vec![', 'populate_account_models_for_scope(\n        &provider.catalog_scope(),\n        vec![').replace('        "gpt-', '            "gpt-').replace('    ]);', '        ],\n    );')
    return text.replace(anchor, anchor + '    // Validation uses the loaded credential scope, not the OAuth account slot.\n' + scoped, 1)


def patch_text():
    return ''.join(''.join(difflib.unified_diff(original_source(source).splitlines(True), corrections(source, original_source(source)).splitlines(True), fromfile='a/' + source, tofile='b/' + source)) for source in INPUTS)


class CatalogFixtures(unittest.TestCase):
    def test_patch_is_exact_pinned_correction(self):
        self.assertEqual((suite.ROOT / PATCH).read_text(), patch_text())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for source in INPUTS:
                target = root / source
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(original_source(source))
            subprocess.run(['git', 'apply', str(suite.ROOT / PATCH)], cwd=root, check=True)
            for source in INPUTS:
                self.assertEqual((root / source).read_text(), corrections(source, original_source(source)))

    def test_every_original_assertion_is_preserved(self):
        for source in INPUTS:
            original = original_source(source)
            corrected = corrections(source, original)
            # Whitespace aside, each assertion and its full argument expression
            # remains identical and in order, not merely the assertion count.
            import re
            assertions = re.findall(r'assert(?:_eq|_ne)?!\([\s\S]*?\);', original)
            normalized = ''.join(corrected.split())
            cursor = 0
            for assertion in assertions:
                needle = ''.join(assertion.split())
                position = normalized.find(needle, cursor)
                self.assertGreaterEqual(position, cursor)
                cursor = position + len(needle)
            self.assertEqual(len(assertions), len(re.findall(r'assert(?:_eq|_ne)?!\(', corrected)))

    def test_only_two_observed_fixture_bodies_change(self):
        names = {ANTHROPIC: 'test_anthropic_live_catalog_replaces_static_fallback_list',
                 OPENAI: 'test_openai_supports_codex_models'}
        for source, name in names.items():
            original = original_source(source)
            corrected = corrections(source, original)
            anchor = 'fn ' + name + '() {\n'
            before, body = original.split(anchor, 1)
            _, after = body.split('\n}\n', 1)
            changed_before, changed_body = corrected.split(anchor, 1)
            _, changed_after = changed_body.split('\n}\n', 1)
            self.assertEqual(before, changed_before)
            self.assertEqual(after, changed_after)
        text = (suite.ROOT / PATCH).read_text()
        self.assertEqual(text.count('\n+++ b/'), 2)
        self.assertNotIn('--test-threads', text)
        self.assertNotIn('#[ignore]', text)

    def test_registered_hashes(self):
        spec = suite.SNAPSHOTS['desktop']
        self.assertIn(('jcode', PATCH), spec['patches'])
        self.assertEqual(hashlib.sha256((suite.ROOT / PATCH).read_bytes()).hexdigest(), spec['fixture_patch_sha256'][PATCH])
        identity = suite.snapshot_fingerprint('desktop')
        with patch.dict(suite.SNAPSHOTS, {'desktop': dict(spec, patches=[item for item in spec['patches'] if item != ('jcode', PATCH)])}):
            self.assertNotEqual(identity, suite.snapshot_fingerprint('desktop'))
        for source in INPUTS:
            self.assertEqual(hashlib.sha256(original_source(source).encode()).hexdigest(), spec['fixture_input_sha256']['jcode/' + source])
        with tempfile.TemporaryDirectory() as directory:
            bad = Path(directory) / 'corrupt'
            bad.write_text('unversioned fixture bytes')
            with self.assertRaisesRegex(ValueError, 'corrupt or unversioned fixture patch'):
                suite.verify_fixture_patch(bad, spec['fixture_patch_sha256'][PATCH])
            for source in INPUTS:
                with self.assertRaisesRegex(ValueError, 'unexpected pinned fixture input SHA256'):
                    suite.verify_fixture_input(bad, spec['fixture_input_sha256']['jcode/' + source])


# Added only to disposable diagnostic sources, never to the measured patch.
ANTHROPIC_CONTROL = '''
#[test]
fn benchmark_fixture_prior_anthropic_catalog_control() {
    let home = tempfile::tempdir().unwrap();
    crate::env::set_var("JCODE_HOME", home.path());
    for key in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN",
                "JCODE_ANTHROPIC_AUTH", "JCODE_ANTHROPIC_API_KEY_NAME",
                "JCODE_ANTHROPIC_ENV_FILE"] {
        crate::env::remove_var(key);
    }
    crate::provider::models::reset_model_catalog_services_for_tests();
    // This stale API scope is returned even when no API credential is loaded.
    populate_anthropic_models_for_scope(
        &anthropic_catalog_scope_for_route(false),
        known_anthropic_model_ids(),
    );
    test_anthropic_live_catalog_replaces_static_fallback_list();
}
'''
OPENAI_CONTROL = '''
#[test]
fn benchmark_fixture_credential_catalog_control() {
    // A valid catalog for this same API-key-shaped credential, but not the
    // active OAuth account. The original body seeds the wrong scope.
    let scope = OpenAIProvider::catalog_scope_for(&CodexCredentials {
        access_token: "test".to_string(), refresh_token: String::new(),
        id_token: None, account_id: None, expires_at: None,
    });
    jcode_base::provider::populate_account_models_for_scope(
        &scope, vec!["gpt-5.6-sol".to_string()],
    );
    test_openai_supports_codex_models();
}
'''


def acceptance(repo, target, full=False):
    """Use an existing DISPOSABLE prepared snapshot, sequential real binaries.

    Local execution is necessary to test these exact disposable sources and
    reuse the coordinator's already-built artifacts without hosted contention.
    No workspace-wide builds, production changes or pin changes are performed.
    """
    import runtime_context
    os.environ[runtime_context.SELECTOR] = runtime_context.POLICY
    live_repo = Path(suite.SNAPSHOTS['desktop']['repos']['jcode'][0]).resolve()
    if repo.resolve() == live_repo or live_repo in repo.resolve().parents:
        raise ValueError('acceptance requires a disposable snapshot, never live jcode')
    env = os.environ.copy()
    env[runtime_context.SELECTOR] = runtime_context.POLICY
    env.setdefault('JUSTRUST_TOOLCHAIN', 'system')
    env['CARGO_TARGET_DIR'] = str(target.resolve())
    cases = ((ANTHROPIC, 'jcode-base', 'benchmark_fixture_prior_anthropic_catalog_control', ANTHROPIC_CONTROL, 'claude-opus-4-6'),
             (OPENAI, 'jcode-provider-openai-runtime', 'benchmark_fixture_credential_catalog_control', OPENAI_CONTROL, "Unsupported OpenAI model 'gpt-5.1-codex'"))
    saved = {source: (repo / source).read_bytes() for source in INPUTS}
    try:
        for source, package, name, control, failure in cases:
            for variant in ('before', 'after'):
                text = original_source(source)
                if variant == 'after':
                    text = corrections(source, text)
                (repo / source).write_text(text + control)
                print(f'CATALOG ACCEPTANCE {variant} {name}', flush=True)
                with runtime_context.runtime_context(env, repo) as child:
                    result = subprocess.run(['justrust', 'test', '--local=controlled pinned catalog fixture diagnostic', '-p', package, '--lib', name, '--', '--nocapture'], cwd=repo, env=child, capture_output=True, text=True)
                print(result.stdout, end='', flush=True)
                print(result.stderr, end='', file=sys.stderr, flush=True)
                output = result.stdout + result.stderr
                if result.returncode != (101 if variant == 'before' else 0):
                    raise RuntimeError(f'{variant} {name}: unexpected exit {result.returncode}')
                if variant == 'before':
                    location = 'catalog_subscription.rs:162:5' if source == ANTHROPIC else 'models_state.rs:30:41'
                    if failure not in output or location not in output or '0 passed; 1 failed' not in output:
                        raise RuntimeError(f'{name}: exact original assertion failure not reproduced')
                if variant == 'after' and not ('1 passed; 0 failed' in output or '1 passed, 0 failed' in output):
                    raise RuntimeError(f'{name}: corrected control did not pass')
            (repo / source).write_bytes(saved[source])
        if full:
            for source in INPUTS:
                (repo / source).write_text(corrections(source, original_source(source)))
            # Apply the independently registered endpoint fixture on this file.
            subprocess.run(['git', 'apply', '--include=' + OPENAI, str(suite.ROOT / 'bench/patches/jcode-openai-endpoint-fixture-lock-v1.patch')], cwd=repo, check=True)
            for package in ('jcode-base', 'jcode-provider-openai-runtime'):
                with runtime_context.runtime_context(env, repo) as child:
                    subprocess.run(['justrust', 'test', '--local=whole affected pinned catalog binary acceptance', '-p', package, '--lib'], cwd=repo, env=child, check=True)
    finally:
        for source, data in saved.items():
            (repo / source).write_bytes(data)


if __name__ == '__main__':
    if sys.argv[1:] == ['--write-patch']:
        (suite.ROOT / PATCH).write_text(patch_text())
    elif len(sys.argv) in (4, 5) and sys.argv[1] == '--accept':
        if len(sys.argv) == 5 and sys.argv[4] != '--full':
            raise ValueError('only --full is supported')
        acceptance(Path(sys.argv[2]).resolve(), Path(sys.argv[3]), full=sys.argv[4:] == ['--full'])
    else:
        unittest.main()
