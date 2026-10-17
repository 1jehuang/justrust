# Next bounded source-macro / mixed-site slice

**Historical plan after A1 commit `12826bfe3`; the bounded first slice is now
implemented and accepted under `SOURCE-MACROS.md`.** That document records the
actual scope, private admission gates, 17 stock / 48 native worker-mode matrix
and final 2,070-test / 102-binary frozen acceptance. The blockers below describe
the pre-implementation baseline, not the current implementation. This plan is
not authorization to generalize the bounded gate. Full Rust, nested/general
hygiene, item generation and broader source macros remain open.

## Exact first vertical slice

- One function-local `macro_rules!` definition, visible in source order.
- Existing pure engine's `ident` / `tt` matcher fragments only.
- Invocation in expression position in that function or its nested blocks.
- Expansion must form one complete expression in the existing supported grammar.
- Preserve definition-site local lookup and invocation-site captured identifiers.
- No recursive/nested invocations, generated items, exports/imports, `$crate`,
  procedural macros, new fragment parsers, or opaque literal parser ingress.
- Existing `println!` handling remains its existing special path. It must not be
  intercepted as an unknown user macro by the new driver.

## Concrete existing source seams

| File / seam | Required bounded change |
|---|---|
| `src/compiler/frontend.rs`: `native_input`, `parse_prepared`, parser block/expression entry | Insert source driver after complete lexical validation, register source-order local macro definitions, parse a complete expanded expression. |
| New `src/compiler/source_macro.rs` (proposed) | Checked definition/invocation inventory and bounded pure-engine orchestration, sharing the original session and budget. |
| `src/compiler/token_tree.rs`, `macro_cursor.rs`, `macro_rules.rs` | Reuse `group`, checked `TokenSlice`, `compile_rules` and `expand_rules`; do not implement a second matcher. |
| `src/compiler/frontend/native_adapter.rs` | Add checked expanded-stream ingress preserving constituent spans, contexts, logical Joint tokens and source provenance. |
| `src/compiler/semantic_name.rs`, `hygiene.rs`, `expansion_span.rs` | Bind real definition/call lexical anchors and class-sensitive checked resolution. Keep foreign/unanchored contexts fail closed. |
| `src/compiler/frontend/root_resolution.rs`, `type_check.rs` | Resolve marked locals through actual definition environments, captures through retained caller context, and item/member references through checked invocation context. |
| `src/compiler/frontend.rs`: `SourceInventory`, original-source replay | Retain expanded-body occurrence inventory and actual checked binder/call correspondence without granting synthetic canonical authority. |

## Concrete blockers before opening the gate

1. There is no production source-macro driver. The parser currently rejects
   `macro_rules` item syntax.
2. Native adapter ingress consumes `LexedEvents` tied to `PreparedLexInput` and
   contiguous prepared ranges. It rejects expanded occurrences and nonroot
   contexts, including punctuation/literals, and name minting forces root context.
   A render-and-relex bridge would lose provenance and is not acceptable.
3. Hygiene currently supplies immutable context algebra only. `ExpansionRecord`
   records definition/call spans, parent, crate and edition, not actual lexical
   environments. A span alone is not a resolution anchor.
4. Current local maps compare semantic keys within explicit scopes. Merely
   admitting marked keys or stripping all marks cannot make definition-site
   locals resolve correctly. Definition anchors must capture the lexical point,
   including source-order visibility, not just an enclosing block identifier.
5. Expression output needs logical-token grammar admission: balanced delimiters,
   preserved Joint constituents and full-expression consumption. Successful pure
   matching/transcription is not native expression acceptance. Unsupported opaque
   output must remain a typed boundary, not flatten into ordinary tokens.
6. Expanded bodies need an occurrence inventory tied to real checked bindings and
   canonical declarations. Existing original-source replay must not silently
   assume every node is an unchanged original source occurrence. Never fabricate
   `OriginalBodyId` authority from an expansion `SpanId` or expansion identifier.
   An expanded spelling/provenance record is not a replay or body-proof capability.
7. Preserve complete lexical validation before projection/expansion, effective cfg
   filtering before macro registration, exact session/ledger ownership, typed
   resource failures and all existing expansion caps.

## Exact native CHECK / BUILD witness

```rust
fn helper() -> i32 { 2 }
fn main() {
    let x: i32 = 40;
    macro_rules! add {
        ($arg:ident) => { x + helper() + $arg };
    }
    {
        let x: i32 = 1;
        let y: i32 = 3;
        let result: i32 = add!(y);
        if result == 45 {
            println!("ok");
        } else {
            println!("bad");
        }
    }
}
```

Required evidence, not yet obtained: actual native CHECK succeeds, native BUILD
publishes an executable, and running it prints exactly `ok\n` with exit status 0.
Repeat the native CHECK/BUILD path at workers 1 and 8, with no stock fallback.
Definition-site `x` is 40, invocation capture `y` is 3, and helper contributes 2.
Incorrect call-site-only lookup yields 6 and must fail the acceptance observation.

Required negative/control witnesses:
- Definition before `let x` cannot see that later binding even if invocation can.
- Unknown captured identifier rejects through the ordinary checked resolver.
- No matching arm and out-of-slice recursive invocation report typed failures.
- A false-cfg item cannot register a macro definition.
- Malformed lexical input remains Syntax even in excluded or inactive token trees.
- Existing root-only foreign-context and original replay mutation controls pass.

## Deferred implementation ownership

These are proposed lanes only. **The coordinator must freeze interfaces and
explicitly release shared parent files after A1 publication before assignment.**

- Rat: parent parser/block/expression seams, new source driver and lexical records.
- Buffalo: expanded token ingress and native adapter constituent-span transport.
- Blowfish: ownerchecked scope anchors, hygiene/name projection and root resolution.
- Butterfly: checker lookup and expanded occurrence-inventory/replay integration.
- Ram: stock controls and actual native CHECK/BUILD/run witnesses.
- Coordinator: shared registration, ownership arbitration, acceptance gates and
  publication. Keep this plan separate from accepted A1 evidence.

No compiler compatibility or speed claim follows from this proposed slice.
