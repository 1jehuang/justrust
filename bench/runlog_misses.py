import json,os,collections,bisect
RUNS=os.path.expanduser('~/.justrust/runs')
gcs=sorted(json.loads(l)['t'] for l in open(os.path.expanduser('~/.justrust/cache/depcache/gc.jsonl')))
GC0=gcs[0]
R=json.load(open('runlog.json'))
seen={}; seen_any={}
cls=collections.Counter(); w=collections.Counter()
for r in sorted(R,key=lambda r:r['start'] or 0):
    p=f"{RUNS}/{r['id']}/units.jsonl"
    if not os.path.exists(p) or r['id']=='20261011-083315454-3785807': continue
    try: m=json.load(open(f"{RUNS}/{r['id']}/meta.json"))
    except: m={}
    rc=m.get('real_rustc') or '?'
    custom_dir='JUSTRUST_DEPCACHE_DIR' in (m.get('env') or {})
    us=[]
    for l in open(p):
        try: us.append(json.loads(l))
        except: pass
    missed=set(u['package'] for u in us if u.get('depcache')=='miss')
    for u in us:
        if u.get('local'): continue
        st=u.get('depcache') or 'none'; t=u.get('end') or 0
        k=(rc,u['package'],u['version'],u.get('build_script'),u.get('opt_level'),u.get('emit'),u.get('test'),u['manifest_dir'])
        k2=(u['package'],u['version'],u.get('build_script'),u.get('opt_level'),u.get('emit'),u['manifest_dir'])
        if st=='miss' and not r['bench'] and r['proj']!='jr-peer-e2e' and not custom_dir:
            if k in seen:
                c='evicted (gc ran since last seen)' if bisect.bisect(gcs,t)>bisect.bisect(gcs,seen[k]) else 'key changed, same toolchain (features/flags/upstream)'
            elif k2 in seen_any: c='new toolchain for a crate seen before'
            else: c='never built before (new version/git rev/profile)'
            if c.startswith('evicted') and seen[k]<GC0: c='key changed, same toolchain (features/flags/upstream)'  # unknown gc before log
            cls[c]+=1; w[c]+=u.get('wall') or 0
        if st in('miss','hit'):
            seen[k]=t; seen_any[k2]=t
tot=sum(w.values())
for c,v in w.most_common(): print(f"{v/3600:5.2f} h {100*v/tot:4.0f}% {cls[c]:6} units  {c}")
