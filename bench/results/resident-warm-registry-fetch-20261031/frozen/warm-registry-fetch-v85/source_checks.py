from pathlib import Path
import hashlib
import json
import re

ROOT = Path('<EXPERIMENT>/warm-registry-fetch-v85')
REPO = Path('<PRIVATE_HOME>/justrust')
SOURCE = Path('toolchain/resident/crates/blaze-registry/src/download.rs')
original = (ROOT / 'original' / SOURCE).read_text()
candidate = (REPO / SOURCE).read_text()

def method(text, signature):
    start = text.index(signature)
    end = text.index('\n    }\n', start) + len('\n    }\n')
    return text[start:end]

def body(text):
    return text[text.index('{\n') + 2:]

old_fetch = method(original, '    pub fn fetch(')
helper = method(candidate, '    fn fetch_with_warm_registry(')
branch = '''        if enabled && let Some(out) = self.warm_registry_batch(packages) {
            warm_registry_fetch_marker();
            return Ok(out);
        }
'''
assert body(helper).replace(branch, '', 1) == body(old_fetch)
assert method(candidate, '    pub fn fetch_one(') == method(original, '    pub fn fetch_one(')

# Reconstruct the entire accepted file by subtracting only explicitly added regions.
reconstructed = candidate[:candidate.index('\n#[cfg(test)]\nmod warm_registry_readonly_v85_tests')]
reconstructed = reconstructed.replace(method(candidate, '    pub fn fetch(') + '\n' + helper, old_fetch, 1)
warm_start = reconstructed.index('    /// Read-only sufficient proof')
warm_end = reconstructed.index('    /// One package, unpacked', warm_start)
reconstructed = reconstructed[:warm_start] + reconstructed[warm_end:]
flag_start = reconstructed.index('// Exact first-use opt-in.')
flag_end = reconstructed.index('/// Writes a `.crate`', flag_start)
reconstructed = reconstructed[:flag_start] + reconstructed[flag_end:]
assert reconstructed == original

compact = lambda text: re.sub(r'\s+', '', text)
frozen_fetch = method(candidate, '        fn fetch_v85_original(')
# Inner methods are indented one extra level; extract via their known next declaration.
frozen_fetch = candidate.split('        fn fetch_v85_original(', 1)[1].split('\n        fn fetch_one_v85_original(', 1)[0]
frozen_body = frozen_fetch[frozen_fetch.index('{\n') + 2:].rstrip()
# The enclosing method terminator is included, but not the next method.
assert compact(frozen_body.replace('self.fetch_one_v85_original(package)', 'self.fetch_one(package)')) == compact(body(old_fetch))
frozen_one = candidate.split('        fn fetch_one_v85_original(', 1)[1].split('\n    }\n\n    struct NoNetwork', 1)[0]
assert compact(frozen_one[frozen_one.index('{\n') + 2:]) == compact(body(method(original, '    pub fn fetch_one(')))
probe = method(candidate, '    fn warm_registry_batch(')
assert 'is_unpacked(' not in probe
for forbidden in ['remove_', 'write(', 'fetch_one(', 'crate_bytes(', 'unpack_package(']:
    assert forbidden not in probe
assert 'File::open' in probe and 'MAX_PACKAGES: usize = 4096' in probe
assert 'file.take(MAX_MARKER_BYTES + 1)' in probe
assert 'packages.is_empty()' in probe
assert candidate.count('b"ra-jit: warm-registry-fetch-v85 batch\\n"') == 1
assert 'warm-registry-fetch-v84 batch' not in candidate
assert 'std::env::var_os("RA_JIT_WARM_REGISTRY_FETCH").as_deref() == Some(std::ffi::OsStr::new("1"))' in candidate
assert candidate.count('#[test]') == 5

v84 = ROOT.parent / 'warm-registry-fetch-v84'
for line in (v84 / 'SHA256SUMS').read_text().splitlines():
    digest, name = line.split('  ', 1)
    assert hashlib.sha256((v84 / name).read_bytes()).hexdigest() == digest
assert (REPO / 'docs/resident-warm-registry-fetch.md').read_bytes() == (v84 / 'candidate/docs/resident-warm-registry-fetch.md').read_bytes()

checks = {
    'original_off_scheduler_body_bytewise': True,
    'public_fetch_one_bytewise': True,
    'full_original_reconstructed_bytewise': True,
    'independent_frozen_oracle_bodies_tokenwise': True,
    'probe_readonly_source_check': True,
    'exact_flag_and_once_marker_source_check': True,
    'immutable_v84_sha_manifest_verified': True,
    'test_functions_written': 5,
    'rust_compiled': False,
    'rust_tests_executed': False,
    'filesystem_fixtures_executed': False,
    'native_activation_verified': False,
    'timing_executed': False,
}
print(json.dumps(checks, indent=2))
