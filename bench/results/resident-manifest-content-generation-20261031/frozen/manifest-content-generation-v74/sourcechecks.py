#!/usr/bin/env python3
"""Static source checks only. Does not import/compile/run candidate Rust."""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parent
original = (root / 'whole.original.rs').read_text()
candidate_path = Path('<PRIVATE_HOME>/justrust/toolchain/resident/crates/ra-jit/src/cargo/whole.rs')
candidate = candidate_path.read_text()
start = candidate.index('/// Keep the original timestamp path exactly when disabled.')
end = candidate.index('/// When `file` was last written, as a stamp.', start)
stripped = candidate[:start] + candidate[end:]
test_start = stripped.index('#[cfg(test)]\nmod manifest_generation_tests {')
test_end = stripped.index('/// The Rust files of the package in `dir`,', test_start)
stripped = stripped[:test_start] + stripped[test_end:]
stripped = stripped.replace('let generation = checkout_generation(root, manifests.clone(), lock);', 'let generation = manifests.clone().chain([lock]).filter_map(|it| written_at(&it)).max();')
stripped = stripped.replace('        generation,\n        kept: false,', '        generation: generation.unwrap_or(0),\n        kept: false,')
checks = {
    'production_unchanged_except_generation_call_and_helpers': stripped == original,
    'exact_one_content_opt_in_env_read': candidate.count('std::env::var_os("RA_JIT_MANIFEST_CONTENT_GENERATION")') == 1,
    'opt_in_process_first_use': 'static ENABLED: std::sync::OnceLock<bool>' in candidate,
    'exact_value_one': '.is_some_and(|it| it == "1")' in candidate,
    'marker_single_literal_site': candidate.count('ra-jit: manifest-content-generation-v1 entered') == 1,
    'marker_once': 'static ENTERED: std::sync::Once' in candidate and 'ENTERED.call_once' in candidate,
    'kept_false_retained': '        kept: false,' in candidate,
    'original_sources_retained': 'sources: manifests.chain(sources).collect()' in candidate,
    'original_off_timestamp_chain': 'return manifests.chain([lock]).filter_map(|it| written_at(&it)).max().unwrap_or(0);' in candidate,
    'fallback_original_timestamp_chain': 'manifests.into_iter().chain([lock]).filter_map(|it| written_at(&it)).max().unwrap_or(0)' in candidate,
    'hash_domain_version': 'ra-jit/whole/manifest-content-generation/v1' in candidate,
    'content_bit127_domain': 'u128::from_le_bytes(tag) | (1 << 127)' in candidate,
    'full_file_reads_no_metadata_cache': candidate.count('std::fs::read(file)?') == 2,
    'five_real_file_test_sources': candidate[candidate.index('mod manifest_generation_tests {'):candidate.index('/// The Rust files of the package in `dir`,')].count('#[test]') == 5,
    'tempfile_fixture': 'tempfile::tempdir()' in candidate,
    'same_metadata_platform_api': '.set_times(times)' in candidate,
    'lossless_native_path_framing': 'as_encoded_bytes()' in candidate[start:end],
    'no_lossy_helper_paths': 'to_string_lossy' not in candidate[start:end],
}
report = {'kind': 'static source inspection only, not Rust execution', 'checks': checks, 'candidate_sha256': hashlib.sha256(candidate_path.read_bytes()).hexdigest()}
(root / 'sourcechecks.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
if not all(checks.values()):
    raise SystemExit(1)
