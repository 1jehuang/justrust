# Target alignment observations

Status: supported query forms implemented. Full Rust layout support remains open. Focused stock/native and graph gates pass. Integration limitations from older parent snapshots are tracked separately, not reported as full-suite success.

`std::mem::align_of::<T>()` remains an ordinary canonical source call. Its embedded body issues an immutable typed query authenticated by declaration, rigid parameter, registry snapshot and exact generic substitution. Generic queries remain symbolic until specialization. CHECK requires no code generation.

The Linux x86-64 query computes native scalar alignment, thin pointers, arrays, tuples and supported Rust structs recursively. This is distinct from private compiler frame and allocation storage stride. It does not change `size_of`, implement enum layout or guarantee `Option<NonNull<T>>` niches. Unsupported enum, closure and unsized layout queries fail explicitly. Over-alignment attributes are currently rejected at source ingress, before the layout query exists.

IR and CFG retain the sealed query and validate its owner and queried type. Closed instances lower to a usize constant. Unresolved queries cannot reach successful machine-code generation. No whole-program node, mutable published fact or external result slot is added.

Acceptance: stock differential CHECK/BUILD/native execution at workers 1 and 8 for scalar widths, arrays, nested structs and generic calls; foreign-source ingress rejection; sealed query owner, parameter and queried-type tamper rejection; unsupported queries rejected during CHECK and BUILD. Graph integrity and full integration gates are required before landing.
