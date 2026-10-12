#!/usr/bin/env python3
"""Time early exit would have saved on the recorded agent compile-error runs.

Inputs:
  runs     replay_runs.py output (the 563 agent runs that ended in
           `could not compile`), plus each run's first rustc error code
           (mine_codes.py output, joined by run id)
  bench    an early-exit-mutations-*.json (bench/early_exit/mutations.py):
           per rustc error code, how often early exit fired correctly,
           wrongly, or not at all, and rust-analyzer's latency

Model per run with first error code C (rates from the mutation bench for C,
the pooled rate when C has fewer than 3 mutations):
  fires correctly (p_ok):   the agent waits t_early = client overhead +
                            rust-analyzer's answer, instead of `wall`
  fires wrongly (p_wrong):  the agent acts on a wrong error. Charged as a
                            whole extra run: + wall (pessimistic, the agent
                            might just rerun)
  does not fire:            unchanged
Expected saving = sum over runs of p_ok * (wall - t_early) - p_wrong * wall.

Usage: replay_savings.py replay.json codes.json mutations.json
"""
import json
import statistics as st
import sys
from collections import defaultdict

CLIENT_S = 0.03  # client, socket, recorder start before the query (e2e 77-81 ms incl. RA)


def main():
    replay = {r["id"]: r for r in json.load(open(sys.argv[1]))}
    codes = {r["id"]: r for r in json.load(open(sys.argv[2]))}
    bench = json.load(open(sys.argv[3]))
    rows = [r for r in bench["rows"] if not r["clean"] and r["rc"] != 0]
    by = defaultdict(lambda: [0, 0, 0])  # ok, wrong, total
    for r in rows:
        c = r["rustc"][0][2] if r["rustc"] else ""
        ok = r["verdict"] in ("correct", "partial", "near", "superset")
        wrong = r["verdict"].startswith("wrong")
        for k in (c, "*"):
            by[k][0] += ok
            by[k][1] += wrong
            by[k][2] += 1
    ra_s = st.median(r["ra_ms"] for r in bench["rows"]) / 1000
    t_early = CLIENT_S + ra_s

    def rates(c):
        b = by[c] if by[c][2] >= 3 else by["*"]
        return b[0] / b[2], b[1] / b[2]

    out = {"t_early_s": round(t_early, 3),
           "pooled": {"ok": by["*"][0], "wrong": by["*"][1], "n": by["*"][2]}}
    # `real`: drops runs of bench/suite.py and scratch copies (the pinned
    # Desktop type-error scenarios are a large share of the 563).
    for name, keep in (("all", lambda c: True),
                       ("real", lambda c: c is not None and "/bench/" not in c["cwd"] and "/scratch/" not in c["cwd"])):
        out[name] = model({k: v for k, v in replay.items() if keep(codes.get(k))}, codes, rates, t_early)
    print(json.dumps(out, indent=1))


def model(replay, codes, rates, t_early):
    total = saved = lost = 0.0
    n_code = 0
    per_code = defaultdict(lambda: [0, 0.0])
    for rid, r in replay.items():
        total += r["wall"]
        c = codes.get(rid)
        code = c["errs"][0]["code"] if c and c["errs"] else ""
        n_code += c is not None
        p_ok, p_wrong = rates(code)
        s = p_ok * max(0.0, r["wall"] - t_early)
        w = p_wrong * r["wall"]
        saved += s
        lost += w
        per_code[code or "syntax/other"][0] += 1
        per_code[code or "syntax/other"][1] += s - w
    return {
        "runs": len(replay), "runs_with_code": n_code, "total_wall_s": round(total, 1),
        "expected_saved_s": round(saved, 1), "expected_wrong_cost_s": round(lost, 1),
        "net_s": round(saved - lost, 1), "net_frac": round((saved - lost) / total, 3),
        "by_code": {k: {"runs": v[0], "net_s": round(v[1], 1),
                        "rate_ok": round(rates("" if k == "syntax/other" else k)[0], 2)}
                    for k, v in sorted(per_code.items(), key=lambda kv: -kv[1][1])},
    }


if __name__ == "__main__":
    main()
