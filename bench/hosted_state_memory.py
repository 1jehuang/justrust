#!/usr/bin/env python3
"""Read hosted compiler-state bytes into RAM and isolate parallel zstd codec cost.

No restored output files are written. Cold means best-effort eviction of only
this experiment's immutable snapshot, never global caches. Compressed decoding
uses RAM-resident frames, not compressed SSD reads or semantic query decoding.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess

REMOTE = r'''
import concurrent.futures, hashlib, json, os, pathlib, statistics, sys, time
source=pathlib.Path(SOURCE)
if source.name!='state-snapshot' or not source.parent.name.startswith('disk-state-'):
    raise SystemExit('expected private immutable experiment snapshot')
if not source.is_dir(): raise SystemExit('snapshot missing')
if any(p.is_symlink() for p in source.rglob('*')): raise SystemExit('symlink snapshot rejected')
files=sorted(p for p in source.rglob('*') if p.is_file())
if not any(p.stat().st_size for p in files): raise SystemExit('snapshot empty')
chunk_bytes=4*1024*1024
chunks=[(str(p),offset,min(chunk_bytes,p.stat().st_size-offset)) for p in files for offset in range(0,p.stat().st_size,chunk_bytes)]
def read_one(chunk):
    path,offset,size=chunk
    fd=os.open(path,os.O_RDONLY)
    try:
        data=os.pread(fd,size,offset)
        if len(data)!=size: raise ValueError('short read')
        return data
    finally: os.close(fd)
def mapped(function, values, workers):
    if workers==1: return [function(v) for v in values]
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        return list(pool.map(function,values))
def storage_reads():
    for line in pathlib.Path('/proc/self/io').read_text().splitlines():
        if line.startswith('read_bytes:'): return int(line.split(':')[1])
def hashes(values): return [hashlib.sha256(v).hexdigest() for v in values]
raw=mapped(read_one,chunks,1)
expected=hashes(raw)
report={'schema':1,'source':str(source),'files':len(files),'bytes':sum(map(len,raw)),'chunks':len(chunks),'chunk_bytes':chunk_bytes,'cpu_count':os.cpu_count(),'loadavg':os.getloadavg(),'scope':'Raw bytes loaded into RAM, not compiler semantic deserialization; compressed frames are RAM-resident','rows':[]}
for cold in (False,True):
    for repetition in range(3):
        for workers in ([1,4,16] if repetition%2==0 else [16,4,1]):
            if cold:
                for p in files:
                    with p.open('rb') as stream: os.posix_fadvise(stream.fileno(),0,0,os.POSIX_FADV_DONTNEED)
            before=storage_reads(); start=time.perf_counter()
            values=mapped(read_one,chunks,workers)
            elapsed=time.perf_counter()-start; after=storage_reads()
            if hashes(values)!=expected: raise ValueError('raw restored bytes differ')
            report['rows'].append({'mode':'raw-guest-cold' if cold else 'raw-warm','workers':workers,'repetition':repetition,'seconds':elapsed,'read_bytes_delta':after-before,'verified':True})
            del values
codec=sys.modules['compressed_state_restore'].Zstd()
start=time.perf_counter(); frames=[codec.compress(v,3) for v in raw]
report['compression']={'level':3,'version':codec.version,'bytes':sum(map(len,frames)),'seconds':time.perf_counter()-start,'ratio':sum(map(len,frames))/sum(map(len,raw))}
del raw
for repetition in range(3):
    for workers in ([1,4,16] if repetition%2==0 else [16,4,1]):
        def decode(i): return codec.decompress(frames[i],chunks[i][2])
        start=time.perf_counter(); values=mapped(decode,range(len(frames)),workers); elapsed=time.perf_counter()-start
        if hashes(values)!=expected: raise ValueError('decoded bytes differ')
        report['rows'].append({'mode':'zstd-RAM-frames','workers':workers,'repetition':repetition,'seconds':elapsed,'verified':True})
        del values
report['summaries']=[{'mode':mode,'workers':workers,'median_seconds':statistics.median(r['seconds'] for r in report['rows'] if r['mode']==mode and r['workers']==workers),'median_read_bytes':statistics.median(r.get('read_bytes_delta',0) for r in report['rows'] if r['mode']==mode and r['workers']==workers)} for mode in ('raw-warm','raw-guest-cold','zstd-RAM-frames') for workers in (1,4,16)]
report['cache_note']='Guest-cold uses per-private-snapshot advisory eviction without fsync; read_bytes confirms guest storage activity where present, not physical-device coldness. No output writes. Codec frames are in RAM. Hash verification outside timing. Host load uncontrolled.'
print('DISK_STATE_JSON='+json.dumps(report))
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent
    helpers = {name: (directory / (name + '.py')).read_text()
               for name in ('disk_state_restore', 'compressed_state_restore')}
    program = 'import types,sys\n'
    for name, code in helpers.items():
        program += f'm=types.ModuleType({name!r});sys.modules[{name!r}]=m;exec({code!r},m.__dict__)\n'
    program += f'SOURCE={args.source!r}\n' + REMOTE
    result = subprocess.run(['justrust', 'remote', 'ssh', 'python3 -c ' + shlex.quote(program)],
                            text=True, capture_output=True, timeout=300)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    lines = [line.split('=', 1)[1] for line in result.stdout.splitlines()
             if line.startswith('DISK_STATE_JSON=')]
    if len(lines) != 1:
        raise RuntimeError('expected one result')
    report = json.loads(lines[0])
    report['helper_sha256'] = {name: hashlib.sha256(code.encode()).hexdigest() for name, code in helpers.items()}
    args.output.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({'compression': report['compression'], 'summaries': report['summaries']},indent=2))


if __name__ == '__main__':
    main()
