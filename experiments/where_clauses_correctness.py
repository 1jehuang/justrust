#!/usr/bin/env python3
"""Bounded type-parameter where-clause differential corpus, not full Rust.

`where T: Trait + Other` on free functions, inherent/trait impl headers and
associated functions joins the SAME predicate environment as inline bounds.
Valid cases must match stock stdout/status at native workers 1/8 (BUILD and
CHECK). Invalid Rust must be rejected by both compilers. Legal exclusions
(Self/higher-ranked/?Sized/outlives/projection subjects) are boundaries,
never parity. Uses the shared bounded module_correctness harness.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, run_case

OK = "where oracle ok\n"
MAIN = 'fn main(){if run()==%d{println!("where oracle ok");}}\n'

PRELUDE = ("struct S{x:i32}struct W<A>{x:A}"
           "trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}"
           "impl T for S{fn v(self)->i32{self.x}}"
           "impl U for S{fn u(self)->i32{self.x*10}}")

VALID = [
    ("free_where_single", "fn g<X>(x:X)->i32 where X:T{x.v()}"
     "fn run()->i32{g(S{x:3})}", 3),
    ("free_where_trailing_comma_multi", "fn h<X,Y>(x:X,y:Y)->i32 where X:T,Y:T+U,"
     "{<X as T>::v(x)+y.u()}fn run()->i32{h(S{x:1},S{x:2})}", 21),
    ("inline_and_where_same_bound", "fn g<X:T>(x:X)->i32 where X:T{x.v()}"
     "fn run()->i32{g(S{x:4})}", 4),
    ("inline_and_where_split_bounds", "fn k<X:T>(x:X,y:X)->i32 where X:U{x.v()+y.u()}"
     "fn run()->i32{k(S{x:1},S{x:2})}", 21),
    ("conditional_impl_where", "impl<A> T for W<A> where A:T{fn v(self)->i32{self.x.v()+1}}"
     "fn g<X>(x:X)->i32 where X:T{x.v()}fn run()->i32{g(W::<W<S>>{x:W::<S>{x:S{x:1}}})}", 3),
    ("inherent_impl_where_method", "impl<A> W<A> where A:T{fn get(self)->i32{self.x.v()}}"
     "fn run()->i32{(W::<S>{x:S{x:5}}).get()}", 5),
    ("method_where_on_impl_param", "impl<A> W<A>{fn get(self)->i32 where A:T{self.x.v()}}"
     "fn run()->i32{(W::<S>{x:S{x:6}}).get()}", 6),
    ("associated_fn_where_forwarding", "impl<A> W<A>{fn make(a:A)->W<A> where A:U{W{x:a}}"
     "fn take(self)->A{self.x}}fn run()->i32{W::<S>::make(S{x:2}).take().u()}", 20),
    ("where_forwarded_to_bounded_callee", "fn inner<X:T>(x:X)->i32{x.v()}"
     "fn outer<X>(x:X)->i32 where X:T{inner(x)}fn run()->i32{outer(S{x:8})}", 8),
    ("where_explicit_type_args", "fn g<X>(x:X)->i32 where X:U{x.u()}"
     "fn run()->i32{g::<S>(S{x:9})}", 90),
    ("where_bound_qualified_path", "fn g<X>(x:X)->i32 where X:T{X::v(x)}"
     "fn run()->i32{g(S{x:7})}", 7),
    ("where_unused_parameter_bound", "fn g<X,Y>(x:X)->i32 where Y:U,X:T{x.v()}"
     "fn run()->i32{g::<S,S>(S{x:2})}", 2),
]

INVALID = [
    ("where_missing_impl_concrete", "fn g<X>(x:X)->i32 where X:T{x.v()}"
     "fn run()->i32{g(5i32)}"),
    ("where_unproven_in_generic_caller", "fn g<X>(x:X)->i32 where X:T{7}"
     "fn bad<X>(x:X)->i32{g(x)}fn run()->i32{7}"),
    ("where_unknown_parameter", "fn g<X>(x:X)->i32 where Y:T{7}fn run()->i32{7}"),
    ("where_unknown_trait", "fn g<X>(x:X)->i32 where X:Missing{7}fn run()->i32{7}"),
    ("where_wrong_bound_method", "fn g<X>(x:X)->i32 where X:U{x.v()}fn run()->i32{7}"),
    ("impl_where_unsatisfied_method", "impl<A> T for W<A> where A:U{fn v(self)->i32{7}}"
     "struct Z{x:i32}impl T for Z{fn v(self)->i32{self.x}}"
     "fn run()->i32{(W::<Z>{x:Z{x:1}}).v()}"),
    ("where_unused_body_move", "fn g<X>(x:X)->(X,X) where X:T{(x,x)}fn run()->i32{7}"),
    ("method_where_unsatisfied", "impl<A> W<A>{fn get(self)->i32 where A:U{7}}"
     "struct Z{x:i32}impl T for Z{fn v(self)->i32{self.x}}"
     "fn run()->i32{(W::<Z>{x:Z{x:1}}).get()}"),
]

BOUNDARY = [
    ("self_where_predicate", "trait V{fn w(self)->i32 where Self:Sized;}"
     "impl V for S{fn w(self)->i32{self.x}}fn run()->i32{S{x:3}.w()}", 3),
    # Corrected before native classification: the original by-value ?Sized
    # parameter is stock E0277, so the boundary uses no unsized value.
    ("relaxed_sized_where", "fn g<X>()->i32 where X:?Sized{3}"
     "fn run()->i32{g::<S>()}", 3),
    ("concrete_subject_where", "fn g()->i32 where S:T{S{x:3}.v()}fn run()->i32{g()}", 3),
    ("higher_ranked_where", "fn g<X>(x:X)->i32 where for<'a> X:T{x.v()}"
     "fn run()->i32{g(S{x:3})}", 3),
]


def corpus():
    for name, body, value in VALID:
        yield Case(name, "valid", PRELUDE + body + "\n" + MAIN % value, OK)
    for name, body in INVALID:
        yield Case(name, "invalid", PRELUDE + body + "\n" + MAIN % 7)
    for name, body, value in BOUNDARY:
        yield Case(name, "boundary", PRELUDE + body + "\n" + MAIN % value, OK)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--rustc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compile-timeout", type=float, default=30)
    parser.add_argument("--run-timeout", type=float, default=5)
    args = parser.parse_args()
    rustc = args.rustc.absolute()
    binary = args.binary.resolve(strict=True)
    cases = list(corpus())
    assert len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-where-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)

        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "where-clauses-v1",
                "case_count": len(cases),
                "case_counts": {k: sum(c.kind == k for c in cases)
                                for k in ("valid", "invalid", "boundary")},
                "workers": [1, 8], "modes": ["build", "check"], "edition": 2024,
                "reference": version["stdout"],
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "corpus_sha256": corpus_hash,
                "note": "legal unsupported boundaries are not invalid Rust or parity"})
        totals = {"accepted_parity": 0, "invalid_rejected": 0,
                  "unsupported_boundary": 0, "failure": 0}
        for index, case in enumerate(cases):
            row = run_case(case, index, directory / str(index), binary, rustc,
                           args.compile_timeout, args.run_timeout)
            record(row)
            totals[row["outcome"]] += 1
            print(f"{case.name}: {row['outcome']} {row['failures']}", flush=True)
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})
        if totals["failure"]:
            raise SystemExit(f"{totals['failure']} where-clause discrepancies in {args.output}")


if __name__ == "__main__":
    main()
