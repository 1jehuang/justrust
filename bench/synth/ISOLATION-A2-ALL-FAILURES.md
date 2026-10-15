# A2 exhaustive failing-test isolation audit, 2026-10-15

Follow-up to [ISOLATION-A2.md](ISOLATION-A2.md). This is diagnostic evidence, not a benchmark rerun, changed expectation, or clean-baseline acceptance.

## Immutable inputs and receipt

- Original `bench/results/native-dispatch-synth40-A2-20261015.json` SHA256: `7b59851e5a660e9fdf18f2953184645ce230307f489ef8a122b228613ff601d3`.
- Original audit `bench/results/native-dispatch-synth40-A2-audit-20261015.json` SHA256: `e31af6d605ca6a386a48c1eb2e2c368bf1bbd842e3de396b6709c1d5f104e0b8`.
- New full-output receipt: `isolation-a2-all-failures-20261015.json`, SHA256 `f197efb884e961d887be1a5a8bc13ea6f08ecbe7410909dc24005c5a45e7f82f`.
- Reproducer: `python3 bench/synth/audit_a2_failures.py "$A2"`. Requires the preserved A2 tree and existing four binaries. No Cargo/compiler invocation. Package-root cwd follows native Cargo's `compilation.rs` test execution contract, not merely recorded workspace command cwd.
- 23 unique failing tests, 36 historical occurrences across points 6, 11, 13, 15, 21, 28, 37. Each selected alone twice: original executable path and same-inode hashed `deps` hardlink. All 46 selected exactly one test. Four additional synthetic catalog controls selected exactly one test. No timeouts.
- Each child has fresh disposable HOME, JCODE_HOME, XDG, runtime, scratch and temporary state. Only PATH is inherited. Credentials, display and DBus variables are absent. Browser and telemetry are explicitly disabled. Python bytecode writes are disabled. Synthetic credentials are deliberately invalid strings. Tests retain their own internal fixture mutations.
- The receipt attests identical before/after hashes of all four preserved binaries and a complete snapshot tree manifest covering content, file modes, directories and symlink targets. No source, original observation or original result was changed. Temporary aliases/configs were removed.
- Historical complete environment and per-point executable hashes were not recorded. Preserved binaries can reflect later A2 recompiles. These are independent present-state reproductions, not exact historical replay. Clean single-test passes cannot rule out whole-process shared-state races.

## Complete classification

`F/F` means exit 101 at both original and deps paths. `F/P` means 101 then 0. `P/P` means both 0. Names below are exact receipt keys. Point numbers are original observations, not newly assigned measurements.

| Exact failed test | Points | Controls | Evidence classification |
| --- | --- | --- | --- |
| `applet_runtime::tests::bundled_github_applet_renders_valid_documents` | 6 | P/P | Not independently reproduced |
| `server::debug_command_exec::tests::debug_tool_selfdev_reload_returns_promptly_for_direct_execution` | 11 | F/F | Gitless snapshot |
| `tool::bash::tests::repository_commands_export_a_logged_cargo_function` | 11 | F/F | Gitless snapshot |
| `tool::tests::tool_descriptions_stay_under_token_cap` | 11 | F/F | Pinned source cap violation |
| `tool::tests::tool_parameter_descriptions_stay_under_token_cap` | 11 | F/F | Pinned source cap violation |
| `tool::tests::post_tool_feedback_hook_appends_stdout_to_tool_result` | 11 | P/P | Not independently reproduced |
| `auth::tests::browser_suppressed_inside_test_harness_without_env_overrides` | 13 | F/P | Cargo executable path regression |
| `provider::tests::test_anthropic_live_catalog_replaces_static_fallback_list` | 13 | P/P | Synthetic dual-credential contamination reproduces separately |
| `tui::app::helpers::helpers_tests::gather_ambient_info_filters_to_session_reminders_when_ambient_disabled` | 15,21,37 | P/P | Not independently reproduced |
| `tui::app::tests::answering_no_on_continue_prompt_lands_on_regular_session` | 15,21,37 | F/F | Unauthenticated fixture/UI expectation mismatch |
| `tui::app::tests::preparing_recent_project_review_finishes_onboarding_and_seeds_the_first_turn` | 15,21,37 | F/F | Gitless snapshot |
| `tui::app::tests::starting_recent_project_review_runs_as_a_visible_local_turn` | 15,21,37 | F/F | Gitless snapshot |
| `tui::app::tests::test_expand_badge_shortcut_does_not_collapse_full_inline_diff` | 15 | P/P | Not independently reproduced |
| `tui::app::tests::test_info_widget_local_direct_api_runtime_shows_cost_based_usage` | 15,21,37 | P/P | Not independently reproduced |
| `tui::app::tests::test_new_for_remote_restores_observe_mode_from_reload_state` | 15 | P/P | Not independently reproduced |
| `tui::app::tests::test_real_draw_click_on_body_anchored_image_label_cycles_level` | 15,21 | P/P | Not independently reproduced |
| `tui::app::tests::test_restore_session_adds_reload_message` | 15,21,37 | P/P | Not independently reproduced |
| `tui::ui::tests::basic::test_cache_retention_estimates_never_create_proactive_expiry_ui` | 15 | P/P | Not independently reproduced |
| `tui::app::tests::test_model_picker_waits_for_async_post_login_catalog_activation` | 21 | P/P | Not independently reproduced |
| `workspace::tests::the_hint_chip_uses_only_spare_tab_space` | 28 | P/P | Not independently reproduced |
| `tui::app::tests::test_login_completed_surfaces_new_provider_models_in_local_model_picker` | 37 | P/P | Not independently reproduced |
| `tui::app::tests::test_new_for_remote_restored_soft_interrupt_resend_triggers_dispatch_state` | 37 | P/P | Not independently reproduced |
| `tui::app::tests::test_subagent_model_large_catalog_uses_cached_searchable_picker` | 37 | P/P | Not independently reproduced |

Summary: seven F/F tests account for 13 occurrences, one F/P for one occurrence, 15 P/P for 22 occurrences. Of the latter, catalog has a sufficient independent contamination reproducer. The other 14 tests, accounting for 21 occurrences, remain unresolved. Do not claim they are fixed, inherently broken, or proven races.

### Git and pinned assertions

Pinned `jcode-build-support/src/paths.rs::is_jcode_repo` requires both a matching Cargo manifest and `.git`. Archive snapshots intentionally contain no `.git`. Logged-Cargo discovery uses the compiled manifest path directly, so changing command cwd or setting a repository override cannot repair that assertion. The debug failure says `Could not find jcode repository directory`. Recent-project tests fail at `onboarding_flow.rs:1583/1598`, `test session should start in Git repository`; their resolver walks ancestors for `.git` (`jcode-import-core/src/repo_ranking.rs`). Four tests, eight occurrences are independently explained by missing baseline metadata. No marker or Git metadata was added to A2.

Pinned token estimator is UTF-8 bytes divided by four. `compile_remote` description is 405 bytes, 101 estimated tokens against cap 20 (`tool/tests.rs:715`). Gmail bcc parameter description is 104 bytes, 26 against cap 25 (`tool/tests.rs:786`). Both fail without any application credentials/config. These are pinned-source expectation conflicts, not compiler or measurement successes.

The no-on-continue assertion at `onboarding_flow.rs:660` is `app.suggestion_prompts().is_empty() || !app.onboarding_welcome_active()`. The false answer finishes onboarding and returns to ordinary session state. `state_ui_input_helpers.rs::suggestion_prompts` still supplies a login hint when AuthStatus has no available credentials, while `onboarding_welcome_active` can remain true with no transcript and a nonempty suggestion list. The test's temporary app-home fixture does not establish an authenticated account. This explains the independently reproduced clean unauthenticated state. It does not prove failure under every credential configuration or authorize changing application/test expectations.

### Catalog contamination: exact synthetic control

| Synthetic application state | Exit |
| --- | --- |
| Neither API nor OAuth | 0 |
| API only | 0 |
| OAuth only | 0 |
| API plus OAuth | 101 |

The API file is `.jcode/config/jcode/anthropic.env`; OAuth fixture is `.jcode/auth.json`. The reproducer includes the fake values/schema; the receipt includes the complete assertion. The dual state reaches the same pinned `catalog_subscription.rs:162` failure as A2: expected only `claude-nebula-preview` and `claude-nebula-preview[1m]`, but actual includes the same 16-model expanded list. This establishes a sufficient credential-route interaction without reading live credentials. Historical secret files were not inspected and their presence in the historical process is not attested, so attribution of the original occurrence to these exact files remains a hypothesis supported by an exact assertion reproducer.

### Cargo path remedy already shipped separately

`5dc5f789e` publishes test binaries at Cargo's hashed profile `deps` path using native Cargo output publication and executes that public path, including split build dirs. It retains per-unit compiler storage and normal-bin behavior. Earlier receipt has same-inode positive and negative path controls. Two real hosted native Cargo CLI tests passed (debug/release, split paths, lib/integration/harness=false, JSON/no-run/direct execution/fresh relink). The frozen A2 toolchain was not replaced. This audit does not broaden that change.

## Proposed fresh-baseline policy, not silently implemented

A2 remains immutable and ineligible for speed claims. Any change below requires a new matched baseline/candidate cohort and explicit supported benchmark identity. Keep generator, snapshots, commands, test expectations, default parallelism, repetitions, weights and quiet thresholds unchanged unless separately approved as a newly named benchmark definition.

1. Version the environment policy, credential/provider routing policy, Git snapshot policy and execution mode. Include their canonical hashes in compare/resume compatibility checks, not merely informational metadata. Also attest toolchain/native Cargo identity and effective source/fixup hashes.
2. For Git-dependent projects, explicitly prepare deterministic Git-aware snapshots from the pinned commit plus an audited object bundle/ref manifest. Record HEAD/tree, reachable refs/tags needed by observable commands, bundle hash, effective source tree and existing benchmark fixup hash. Do not create an empty `.git`, copy live credentials/hooks/config/remotes, or share mutable object alternates. Verify Git observables and dependency topology before both stacks. Git-aware and archive-only cohorts must not compare as identical environments.
3. Specify a credential-free, application-owned state seed and its digest/lifecycle. Do not blindly replace HOME in the benchmark: Cargo/rustup/justrust caches and authentication rely on it. Review pinned app storage and external-auth paths, use dedicated application/runtime state plus explicit routing-variable scrubbing, and never copy live account state. Record key policy rather than secret values. Match pre/previous/measured logical state and seed resets across stacks/repetitions/points.
4. Run untimed correctness preflight using the unchanged full test scopes and ordinary parallelism under that declared policy. Individual controls cannot discharge the 14 unresolved tests or shared-process interactions. Pinned token caps and unauthenticated onboarding expectations require an explicit supported workload/source decision before a new cohort, not harness suppression or changing A2 expectations.
5. Only start a full paired measurement once outcomes are valid and the existing quiet gate passes. Current unrelated user activity must not be stopped and thresholds must not be relaxed. A hosted benchmark requires an explicitly supported, identically matched hosted execution-mode cohort, not a substitute laptop sample.

No new production harness isolation change is made by this follow-up. The earlier locking/restoration/failure-log fix preserves definitions. This follow-up ships only the diagnostic, safety tests, receipt and policy proposal.

## Validation and remaining gates

`PYTHONPATH=bench/synth:bench PYTHONDONTWRITEBYTECODE=1 python3 -m unittest bench/synth/test_audit_a2_failures.py bench/synth/test_isolation.py bench/test_synth_reporting.py bench/test_synth_context.py`: 34 tests passed. New five safety tests cover environment exclusion/telemetry/browser controls, synthetic-state cleanup, path escape rejection, child-group timeout cleanup and tree-manifest sensitivity. Full diagnostic: 50 targeted binary invocations, no builds/timeouts, identical original tree/binary hashes. These are not benchmark timings.

Earlier owned hosted exports remain `data/runs/isolation-a2-cargo/2026-10.jsonl`: check `20261015-052129578-780952`, native CLI test `20261015-052228514-792978`, strict clippy `20261015-052452170-815969`. Strict clippy is blocked by ten unchanged inherited native Cargo lint errors, as enumerated in the earlier audit. No lint allowance or cleanup was introduced. No full benchmark, compiler rebuild, default promotion or speedup claim is made.
