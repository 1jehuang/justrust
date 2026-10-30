import json, statistics
from pathlib import Path
root = Path('/home/jeremy/.jcode/scratch/resident-root-snapshot-profile-v17-20261030')
selected = ('worker roots', 'worker apply', 'worker check', 'client planning', 'client command')
rows=[]
for stack in ('old','new'):
    for rep in range(5):
        path=root/f'{stack}-{rep}.trace.json'
        data=json.loads(path.read_text())
        intervals={name:[{'thread':e.get('tid'),'start_us':e['ts'],'duration_us':e['dur']} for e in data['traceEvents'] if e.get('ph')=='X' and e.get('name')==name] for name in selected}
        rows.append({'stack':stack,'replicate':rep,'intervals':intervals})
medians={}
for stack in ('old','new'):
    medians[stack]={}
    for name in selected:
        samples=[row['intervals'][name][0]['duration_us'] for row in rows if row['stack']==stack and len(row['intervals'][name])==1]
        medians[stack][name]={'single_interval_samples':len(samples),'median_elapsed_us':statistics.median(samples) if len(samples)==5 else None}
report={'scope':'nested elapsed intervals, not CPU work, time saved, critical path or total speedup','rows':rows,'medians':medians}
(root/'phase-summary.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(medians,indent=2))
