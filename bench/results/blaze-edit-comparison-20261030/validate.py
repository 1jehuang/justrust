#!/usr/bin/env python3
"""Audit retained observations, not performance significance or full Rust correctness."""
from pathlib import Path
import hashlib,json,re,statistics
ROOT=Path(__file__).resolve().parent
REPO=ROOT.parents[2]
def load(path): return json.loads(path.read_text())
def main():
    run=ROOT/'complete'
    meta=load(run/'metadata.json')
    assert meta['identities_before']==meta['identities_after']
    assert meta['identities_unchanged'] and meta['source_restored'] and meta['failure'] is None
    rows=[json.loads(line) for line in (run/'measurements.jsonl').read_text().splitlines()]
    assert len(rows)==9
    for replicate in range(3):
        group=[r for r in rows if r['replicate']==replicate]
        assert len(group)==3 and len({r['source_sha256'] for r in group})==1
    medians={}
    ids=[]
    for stack in ('owned-off','owned-on','blaze'):
        prep=load(run/f'prepare-000-{stack}.json')
        error=load(run/f'type-error-000-{stack}.json')
        recovery=load(run/f'recovery-000-{stack}.json')
        assert prep['status']==recovery['status']==0
        assert error['status']==101 and error['gate_passed'] and not error['timed_out']
        samples=[r for r in rows if r['stack']==stack]
        assert len(samples)==3 and all(r['status']==0 and not r['timed_out'] and not r['error'] for r in samples)
        medians[stack]=statistics.median(r['elapsed_seconds'] for r in samples)
        for r in samples:
            assert r['noise']['contamination']=='unknown'
            stderr=(run/f"edit-{r['replicate']:03d}-{stack}.stderr").read_text()
            if stack=='blaze':
                assert '[blaze]' in stderr and 'RUSTFLAGS' not in r['environment']
            else:
                policy=r['environment']; assert policy['JUSTRUST_REMOTE']=='off' and policy['JUSTRUST_LOCALCACHE']=='0'
                assert all(f'-Z{flag}={"yes" if stack=="owned-on" else "no"}' in policy['RUSTFLAGS'] for flag in ('jr-empty-lint-level-fast-path','jr-cache-small-query-values','jr-cache-associated-items'))
        if stack!='blaze':
            exported={r['id']:r for p in (REPO/'data/runs'/f'xps14-blaze-{stack}-20261030').glob('*.jsonl') for r in map(json.loads,p.read_text().splitlines())}
            assert len(exported)==6
            for phase,n in [('prepare',0),('type-error',0),('recovery',0),('edit',0),('edit',1),('edit',2)]:
                text=(run/f'{phase}-{n:03d}-{stack}.stderr').read_text()
                found=re.findall(r'justrust log (\d{8}-\d+-\d+)',text)
                assert len(found)==1 and found[0] in exported
                assert exported[found[0]]['exit']==(101 if phase=='type-error' else 0)
                ids.append(found[0])
    assert medians==load(run/'summary.json')['medians_seconds']
    manifest=load(ROOT/'evidence-sha256.json')
    for name,sha in manifest.items(): assert hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==sha,name
    report={'verdict':'passed','matched_edits':9,'expected_error_and_recovery_stacks':3,'exported_owned_calls':len(ids),'run_ids':ids,'medians_seconds':medians,'identities_unchanged':True,'contamination':'unknown','scope':'retained focused check-only observation audit, not a general speed or default-promotion gate'}
    (ROOT/'validation.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))
if __name__=='__main__':main()
