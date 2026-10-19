# Typed dependency-cycle witness prerequisite

Source snapshot `e3d8be3629b8e2ad82d2ba71cd34adb47a0c478f` contains only the engine prerequisite cherry-picked from reviewed `54d99c04e`, plus the previously published main. It does not activate scalar const parsing, evaluation or source support.

The private optional Report witness is constructed from actual unresolved live waits at engine quiescence, independently of captured trace events. It records a complete ordered closed cycle of consumer/producer keys and output slots. Count/byte arithmetic and diagnostic allocations are checked and fallible. External-waker waits, allocation failure and non-cycle failures do not mint partial evidence. Existing Error text and scheduling/acceptance behavior are unchanged. Compiler-specific E0391 classification belongs to the later source-const consumer and is not delivered here.

Explicit hosted checks:

| Check | Run | Result |
| --- | --- | --- |
| Check | `20261018-222247762-2041104` | Passed |
| Engine, typed witness and graph integrity | `20261018-222253730-2041489` | 29 passed |
| Witness allocation-failure/overflow units | `20261018-222310052-2042283` | 2 passed |
| Strict all-target lint | `20261018-222326303-2040538` | Passed |
| Complete library suite | `20261018-222342516-2044771` | 1,397 passed |

Formatting passed. Five owned redacted records are in `central.jsonl`. This is prerequisite scoped and library acceptance, not a new full integration-suite verdict. No graph budget, size ceiling or mutable-state debt exemption was added.
