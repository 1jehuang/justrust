"""Opt-in release test on a committed client workspace, never the current checkout."""
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import tempfile

# Release CI omits the vendored toolchain and Wrangler state. The client
# workspace also excludes bench, including its large recorded workloads.
SKIP = {b'toolchain', b'.wrangler', b'bench', b'data'}
ARGS = ['test', '--workspace', '--locked']


def materialize(root, sha, destination):
    if not re.fullmatch('[0-9a-f]{40}', sha):
        raise ValueError('exact gate requires a full commit SHA')
    git = ['git', '--no-replace-objects', '-C', str(root)]
    def read(*args):
        return subprocess.check_output([*git, *args])
    if read('cat-file', '-t', sha).strip() != b'commit':
        raise ValueError('expected commit object')
    roots = [os.fsdecode(p) for p in read('ls-tree', '-z', '--name-only', sha).split(b'\0') if p and p not in SKIP]
    entries = read('ls-tree', '-rz', sha, '--', *roots).split(b'\0')
    inventory = []
    # Read blobs, not git archive: export-ignore and export-subst must not
    # drop workspace members or rewrite bytes. Never copy local Git config.
    for row in filter(None, entries):
        header, name = row.split(b'\t', 1)
        mode, kind, oid = header.split()
        path = Path(os.fsdecode(name))
        if (mode not in (b'100644', b'100755') or kind != b'blob' or path.is_absolute()
                or any(p in {'.git', '..', 'target', '.justrust'} for p in path.parts)):
            raise ValueError(f'unsupported client input: {path}')
        data = read('cat-file', 'blob', oid)
        if hashlib.sha1(b'blob %d\0' % len(data) + data).hexdigest().encode() != oid:
            raise ValueError(f'Git blob identity mismatch: {path}')
        target = destination / path
        target.parent.mkdir(exist_ok=True, parents=True)
        with target.open('xb') as f:
            f.write(data)
        target.chmod(int(mode, 8) & 0o777)
        inventory.append([str(path), hashlib.sha256(data).hexdigest(), mode == b'100755'])
    return inventory


# This verifier runs on the Linux build host, not in the launching checkout.
# Ignore only tool-owned root directories and the two added witness files.
VERIFY = '''
import hashlib, json, os, stat, sys
from pathlib import Path
name, identity = sys.argv[1:]
probe = Path('tests') / name
raw = probe.with_suffix('.json').read_bytes()
if hashlib.sha256(raw).hexdigest() != identity:
    sys.exit('wrong source inventory')
expected = {path:(digest, executable) for path,digest,executable in json.loads(raw)}
seen = set()
for root, dirs, files in os.walk('.'):
    if root == '.':
        dirs[:] = [d for d in dirs if d not in ('target', '.git', '.justrust')]
    for d in dirs:
        if (Path(root) / d).is_symlink():
            sys.exit('unexpected symlink: ' + d)
    for filename in files:
        p = Path(root) / filename
        if p in (probe.with_suffix('.rs'), probe.with_suffix('.json')):
            continue
        mode = p.lstat().st_mode
        if str(p) not in expected or not stat.S_ISREG(mode):
            sys.exit('unexpected source input: ' + str(p))
        digest, executable = expected[str(p)]
        if bool(mode & 0o111) != executable or hashlib.sha256(p.read_bytes()).hexdigest() != digest:
            sys.exit('changed source input: ' + str(p))
        seen.add(str(p))
if seen != expected.keys():
    sys.exit('missing source input: ' + str(sorted(expected.keys() - seen)))
'''


def install_witness(directory, sha, inventory):
    # One additive test target. Nothing in the candidate is rewritten. Its
    # unique passing test line in the *same run's* log witnesses the host's
    # source bytes, not just a local pathname or a successful Cargo exit.
    manifest = json.dumps(inventory, ensure_ascii=True, separators=(',', ':')).encode()
    identity = hashlib.sha256(manifest).hexdigest()
    name = f'exact_{sha}_{identity}_{secrets.token_hex(8)}'
    probe = directory / 'tests' / f'{name}.rs'
    probe.parent.mkdir(exist_ok=True)
    with probe.open('x') as f:
        f.write(f'#[test]\nfn {name}() {{\n')
        f.write('assert!(std::process::Command::new("python3").args(["-I", "-c", ')
        f.write(f'r##"{VERIFY}"##, "{name}", "{identity}"]).status().unwrap().success());\n}}\n')
    with probe.with_suffix('.json').open('xb') as f:
        f.write(manifest)
    return name


def gate(root, sha, *, client='justrust', invoke=subprocess.run):
    env = dict(os.environ, JUSTRUST_NO_UPDATE='1', JUSTRUST_REMOTE_REQUIRED='1', JUSTRUST_TEST_REUSE='0')
    with tempfile.TemporaryDirectory(prefix=f'jr-release-{sha}-', dir=os.getenv('JCODE_SCRATCH_DIR')) as tmp:
        cwd = Path(tmp).resolve()
        if cwd.is_relative_to(root.resolve()) or any(
                (p / '.git').exists() or (p / 'Cargo.toml').exists() for p in cwd.parents):
            raise ValueError('scratch must be outside Git and Cargo workspaces')
        name = install_witness(cwd, sha, materialize(root, sha, cwd))
        def command(*args, capture=False):
            return invoke([client, *args], cwd=cwd, env=env, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None).stdout
        command('test', '--remote', *ARGS[1:])
        rows = [json.loads(s) for s in command('runs', '--here', '--json', '-n', '10', capture=True).splitlines()]
        matches = [r for r in rows if r.get('cwd') == str(cwd) and r.get('args') == ARGS]
        if len(matches) != 1:
            raise ValueError('missing or ambiguous exact-workspace run')
        row = matches[0]
        if (row.get('remote') != 'hosted' or row.get('place', 'hosted') != 'hosted' or row.get('exit') != 0
                or not re.fullmatch(r'[A-Za-z0-9_-]+', row.get('id', ''))):
            raise ValueError('expected successful hosted run identity')
        log = command('log', row['id'], '--grep', name, capture=True)
        if f'test {name} ... ok' not in log.splitlines():
            raise ValueError('hosted source witness missing or mismatched')
        print(f'Exact hosted gate passed: source={sha} witness={name} run={row["id"]}', flush=True)
        return sha
