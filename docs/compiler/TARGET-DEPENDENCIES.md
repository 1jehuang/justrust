# Target-conditioned dependency selection

Status: implemented for the pinned native Linux x86-64 target; acceptance results are recorded below after verification. This is a bounded project-discovery improvement, not full Cargo resolution or self-hosting.

## Frozen contract

- `[target.'cfg(...)'.dependencies]` and `dev-dependencies` use the existing bounded cfg parser with the pinned native target facts. Exact target-triple tables select `x86_64-unknown-linux-gnu`.
- Boolean `all`, `any`, and `not` predicates are validated even inside branches whose value is already determined. Malformed predicates fail explicitly.
- Crate/profile-dependent `feature`, `debug_assertions`, `test`, `proc_macro`, and `panic` predicates are explicitly unsupported for dependency selection.
- Inactive edges remain declarations for feature-name lookup but do not activate, compile, or acquire registry/git packages. Inactive path declarations still require a real package with the declared identity, matching Cargo's manifest validation.
- Repeated same-name declarations are considered together when applying `dep:`, strong dependency-feature, and implicit optional-dependency feature requests. An inactive declaration must not mask an active one.
- Optional locked registry dependencies whose source is unavailable or fails authentication do not block an unselected feature. Their locked identity remains in the manifest. Selecting the feature still requires authenticated source and fails explicitly when it is missing or corrupted. `tests/projects.rs::inactive_optional_registry_source_is_not_required_but_activation_checks_it` covers CHECK/BUILD/run at workers 1 and 8, including corrupted-source rejection after activation. This removes the cached-optional-package blocker exposed by the unchanged Adler2 probe, not its edition or source-language blockers.
- Active build dependencies remain explicitly unsupported. Inactive build dependencies do not demand their source packages.

## Representation and graph boundary

`Dependency::target_active` is immutable resolved manifest data. Existing project feature traversal and visible-dependency selection consume it. This does not add compiler node kinds, global state, or change graph-integrity ceilings. The existing project driver remains a compatibility limitation: this patch does not implement per-crate graph-native orchestration.

## Observed acceptance

Hosted run `20261020-025956500-3578205` passed the initial four integration tests on 2026-10-20. The selected workspace matched stock Cargo output and native CHECK/run at workers 1 and 8. Final run `20261020-031347420-3616208` passed all five target-dependency tests alongside project configuration, projects and graph integrity (36 tests total). Final strict all-target Clippy passed in `20261020-031644716-3623631`.

## Acceptance gates

`tests/target_dependencies.rs` compares an unchanged conditional path-dependency workspace against stock Cargo, then native CHECK and run at workers 1 and 8. It covers duplicate conditional optional declarations, selected feature propagation, an inactive source containing `compile_error!`, inactive unavailable registry/build edges, malformed predicates, and inactive missing paths. Unit tests exercise native triples and Boolean target predicates.

Full Cargo feature unification, alternate targets, host/target build-script separation, git acquisition, proc macros, resolver-version compatibility, and self-hosting remain open. No unchanged real dependency or full-program corpus pass is claimed by these fixtures alone.
