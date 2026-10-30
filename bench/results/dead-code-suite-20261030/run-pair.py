#!/usr/bin/env python3
"""Controlled named-suite diagnostic, not comprehensive default promotion."""
from pathlib import Path
import hashlib
import json
import os
import subprocess

ROOT = Path('/home/jeremy/justrust')
SCRATCH = Path('/home/jeremy/.jcode/scratch/dead-item-cache-20261030')
HOME = SCRATCH / 'bench-home'
PREFIX = HOME / 'toolchains/staged/jr-b7b7efc9982d'
OUT = ROOT / 'bench/results/dead-code-suite-20261030'
OUT.mkdir(exist_ok=False)
manifest = json.loads((SCRATCH / 'suite-toolchain-identity.json').read_text())

def identity():
    result = {}
    for role, item in manifest['artifacts'].items():
        digest = hashlib.sha256((PREFIX / item['relative_path']).read_bytes()).hexdigest()
        if digest != item['sha256']:
            raise RuntimeError('artifact mismatch: ' + role)
        result[role] = digest
    return result

before = identity()
client = SCRATCH.parent / 'query-retention-bench-20261030/justrust-client'
env = {k: v for k, v in os.environ.items() if not k.startswith(('JUSTRUST_', 'BLAZE_', 'RA_JIT_', 'CARGO_PROFILE_')) and k not in ('RUSTC','RUSTDOC','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER','CARGO')}
env.update(JUSTRUST=str(client), JUSTRUST_HOME=str(HOME),
           JUSTRUST_BENCH_WORK=str(SCRATCH / 'suite-work'),
           JUSTRUST_REMOTE='off', JUSTRUST_DEPCACHE='1',
           JUSTRUST_LOCALCACHE='0', JUSTRUST_LOCALCACHE_TARGET_BOUND='0',
           JUSTRUST_PREBUILT='off', JUSTRUST_PREBUILT_ALWAYS='0',
           JUSTRUST_BUILDD='0', JUSTRUST_CACHE_PROC_MACROS='0',
           JUSTRUST_CARGO_FORK='0', JUSTRUST_HOTPATCH='0',
           JUSTRUST_BENCH_RUNTIME_CONTEXT='jcode-isolated-v1',
           CARGO_HOME='/home/jeremy/.cargo', RUSTUP_HOME='/home/jeremy/.rustup',
           JUSTRUST_REAL_CARGO=str(PREFIX / 'bin/cargo'),
           CARGO_PROFILE_DEV_OPT_LEVEL='0', CARGO_PROFILE_DEV_DEBUG='0',
           CARGO_PROFILE_TEST_OPT_LEVEL='0', CARGO_PROFILE_TEST_DEBUG='0')
state = {'scope':'named-justrust-body-test-only-not-comprehensive',
         'artifacts_before':before, 'client_sha256':hashlib.sha256(client.read_bytes()).hexdigest(),
         'policy':'Same pinned source/scenario/profiles/toolchain/runtime isolation. Private depcache enabled equally to preserve pinned build-script fixture contract. Only dead-code item flag changes. Hotpatch disabled.',
         'status':'running','calls':[]}

def save():
    (OUT / 'state.json').write_text(json.dumps(state,indent=2)+'\n')

save()
try:
    for mode in ('no','yes'):
        env['RUSTFLAGS'] = '-Zjr-empty-lint-level-fast-path=yes -Zjr-cache-small-query-values=yes -Zjr-cache-associated-items=yes -Zjr-cache-dead-code-items=' + mode
        command = ['python3','bench/suite.py','run','justrust-body-test','-n','3','--warmup','1','--quiet-wait','0','--toolchain','jr-b7b7efc9982d','-o',str(OUT / (mode+'.json'))]
        with (OUT / (mode+'.stdout')).open('x') as out, (OUT / (mode+'.stderr')).open('x') as err:
            p = subprocess.run(command,cwd=ROOT,env=env,stdout=out,stderr=err)
        state['calls'].append({'mode':mode,'command':command,'status':p.returncode,'flags':env['RUSTFLAGS']})
        save()
        identity()
        if p.returncode:
            raise RuntimeError('named suite failed: '+mode)
    command=['python3','bench/suite.py','compare',str(OUT/'no.json'),str(OUT/'yes.json')]
    with (OUT/'compare.stdout').open('x') as out,(OUT/'compare.stderr').open('x') as err:
        p=subprocess.run(command,cwd=ROOT,env=env,stdout=out,stderr=err)
    if p.returncode:
        raise RuntimeError('comparison rejected')
    state['status']='completed'
except BaseException as exc:
    state['status']='failed'
    state['error']=repr(exc)
    raise
finally:
    state['artifacts_after']=identity()
    state['client_sha256_after']=hashlib.sha256(client.read_bytes()).hexdigest()
    save()
