#!/usr/bin/env python3
"""Read-only source checks, not native compilation, tests or runtime evidence."""
import hashlib
import json
from pathlib import Path

root = Path('<native-root>/balanced-manifest-syntax-v82')
v81 = root.parent / 'parallel-manifest-syntax-v81'
repo = Path('<repo>')
original = (root / 'originals/toolchain/resident/crates/blaze-manifest/src/manifest.rs').read_text()
candidate = (repo / 'toolchain/resident/crates/blaze-manifest/src/manifest.rs').read_text()
oracle = (repo / 'toolchain/resident/crates/blaze-manifest/tests/parallel_manifest_syntax/mod.rs').read_text()
old_metadata = (root / 'originals/toolchain/resident/crates/blaze-metadata/src/lib.rs').read_text()
metadata = (repo / 'toolchain/resident/crates/blaze-metadata/src/lib.rs').read_text()

old_fn = original.split('pub fn read_manifest(', 1)[1].split('\nfn to_workspace_config(', 1)[0]
old_body = old_fn.split(') -> anyhow::Result<EitherManifest> {\n', 1)[1]
new_body = candidate.split('fn normalize_manifest(', 1)[1].split('\nfn to_workspace_config(', 1)[0].split(') -> anyhow::Result<EitherManifest> {\n', 1)[1]
semantic_body = '    let mut warnings' + old_body.split('    let mut warnings', 1)[1]
assert semantic_body == new_body, 'semantic normalization body differs bytewise'

prefix = old_body.split('    let mut warnings', 1)[0].rstrip()
syntax = candidate.split('fn read_manifest_syntax(', 1)[1].split('\n}\n', 1)[0].split(') -> anyhow::Result<TomlManifest> {\n', 1)[1]
assert syntax.removesuffix('\n    Ok(original_toml)').rstrip() == prefix, 'original sequential syntax prefix changed'

oracle_body = oracle.split('fn original_read_manifest(', 1)[1].split('\nstruct Fixture', 1)[0].split(') -> anyhow::Result<EitherManifest> {\n', 1)[1].rstrip()
assert oracle_body == old_body.rstrip(), 'test oracle is not the full frozen original body'

assert original.split('pub fn read_manifest(', 1)[0] == candidate.split('pub fn read_manifest(', 1)[0]
assert original.split('\nfn to_workspace_config(', 1)[1] == candidate.split('\nfn to_workspace_config(', 1)[1]

old_fetch = old_metadata.split('\nfn fetch(', 1)[1].split('\n/// The registries', 1)[0]
new_fetch = metadata.split('\nfn fetch(', 1)[1].split('\nfn parallel_manifest_parse_enabled', 1)[0]
old_tail = old_fetch.split('    let mut read = Vec::with_capacity(texts.len());', 1)[1]
new_tail = new_fetch.split('    let mut read = Vec::with_capacity(texts.len());', 1)[1]
assert old_tail == new_tail, 'OFF ordered fetch tail differs bytewise'
assert old_fetch.split('    let roots = sources.fetch(&locked)?;', 1)[0] == new_fetch.split('    let roots = {', 1)[0], 'fetch preparation changed'
assert old_metadata.split('\nfn fetch(', 1)[0] == metadata.split('\nfn fetch(', 1)[0]
assert old_metadata.split('\n/// The registries', 1)[1] == metadata.split('\n/// The registries', 1)[1]

worker = candidate.split('#[cfg(all(target_os = "linux", target_arch = "x86_64"))]\nfn preparse_regular_manifests', 1)[1].split('\n#[cfg(test)]', 1)[0]
assert 'ctx' not in worker and 'Context' not in worker
assert 'unsafe impl' not in candidate
assert 'spawn_scoped' in worker and 'join()' in worker
assert 'custom_flags(0x800 | 0x20000)' in worker
assert 'file.metadata().ok()?.is_file()' in worker
assert 'MARKER.call_once' in worker
assert candidate.count('ra-jit: parallel-manifest-parse-v81 batch') == 1
assert 'paths.len() < 8' in candidate
assert '.clamp(2, 4)' in worker
assert 'OnceLock<bool>' in metadata
assert 'RA_JIT_PARALLEL_MANIFEST_PARSE' in metadata
assert 'from.is_remote_registry()' in metadata
assert 'value == std::ffi::OsStr::new("1")' in metadata
assert '.set_var(' not in oracle


# v82 is a bounded scheduling experiment, not an untracked semantic rewrite.
frozen_manifest = (v81 / 'candidates/toolchain/resident/crates/blaze-manifest/src/manifest.rs').read_text()
frozen_metadata = (v81 / 'candidates/toolchain/resident/crates/blaze-metadata/src/lib.rs').read_text()
assert metadata == frozen_metadata, 'metadata differs from frozen v81 factor'
anchor = '#[cfg(all(target_os = "linux", target_arch = "x86_64"))]\nfn preparse_regular_manifests'
assert candidate.split(anchor, 1)[0] == frozen_manifest.split(anchor, 1)[0]
assert candidate.split('\n#[cfg(test)]', 1)[1] == frozen_manifest.split('\n#[cfg(test)]', 1)[1]
scheduler = worker.split('fn schedule_manifest_syntax', 1)[1]
assert 'AtomicUsize::new(0)' in scheduler
assert 'next.fetch_add(1, Ordering::Relaxed)' in scheduler
assert 'local.push((index, read(index)))' in scheduler
assert 'ordered[index] = Some(value)' in scheduler
assert 'ordered.into_iter().collect()' in scheduler
assert 'handle.join()' in scheduler and 'Err(_) => failed = true' in scheduler
assert scheduler.index('handle.join()') < scheduler.index('if failed {')
assert 'paths.chunks' not in worker and 'Mutex' not in scheduler
assert 'writeln!' not in scheduler and 'eprintln!' not in scheduler
assert '!(2..=4).contains(&workers)' in scheduler
assert 'schedule_manifest_syntax(paths.len(), workers' in worker
assert worker.index('schedule_manifest_syntax(paths.len(), workers') < worker.index('MARKER.call_once')
# The rejected v81 source/evidence stays byte-for-byte sealed.
for line in (v81 / 'SHA256SUMS').read_text().splitlines():
    expected, relative = line.split(None, 1)
    path = Path(relative)
    if not path.is_absolute():
        path = v81 / relative
    assert hashlib.sha256(path.read_bytes()).hexdigest() == expected, str(path)
assert (repo / 'bench/resident_suite.py').read_bytes() == (root.parent / 'parallel-manifest-harness-v81/originals/bench/resident_suite.py').read_bytes()
assert (repo / 'docs/resident-suite.md').read_bytes() == (root.parent / 'parallel-manifest-harness-v81/originals/docs/resident-suite.md').read_bytes()
assert not (repo / 'bench/test_resident_parallel_manifest.py').exists()

files = [
    'toolchain/resident/crates/blaze-manifest/src/manifest.rs',
    'toolchain/resident/crates/blaze-metadata/src/lib.rs',
    'toolchain/resident/crates/blaze-manifest/tests/parallel_manifest_syntax/mod.rs',
    'toolchain/resident/crates/blaze-metadata/tests/parallel_manifest_syntax/mod.rs',
    'docs/resident-balanced-manifest-syntax.md',
]
print(json.dumps({
    'status': 'static-source-checks-only-pass',
    'accepted_revision': 'c8829bae02fd234f46012ce6996baa14c09b3a1c',
    'semantic_body_bytewise_unchanged': True,
    'v82_dynamic_index_local_results_ordered_gather': True,
    'v81_frozen_evidence_receipts_unchanged': True,
    'metadata_bytewise_v81_and_live_harness_accepted': True,
    'sequential_syntax_prefix_bytewise_unchanged': True,
    'full_original_test_oracle_bytewise_unchanged': True,
    'off_ordered_fetch_tail_bytewise_unchanged': True,
    'unrelated_existing_file_regions_bytewise_unchanged': True,
    'native_send_compilation': 'NOT RUN, ROOT GATE',
    'native_tests': 'NOT RUN, ROOT GATE',
    'runtime_performance': 'NOT RUN, NO SPEED CLAIM',
    'files': {file: hashlib.sha256((repo / file).read_bytes()).hexdigest() for file in files},
    'manifest_fixture_count': sum(line == '#[test]' for line in oracle.splitlines()),
    'metadata_fixture_count': sum(line == '#[test]' for line in (repo / files[3]).read_text().splitlines()),
}, indent=2, sort_keys=True))
