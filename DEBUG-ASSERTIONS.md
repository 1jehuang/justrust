# Bounded debug assertions

Status: implementation in progress. Full Rust remains open.

Add checked `debug_assert!`, `debug_assert_eq!` and `debug_assert_ne!` with the same supported scalar comparison and literal-message domains as existing assertions. Native program configuration, not the compiler host profile, determines whether they execute. Preserve the current native default (enabled), and expose explicit enabled/disabled configuration consistently to source cfg evaluation, direct preparation and observed graph compilation. Reserved user cfg must not spoof this setting.

Disabled assertions remain parsed, name-resolved and type-checked, but neither arguments nor messages execute. Enabled assertions evaluate operands exactly once and reuse existing checked assertion/panic paths. Preserve lexical macro lookup, source occurrence authority, budget accounting, diagnostics and all-source validation. No general formatting, trait equality, optimizer or full Cargo profile implementation is implied.

Acceptance requires independently measured stock sources in both configurations, exact stdout/stderr and abort behavior, invalid arguments rejected even when disabled, cfg consistency, debug/release compiler builds independent of native program settings, public authority controls, CLI and observed graph workers1/8, full regression tests, strict lint and formatting.
