import json,os,glob,statistics as st,sys
R=os.path.expanduser('~/.justrust/runs')
rows=[]
for d in os.listdir(R):
    p=os.path.join(R,d)
    try:
        s=json.load(open(p+'/summary.json')); m=json.load(open(p+'/meta.json'))
    except Exception: continue
    if not m.get('agent_session'): continue
    if s.get('exit')==0: continue
    try: lines=[json.loads(l) for l in open(p+'/output.jsonl')]
    except Exception: continue
    if not any('could not compile' in l['l'] for l in lines): continue
    start=s['start']; end=start+s['wall']
    first_err=next((l['t'] for l in lines if l['l'].startswith('error[') or l['l'].startswith('error:') and 'could not compile' not in l['l']),None)
    cnc=next((l['t'] for l in lines if l['l'].startswith('error: could not compile')),None)
    if cnc is None: continue
    after=[l for l in lines if l['t']>cnc]
    later_err=sum(1 for l in after if l['l'].startswith('error[') or (l['l'].startswith('error:') and 'could not compile' not in l['l']))
    later_cnc=sum(1 for l in after if l['l'].startswith('error: could not compile'))
    rows.append(dict(id=d,wall=s['wall'],cnc=cnc-start,first=(first_err or cnc)-start,tail=end-cnc,later_err=later_err,later_cnc=later_cnc,sub=m['subcommand'],kg=any('keep-going' in a for a in m['args'])))
n=len(rows); print('runs',n)
print('wall med',st.median(r['wall'] for r in rows),'cnc med',st.median(r['cnc'] for r in rows),'first med',st.median(r['first'] for r in rows))
tot=sum(r['wall'] for r in rows); tail=sum(r['tail'] for r in rows)
print('total h',tot/3600,'tail after could-not-compile h',tail/3600, 'frac',tail/tot)
print('tail med',st.median(r['tail'] for r in rows),'p90',sorted(r['tail'] for r in rows)[int(.9*n)])
print('runs with later errors',sum(1 for r in rows if r['later_err']),'later cnc',sum(1 for r in rows if r['later_cnc']))
print('tail>1s',sum(1 for r in rows if r['tail']>1))
json.dump(rows,open(sys.argv[1],'w'))
