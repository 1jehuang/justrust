#!/usr/bin/env python3
"""Actual vs maximum safe reuse per build, from production per-operation logs.

Weights are compiler CPU seconds (measured where logged, estimated where not;
every estimate is tagged). Categories per build:
  actual    reuse that happened (CPU work not redone)
  missed    proven missed safe reuse (work redone whose result was provably
            reusable)
  necessary recomputation that had to happen
  unknown   work whose reusability the logs cannot decide
maximum = actual + missed (lower bound); actual + missed + unknown (upper).
captured = 100 * actual / maximum, reported as a range when unknown > 0.
Aggregates sum totals across builds before dividing.
"""
import collections, glob, json, os, sys, datetime

RUNS = os.path.expanduser('~/.justrust/runs')
since = datetime.datetime.fromisoformat(sys.argv[1]).timestamp() if len(sys.argv) > 1 else 0

# Dependency-cache miss reasons whose only differing part is provably output-
# irrelevant or where the artifact existed and was lost.
def dep_miss_class(why):
    if not why:
        return 'unknown', 'miss without recorded reason'
    head = why.split(':')[0]
    if head == 'evicted':
        return 'missed', 'evicted (artifact existed)'
    if head.startswith('first-seen'):
        return 'necessary', 'first-seen key'
    if head == 'compile-failed':
        return 'necessary', 'compile failed'
    detail = why.split(': ', 1)[1] if ': ' in why else ''
    parts = set(head.split('+'))
    if parts == {'env'} and 'CARGO_RESOLVER_' in detail and detail.count(',') == 0:
        return 'missed', 'key split only by CARGO_RESOLVER_* (output-irrelevant)'
    if parts == {'flags'} and detail and all('linker=' in f for f in detail.split(', ')):
        return 'missed', 'key split only by -C linker on rlib (output-irrelevant)'
    if 'toolchain' in parts:
        return 'necessary', 'toolchain differs'
    if parts & {'flags', 'dependency', 'native-lib'}:
        return 'necessary', 'flags/dependency/native-lib differ'
    return 'unknown', 'key part ' + head

# Pass 1: per-query mean exclusive compute cost from executions in production
# (new/changed/same/green_recompute), to estimate compute avoided by loads.
qcost = collections.defaultdict(lambda: [0.0, 0])
crate_cpu = {}  # (crate, version, opt) -> last measured miss CPU
files = []
for up in glob.glob(f'{RUNS}/*/units.jsonl'):
    if os.path.getmtime(up) < since:
        continue
    files.append(up)
    for l in open(up):
        try:
            u = json.loads(l)
        except Exception:
            continue
        for x in u.get('reuse') or []:
            if x['outcome'] in ('new', 'changed', 'same', 'green_recompute') and x['count']:
                c = qcost[x['query']]; c[0] += x['secs']; c[1] += x['count']
        if u.get('depcache') == 'miss' and u.get('exit', 0) == 0:
            crate_cpu[(u['crate_name'], u.get('version'), u.get('opt_level'))] = (u.get('user') or 0) + (u.get('sys') or 0)

def mean(q):
    s, n = qcost.get(q, (0, 0))
    return s / n if n else None

builds = []
for up in files:
    rid = up.split('/')[-2]
    try:
        s = json.load(open(up.replace('units.jsonl', 'summary.json')))
    except Exception:
        s = {}
    b = collections.Counter(); why = collections.Counter(); ev = collections.Counter()
    for l in open(up):
        try:
            u = json.loads(l)
        except Exception:
            continue
        cpu = (u.get('user') or 0) + (u.get('sys') or 0)
        dc = u.get('depcache')
        if u.get('build_script') and not u.get('crate_types'):
            continue
        if dc in ('hit', 'peer', 'local', 'remote', 'prebuilt'):
            k = (u['crate_name'], u.get('version'), u.get('opt_level'))
            if k in crate_cpu:
                b['actual'] += crate_cpu[k]; ev['dep hit, CPU from measured miss'] += 1
            else:
                b['actual'] += u.get('depcache_saved_secs') or 0; ev['dep hit, CPU est. = original wall'] += 1
            continue
        if dc == 'miss':
            c, r = dep_miss_class(u.get('depcache_why'))
            b[c] += cpu; why[(c, r)] += cpu
            continue
        r = u.get('reuse')
        if not r:
            b['unknown'] += cpu; why[('unknown', 'compiled unit without reuse ledger')] += cpu
            continue
        warm = any(x['outcome'] == 'loaded' for x in r)
        led = 0.0
        for x in r:
            o, t = x['outcome'], x['secs']; led += t
            if o == 'loaded':
                m = mean(x['query'])
                if m is None:
                    why[('unknown', 'load of query never seen executing')] += 0
                    ev['loaded nodes without cost sample'] += x['count']
                else:
                    b['actual'] += m * x['count']; ev['loaded, avoided CPU est. from mean execution'] += x['count']
                b['necessary'] += t; why[('necessary', 'cache load cost')] += t
            elif o in ('green_recompute', 'same'):
                b['missed'] += t; why[('missed', o)] += t
            elif o == 'changed':
                b['necessary'] += t; why[('necessary', 'changed')] += t
            elif o == 'new':
                c = 'necessary' if warm else 'unknown'
                b[c] += t; why[(c, 'new (warm session)' if warm else 'new (cold session: prior state may have existed elsewhere)')] += t
            else:  # nohash, eval_always
                b['unknown'] += t; why[('unknown', o)] += t
        rest = max(cpu - led, 0.0)
        b['unknown'] += rest; why[('unknown', 'CPU outside query ledger (LLVM threads, link, metadata)')] += rest
    fresh = (s.get('units') or {}).get('fresh') or 0
    builds.append((rid, s, b, why, ev, fresh))

def cap(b):
    a, m, u = b['actual'], b['missed'], b['unknown']
    lo_max, hi_max = a + m, a + m + u
    lo = 100 * a / hi_max if hi_max else None
    hi = 100 * a / lo_max if lo_max else None
    return lo_max, hi_max, lo, hi

out = os.path.join(os.environ.get('JCODE_SCRATCH_DIR', '/tmp'), 'reuse-captured.jsonl')
T = collections.Counter(); W = collections.Counter(); E = collections.Counter(); F = 0; NB = 0
with open(out, 'w') as f:
    for rid, s, b, why, ev, fresh in builds:
        lo_max, hi_max, lo, hi = cap(b)
        f.write(json.dumps({'run': rid, 'args': ' '.join((s.get('args') or [])[:6]),
            'where': (s.get('remote') or {}).get('machine') or 'local', 'toolchain': s.get('toolchain'),
            'actual': round(b['actual'], 3), 'missed': round(b['missed'], 3),
            'necessary': round(b['necessary'], 3), 'unknown': round(b['unknown'], 3),
            'max_lo': round(lo_max, 3), 'max_hi': round(hi_max, 3),
            'captured_lo': lo and round(lo, 1), 'captured_hi': hi and round(hi, 1),
            'fresh_units_unquantified': fresh}) + '\n')
        T.update(b); W.update(why); E.update(ev); F += fresh; NB += 1
print(f'{NB} builds, per-build rows: {out}')
print({k: round(v / 3600, 2) for k, v in T.items()}, '(CPU hours)')
lo_max, hi_max, lo, hi = cap(T)
print(f'maximum safe reuse: {lo_max/3600:.2f}h .. {hi_max/3600:.2f}h; captured {lo:.1f}% .. {hi:.1f}%')
for k, v in sorted(W.items(), key=lambda kv: -kv[1])[:24]:
    print(f'  {v/3600:7.2f}h  {k[0]:9} {k[1]}')
print('evidence:', dict(E))
print('cargo-fresh units (reused, cost not logged):', F)
