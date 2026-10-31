#!/usr/bin/env python3
import hashlib,json,shutil
from pathlib import Path
R=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
S=Path('/home/jeremy/justrust/toolchain/resident/vendor/rust-analyzer')
U=R/'candidate-unit-v48-snapshot'
D=U/'vendor/rust-analyzer'
def hashes(p):
    return {str(x.relative_to(p)):hashlib.sha256(x.read_bytes()).hexdigest() for x in sorted(p.rglob('*')) if x.is_file()}
def main():
    assert not D.exists()
    original=hashes(S)
    shutil.copytree(S,D,symlinks=True)
    copied=hashes(D)
    assert copied==original
    for name in ('rustc_abi','rustc_type_ir','rustc_next_trait_solver','salsa','salsa-macros','salsa-macro-rules'):
        target=S.parent/name
        assert target.is_dir(),target
        (D.parent/name).symlink_to(target,target_is_directory=True)
    lock=R/'candidate-unit-v24-snapshot/vendor/rust-analyzer/Cargo.lock'
    assert lock.is_file()
    shutil.copy2(lock,D/'Cargo.lock')
    selected=hashes(D)
    assert [k for k in original if original[k]!=selected[k]]==['Cargo.lock']
    (U/'source-copy-before.json').write_text(json.dumps({'original':original,'copy':copied,'selected':selected,'private_lock_origin':str(lock)},indent=2)+'\n')
    assert hashes(S)==original
    print(json.dumps({'files':len(original),'only_private_delta':'Cargo.lock','output':str(U)}))
if __name__=='__main__':main()
