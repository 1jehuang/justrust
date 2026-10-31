from pathlib import Path
import subprocess, hashlib, re
repo = Path('<repo>')
root = Path(__file__).parent
base = root / 'originals'
files = list(base.rglob('*.rs'))
def section(text, start, end):
    return text[text.index(start):text.index(end, text.index(start))]
for original in files:
    rel = original.relative_to(base)
    assert original.read_bytes() == subprocess.check_output(['git','show',f'c8829bae02fd234f46012ce6996baa14c09b3a1c:{rel}'], cwd=repo), rel
print('PASS six complete originals match accepted c8829bae0')
lib = Path('toolchain/resident/crates/blaze-metadata/src/lib.rs')
old, new = (base/lib).read_text(), (repo/lib).read_text()
assert section(old, 'pub fn programs(', '\n/// The unit graph') == section(new, 'pub fn programs(', '\n/// The native planning')
assert section(old, 'fn described(', '\n/// The metadata of a workspace') == section(new, 'fn described(', '\n/// The metadata of a workspace')
summ = Path('toolchain/resident/crates/blaze-metadata/src/summary.rs')
o, n = (base/summ).read_text(), (repo/summ).read_text()
assert section(o, 'pub(crate) fn read(', '\n/// `path`') == section(n, 'pub(crate) fn read(', '\n/// Native v83')
print('PASS original programs, described and summary::read byte-identical')
for rel, marker, end in [
 ('toolchain/resident/crates/blaze-manifest/src/metadata.rs','#[derive(Serialize)]\npub struct SerializedPackage', '\n/// `[hints]`'),
 ('toolchain/resident/crates/blaze-manifest/src/dependency.rs','pub struct SerializedDependency', '\n}'),
 ('toolchain/resident/crates/blaze-manifest/src/targets.rs','pub struct SerializedTarget', '\n}')]:
    assert section((base/rel).read_text(), marker, end) == section((repo/rel).read_text(), marker, end)
print('PASS serialized package/dependency/target structs unchanged')
native = section(new, 'fn programs_native_typed_inner(', '\n/// The unit graph')
assert 'serde_json' not in native and 'described(' not in native
assert 'resolving_with_mode(asked, None, None, true' in native
assert new.count('summary::read_typed(') == 2
assert 'resolving_with_mode(asked, pick, relock, false, then)' in new
assert 'OnceLock<bool>' in new and 'v == "1"' in new
assert n.count('ra-jit: native-typed-planning-v83 entered') == 1
assert 'let _ = writeln!' in n and 'static ENTERED: std::sync::Once' in n
print('PASS typed final JSON erasure, both summary conversion sites, default-OFF gate, fixed ignored-error marker')
for rel in subprocess.check_output(['git','ls-files','toolchain/resident/crates/blaze-manifest/tests','toolchain/resident/crates/blaze-metadata/tests'],cwd=repo,text=True).splitlines():
    assert (repo/rel).read_bytes() == subprocess.check_output(['git','show',f'HEAD:{rel}'],cwd=repo),rel
print('PASS all tracked existing fixture/test files unchanged')
owned = [str(p.relative_to(base)) for p in files] + ['toolchain/resident/crates/blaze-metadata/tests/native_typed_planning/mod.rs','docs/resident-native-typed-planning.md']
subprocess.run(['git','diff','--check','--',*owned],cwd=repo,check=True)
print('PASS owned-file whitespace checks; unrelated FINDINGS whitespace excluded')
print('SOURCE ONLY: no compilation, tests, probes or benchmarks executed')
