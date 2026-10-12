"""Error codes and wall times of agent compile-error runs in ~/.justrust/runs.
Usage: mine_codes.py out.json. Same run selection as replay_runs.py."""
import json, os, sys, collections, re
R = os.path.expanduser('~/.justrust/runs')
rows = []
for d in os.listdir(R):
    p = os.path.join(R, d)
    try:
        s = json.load(open(p + '/summary.json')); m = json.load(open(p + '/meta.json'))
    except Exception:
        continue
    if not m.get('agent_session') or s.get('exit') == 0:
        continue
    try:
        lines = [json.loads(l) for l in open(p + '/output.jsonl')]
    except Exception:
        continue
    if not any('could not compile' in l['l'] for l in lines):
        continue
    errs = []
    code = None
    for l in lines:
        t = l['l'].lstrip()
        if t.startswith('error['):
            code = t[6:].split(']')[0]; msg = t
        elif t.startswith('error:') and 'could not compile' not in t and 'aborting' not in t:
            code = ''; msg = t
        elif t.startswith('--> ') and code is not None:
            errs.append(dict(code=code, t=l['t'] - s['start'], loc=t[4:], msg=msg[:160])); code = None
    if not errs:
        continue
    rows.append(dict(id=d, cwd=m['cwd'], sub=m['subcommand'], wall=s['wall'], errs=errs,
                     early=s.get('early')))
c = collections.Counter(e['code'] or 'syntax/other' for r in rows for e in r['errs'][:1])
print('runs', len(rows), 'with early info', sum(1 for r in rows if r['early']))
print('first error code:', c.most_common(30))
proj = collections.Counter(('desktop' if 'desktop' in r['cwd'] else 'jcode' if 'jcode' in r['cwd'] else 'other') for r in rows)
print('projects', proj)
json.dump(rows, open(sys.argv[1], 'w'))
