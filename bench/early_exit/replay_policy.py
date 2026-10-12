import json,os,statistics as st
R=os.path.expanduser('~/.justrust/runs')
rows=json.load(open(os.environ['S']+'/replay.json'))
idx={}
for d in os.listdir(R):
    try: m=json.load(open(f'{R}/{d}/meta.json'))
    except: continue
    if m.get('agent_session'): idx.setdefault((m['agent_session'],m['cwd']),[]).append(m['start'])
for v in idx.values(): v.sort()
res={'n':len(rows)}
pol={'kill':0,'kill_sunk<1s':0,'kill_sunk<3s':0,'finish_bg':0}
harm={k:0 for k in pol}
tails=0
for r in rows:
    p=f"{R}/{r['id']}"; m=json.load(open(p+'/meta.json')); s=json.load(open(p+'/summary.json'))
    t0=s['start']; trig=t0+r['cnc']; end=t0+s['wall']; tail=r['tail']; tails+=tail
    P=json.load(open(p+'/processes.json'))
    sunk=max([trig-q['first_seen'] for q in P if q['kind']=='rustc' and '/shim/' not in q['cmdline'] and q['first_seen']<trig-0.05 and q['last_seen']>trig+0.2]+[0])
    nxt=[t for t in idx.get((m.get('agent_session'),m['cwd']),[]) if t>t0+1]
    gap=max(0.0,(nxt[0]-end)) if nxt else 1e9
    rem=tail
    for k,lim in (('kill',1e9),('kill_sunk<1s',1),('kill_sunk<3s',3)):
        if sunk<lim:
            pol[k]+=tail-sunk
            if sunk>tail: harm[k]+=1
    pol['finish_bg']+=min(tail,gap)
    pol.setdefault('return_then_kill_on_next',0); pol['return_then_kill_on_next']+=tail-(min(sunk,max(0,tail-gap)) if gap<tail else 0)
res['tail_s']=tails; res['saved_s']=pol; res['runs_made_worse']=harm
print(json.dumps(res,indent=1))
