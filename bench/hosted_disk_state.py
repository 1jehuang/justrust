#!/usr/bin/env python3
"""Hosted disk-backed incremental reuse experiment on a pinned justrust archive.

Existing rustc incremental state only, not a full compiler heap checkpoint.
Uses normal authenticated justrust hosted commands. Mutates only private scratch.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import statistics
import subprocess
import time


def remote_python(program, *args):
    command = 'python3 -c ' + shlex.quote(program) + ' ' + ' '.join(shlex.quote(str(a)) for a in args)
    result = subprocess.run(['justrust', 'remote', 'ssh', command], text=True, capture_output=True, timeout=180)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    marker = result.stdout.find('DISK_STATE_JSON=')
    if marker < 0:
        raise RuntimeError('missing remote result: ' + result.stdout + result.stderr)
    return json.loads(result.stdout[marker:].splitlines()[0].split('=', 1)[1])


REMOTE = r'''
import concurrent.futures, hashlib, json, os, pathlib, shutil, sys, time
root=pathlib.Path(sys.argv[1]); action=sys.argv[2]; workers=int(sys.argv[3])
if not root.name.startswith('disk-state-') or root == root.parent:
    raise SystemExit('unsafe experiment root')
target=root/'source'/'target'/'debug'/'incremental'; snapshot=root/'state-snapshot'
def manifest(path):
    return {str(p.relative_to(path)):hashlib.file_digest(p.open('rb'),'sha256').hexdigest() for p in path.rglob('*') if p.is_file()}
def copy(src,dst):
    files=[p for p in src.rglob('*') if p.is_file()]
    for p in src.rglob('*'):
        if p.is_dir(): (dst/p.relative_to(src)).mkdir(parents=True,exist_ok=True)
    dst.mkdir(parents=True,exist_ok=True)
    def one(p): shutil.copy2(p,dst/p.relative_to(src))
    start=time.perf_counter()
    if workers == 1:
        for p in files: one(p)
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool: list(pool.map(one,files))
    return time.perf_counter()-start, len(files), sum(p.stat().st_size for p in files)
if action=='snapshot':
    if not target.is_dir() or not any(p.is_file() for p in target.rglob('*')): raise SystemExit('incremental state missing or empty')
    if snapshot.exists(): raise SystemExit('snapshot already exists')
    elapsed,files,size=copy(target,snapshot)
    expected=manifest(target)
    if expected != manifest(snapshot): raise SystemExit('snapshot hash mismatch')
    (root/'state-manifest.json').write_text(json.dumps(expected,sort_keys=True))
    out=dict(seconds=elapsed,files=files,bytes=size)
elif action in ('fresh','restore'):
    if action=='restore' and not (snapshot.is_dir() and (root/'state-manifest.json').is_file()): raise SystemExit('valid snapshot required')
    if target.exists(): shutil.rmtree(target)
    if action=='restore':
        elapsed,files,size=copy(snapshot,target)
        if manifest(target) != json.loads((root/'state-manifest.json').read_text()): raise SystemExit('restore hash mismatch')
        out=dict(seconds=elapsed,files=files,bytes=size)
    else: out=dict(seconds=0,files=0,bytes=0)
elif action=='summary':
    path=pathlib.Path.home()/'.justrust'/'runs'/sys.argv[4]/'summary.json'
    out=json.loads(path.read_text())
else: raise SystemExit('bad action')
out['loadavg']=os.getloadavg(); out['cpu_count']=os.cpu_count()
print('DISK_STATE_JSON='+json.dumps(out))
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--command', choices=('check', 'build'), default='check')
    args = parser.parse_args()
    if args.rounds < 3:
        parser.error('at least three rounds are required')
    root = args.root.resolve()
    if not root.name.startswith('disk-state-') or root.exists():
        parser.error('root must be a new disk-state-* scratch directory')
    source = root / 'source'
    source.mkdir(parents=True)
    repo = Path(__file__).resolve().parents[1]
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip()
    archive = subprocess.run(['git', 'archive', revision, 'Cargo.toml', 'Cargo.lock', 'src', 'docs/AGENTS-snippet.md', 'scripts/sign-release.py'], cwd=repo, capture_output=True, check=True).stdout
    subprocess.run(['tar', '-x', '-C', str(source)], input=archive, check=True)
    (source / '.gitignore').write_text('/target/\n')
    edit = source / 'src/runs.rs'
    original = edit.read_text()
    anchor = 'pub fn load_index() -> Vec<IndexEntry> {\n'
    if original.count(anchor) != 1:
        raise RuntimeError('edit anchor missing or ambiguous')
    env = dict(os.environ)
    env.update(JUSTRUST_LOCALCACHE='0', JUSTRUST_LOCALCACHE_TARGET_BOUND='0',
               JUSTRUST_DEPCACHE='0', JUSTRUST_PEER_CACHE='0', JUSTRUST_PREBUILT='off',
               JUSTRUST_CACHE_PROC_MACROS='0', JUSTRUST_BUILDD='0', JUSTRUST_CARGO_FORK='0',
               JUSTRUST_CUTOFF='0', JUSTRUST_CARGO_CUTOFF='0', JUSTRUST_PASSES='on',
               CARGO_INCREMENTAL='1', JUSTRUST_SLOTS='0')
    command = ['justrust', args.command, '--remote', '--lib', '--locked']
    if args.command == 'check':
        command += ['--target-dir', 'target']
    report = dict(schema=1, source_revision=revision, command=command,
                  scope='Existing disk-backed rustc incremental cache, not full-state persistence or parallel query decoding',
                  cache_condition='OS page cache uncontrolled/warm, no machine-wide cache dropping', rows=[])
    def save():
        (root/'results.json').write_text(json.dumps(report,indent=2)+'\n')
    def compile_row(name):
        start=time.perf_counter()
        result=subprocess.run(command,cwd=source,env=env,text=True,capture_output=True,timeout=900)
        elapsed=time.perf_counter()-start
        (root/(name+'.stdout')).write_text(result.stdout)
        (root/(name+'.stderr')).write_text(result.stderr)
        ids=re.findall(r'justrust (?:show|log) ([0-9]{8}-[0-9]+-[A-Za-z0-9_-]+)',result.stdout+result.stderr)
        row=dict(name=name,status=result.returncode,client_wall=elapsed,run_id=ids[-1] if ids else None)
        if row['run_id']:
            local=Path.home()/'.justrust/runs'/row['run_id']/'summary.json'
            if local.exists(): row['summary']=json.loads(local.read_text())
            else: row['summary']=remote_python(REMOTE,root,'summary',1,row['run_id'])
        report['rows'].append(row); save()
        print('JCODE_PROGRESS '+json.dumps({'row':name,'status':result.returncode,'client_wall':elapsed}),flush=True)
        if result.returncode: raise RuntimeError(f'{name} failed, retained logs at {root}')
        return row
    compile_row('prepare')
    report['snapshot']=remote_python(REMOTE,root,'snapshot',1); save()
    modes=[('fresh',1),('restore',1),('restore',4),('restore',16)]
    for n in range(args.rounds):
        order=modes[n%len(modes):]+modes[:n%len(modes)]
        for mode,workers in order:
            restore=remote_python(REMOTE,root,mode,workers)
            marker = 1000 + n * len(modes) + modes.index((mode, workers))
            edit.write_text(original.replace(anchor,anchor+f'    std::hint::black_box({marker}u64);\n'))
            source_sha256 = hashlib.sha256(edit.read_bytes()).hexdigest()
            row=compile_row(f'round-{n}-{mode}-{workers}')
            if row.get('summary', {}).get('units', {}).get('local') != 1:
                raise RuntimeError('expected exactly one local compiler invocation, not a no-op or unrelated build')
            row.update(mode=mode,workers=workers,restore=restore,source_sha256=source_sha256,edit_marker=marker)
            row['restore_plus_client_wall']=restore['seconds']+row['client_wall']
            save()
    report['groups']={}
    for mode,workers in modes:
        rows=[r for r in report['rows'] if r.get('mode')==mode and r.get('workers')==workers]
        report['groups'][f'{mode}-{workers}']=dict(samples=len(rows),client_wall_median=statistics.median(r['client_wall'] for r in rows),restore_median=statistics.median(r['restore']['seconds'] for r in rows),restore_plus_client_wall_median=statistics.median(r['restore_plus_client_wall'] for r in rows))
    save()
    print(json.dumps(report['groups'],indent=2))


if __name__ == '__main__':
    main()
