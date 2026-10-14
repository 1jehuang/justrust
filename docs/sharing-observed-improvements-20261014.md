# Sharing: concrete observed improvements, 2026-10-14

This closes the delivered safe-sharing and native-cache correctness feedback loops using actual public-interface outcomes, not source inspection or aggregate test counts. It is a read-only re-audit of completed executions, not a new performance experiment. The adjacent JSON records hashes, exact exported run IDs and all 18 requirement observations with their original scopes.

## Public sharing actually delivered reuse

All eight fresh-cache CLI check/test executions across nightly/fullJR and default/debug0 restored one trusted prebuilt dependency, with **one public hit and zero cache misses per command**. Every test command ran **two tests successfully**. The audit rejoined each original recorded `output.jsonl` SHA, actual run summary and exported receipt, not just a report assertion. For example:

- `20261013-230143882-2018034`: nightly check, public hit 1, miss 0, exit 0.
- `20261013-230144909-2018143`: nightly tests, public hit 1, miss 0, two tests passed.
- `20261013-230149595-2019101`: fullJR/debug0 tests, public hit 1, miss 0, two tests passed.

The concrete outcome is reuse of an already compiled trusted dependency while preserving successful consumer checks/tests. It is not a claim that network transfer is always faster than local compilation. Timing ratios would require the separate prescribed before/after suite.

The normal installed/frozen hosted check, complete test suite, warning-free strict all-target lint and repeated check were also rejoined to actual recorded output hashes and exported records. The actual native invalidation and host-native/response-file integration tests passed within that complete source suite, not only in a copied fixture. Existing loader, package identity, private publication and error/recovery requirements retain their separate observed scopes in the JSON.

## Native caching is observably more correct

The preserved original real-shim acceptance failed after changing the bundled C helper from version 1 to version 2 without changing Rust metadata: the cached executable still printed **1**, while the same uncached compiler invocation printed **2**. The original failure log SHA is `2f937c5bce08a91d67d5b5040ac947a7b24a87cdf353c800fe50be9f2c6a0a26`.

Corrected real-shim run **`20261013-201333279-898502`**, already exported in `data/runs/sharing-20261014`, observed:

| Native payload | Actual executable output | Actual cache outcome |
|---|---|---|
| Version 1 | 1 | Miss |
| Changed to version 2, identical Rust metadata | 2 | Miss |
| Unchanged version 2, outputs removed before invocation | 2 | Hit with correct restoration |

This is the concrete before/after improvement: stale native code no longer produces the wrong result, and unchanged native code still benefits from caching. The historical first-green run is not relabeled as the later final client binary. The final native-safe client independently passed the same named integration assertions in the actual full hosted source suite, recorded in `sharing-reload-followthrough-20261014`.

## Limits remain explicit

- No cross-CPU stale execution or compile-time speedup is claimed.
- Public native build-script replay remains unauthorized pending a complete trusted native closure, kernel/CPU/environment determinants and ABI/runtime acceptance.
- Genuine first-new-tenant boot remains unobserved. Registry eager seeding remains off.
- The scheduled trusted producer is no longer pending: `sharing-fleet-runtime-20261014` records actual success, 81 reported bundle files, 19,994 unique index keys and zero public build-script runs. Not every new bundle was independently downloaded.
- The initial re-audit compared CLI tee-report bytes with recorded-output hashes and correctly failed. These are different representations. After using original `output.jsonl` bytes, all eight hashes match. This was an audit-harness mistake, not a hidden build failure.

No new compiler run, host operation, installation, fleet mutation or cache deletion was needed for this re-audit. The safe retrieval and native correctness requirements have actual observed outcomes. Unimplemented native-public authorization and first-tenant coverage are not declared complete.
