from pathlib import Path
import hashlib
import re
import subprocess

root = Path('<EXPERIMENT>/source-identity-v86')
repo = Path('<PRIVATE_HOME>/justrust')
relative = 'toolchain/resident/crates/blaze-metadata/src/lib.rs'
original = (root / 'original-lib.rs').read_text()
accepted = (root / 'accepted-HEAD-lib.rs').read_text()
candidate = (repo / relative).read_text()

def function(text, name, indent=''):
    start = text.index(indent + 'fn ' + name + '(')
    end = text.index('\n' + indent + '}', start) + len(indent) + 2
    return text[start:end]

def body(text, name, indent=''):
    f = function(text, name, indent)
    return f[f.index('{'):]

def tokens(s):
    return re.sub(r'\s+', '', s)

assert original == accepted, 'initial source differed from accepted HEAD'
assert subprocess.check_output(['git', 'show', 'HEAD:' + relative], cwd=repo).decode() == accepted, 'accepted file changed in HEAD'
assert body(original, 'fetch') == body(candidate, 'fetch_original'), 'OFF full original body drift'
assert function(original, 'manifest_source') == function(candidate, 'manifest_source'), 'original source resolver drift'
assert tokens(body(original, 'manifest_source')) == tokens(body(candidate, 'original_manifest_source', '    ')), 'independent full source oracle drift'
expected = body(original, 'fetch').replace('    for (source_text, (dir, _))', '    let mut reuse = SourceIdentityReuse::default();\n    for (source_text, (dir, _))').replace('manifest_source(&source_text, &ctx.config)?', 'reuse.resolve(&source_text, &ctx.config)?')
assert expected == body(candidate, 'fetch_source_identity_reuse'), 'candidate changes beyond resolution substitution'
wrapper = function(candidate, 'fetch')
assert 'if !source_identity_reuse_enabled()' in wrapper and 'return fetch_original(ids, checksums, ctx, sources);' in wrapper
resolve = function(candidate, 'resolve', '    ')
assert resolve.index('config.check_registry_index_not_set()?') < resolve.index('source_identity_reuse_marker()')
assert resolve.index('let resolved = manifest_source(source, config)?;') < resolve.index('self.successful.insert(')
assert candidate.count('ra-jit: source-identity-reuse-v86 batch') == 1
assert 'static ENABLED: std::sync::OnceLock<bool>' in candidate
assert 'v == "1"' in candidate
print('PASS: accepted HEAD equals initial full source')
print('PASS: OFF fetch body bytewise original, manifest_source bytewise original')
print('PASS: independent full manifest_source oracle tokenwise original')
print('PASS: ON fetch differs only by local map and resolution call')
print('PASS: hit validation precedes marker, successful resolution precedes insertion')
print('PASS: exact first-use flag and fixed marker present')
print('UNEXECUTED: Rust type checks, fixtures, native/off-bytewise/marker/performance acceptance')
print('candidate SHA256:', hashlib.sha256(candidate.encode()).hexdigest())
