# Fixed-machine compiler feasibility experiment

**Opt-in and NONAUTHORIZING.** This is a bounded local experiment with owned source inputs and output bytes. It does not authorize cache reuse, cross-tenant sharing, or production execution. Safe global cross-tenant caching remains unfinished.

Run from the checkout root:

```sh
python3 -m unittest discover -s bench/closed-execution -p 'test_*.py' -v
python3 bench/closed-execution/run.py --opt-in-nonauthorizing --output /your/new/results-directory
```

Requires the existing `target/debug/jr-rust`, `cc` with static libc, `readelf`, `ldd`, installed QEMU, and the default readable kernel (override with `--compiler`, `--kernel`, `--qemu`). Only run with a trusted owned compiler ELF: dependency discovery uses `ldd`, and native comparison executes that ELF. No compiler installation or Rust build is performed. No account or cache credentials are copied to the guest.

## Machine and scope

The runner packages a static C PID1 and a canonical sorted uncompressed newc archive. Inodes, ownership, timestamps, modes and padding are generated, not inherited. It copies the exact compiler, ELF interpreter, dynamic dependencies, and the existing compiler's native linker capsule inputs. Source capture is isolated under `/work`, not at the filesystem root. Hashes of all copied files, kernel, initramfs, QEMU executable, and the ordered configuration are in `report.json`.

The guest uses a versioned i440fx machine, one fixed qemu64 CPU, single-thread TCG, no KVM, fixed seed, fixed RTC base with VM clock, and fixed `icount shift=3,align=off,sleep=off`. RDRAND and RDSEED are disabled. No guest network, disk, host filesystem share, or monitor is configured. Guest `/proc` and `/dev` are guest-local procfs/devtmpfs, not host mounts. ASLR remains a guest userspace input. `nokaslr` only addresses kernel relocation and is not proof of address-space closure.

PID1 explicitly credits 512 bits from the public fixed 64-byte sequence `(17*i+3)%256` using `RNDADDENTROPY`, to avoid entropy starvation. These bytes are not secret and the ioctl does not establish that the resulting kernel random stream is the only entropy input. The VM is disposable and must never be used to generate secrets.

Each boot performs real valid CHECK, BUILD, execution of the owned ELF, invalid CHECK, invalid BUILD, missing-source rejection, and existing-output rejection. It verifies invalid BUILD leaves no executable, sends the complete bounded owned ELF bytes over serial, then powers off. Host parsing requires every expected exit status, exactly one PASS, and ELF bytes. Signals, exec errors, missing results, malformed data, QEMU failure, timeouts and output-limit failures do not report success. The host independently compares SHA-256 of actual ELF bytes across boots and native runs. This protocol is trusted experiment instrumentation, not hostile-guest attestation.

Host child process groups are killed and waited on during cleanup. Direct children also arm Linux `PR_SET_PDEATHSIG(SIGKILL)` with a parent-race check, because an actual server reload interrupted the first measurement and bypassed Python cleanup. The resulting orphan was explicitly terminated before restarting. Reports are checkpointed after native measurements and each completed guest boot. Abrupt host/server failure can still leave owned temporary files and incomplete reports, which must not count as acceptance. Each native command has a 90-second wall cap, each QEMU boot 480 seconds, logs 4 MiB, and guest children a 1800-second **guest virtual-time** alarm. `--repeats` permits only two or three boots. Temporary archives are removed. Small serial logs, report and owned ELFs remain in the selected output directory.

## Interpretation

Native wall times measure CHECK, BUILD and execution individually for the same compiler/source bytes with a minimal explicit environment. Guest wall time includes QEMU startup, kernel boot, archive unpacking, all seven operations, serial byte export, sync and shutdown. These are deliberately different scopes, not an apples-to-apples compiler overhead ratio. Host load is uncontrolled. Guest compiler JSON nanoseconds reflect the guest's instruction-counted virtual clock, not host elapsed CPU or wall time. The script makes no generalized speed claim.

Source locations differ between native and guest runs. The fixture's overflow/assertion panic text embeds those locations, so cross-environment ELF inequality is expected and is not by itself evidence of nondeterministic execution. Repeated guest bytes at the same guest paths are the repeatability observation. The report's `success_scope` deliberately limits success to owned-operation acceptance and same-path guest ELF equality. `native_guest_elf_equal` remains a separate observation and never authorizes sharing.

QEMU's primary documentation distinguishes instruction-counted operation from record/replay, which logs nondeterministic events. This experiment does **not** enable record/replay. Fixed icount, CPU, clocks and seed alone are not proof of complete determinism. Repeated equal ELFs are observations for these tiny fixtures on this host, not CPU, ASLR, scheduler, firmware, kernel-input, QEMU-version, or cross-host closure. The executable QEMU hash also does not cover all host shared libraries or firmware. Kernel/build configuration and all implicit device inputs are not exhaustively closed by this experiment.

## Retained observation (October 30, 2026 UTC)

`results/acceptance-durable/` contains two complete real boots, their exact owned ELF bytes, bounded serial logs and machine-readable report. Both boots passed all seven operation expectations. Both 8,496-byte guest ELFs have SHA-256 `f40ea7aaaecc89461d6806702c0e72fc12078f33565fe08145496ce4a17aeba6`. The complete serial logs also matched byte-for-byte in this pair. That is a two-run observation, not a general determinism proof.

| Host wall measurement | Repeat 1 | Repeat 2 |
| --- | ---: | ---: |
| Native valid CHECK | 5.527 s | 1.716 s |
| Native valid BUILD | 3.287 s | 5.226 s |
| Native owned executable run | 0.00235 s | 0.00410 s |
| Entire guest boot + seven operations + export/shutdown | 183.851 s | 174.351 s |

These measurements use the pre-existing debug compiler binary with SHA-256 `3e74d0659272e377f4a84c029a0c94a4e7261a2eff8fbd541ce5182c6cabb858`, one compiler worker, the 74-byte valid source embedded in the runner, and uncontrolled competing host load. They neither benchmark release-mode compiler performance nor compare equivalent total workloads. Native ELF size was 8,602 bytes and native/guest equality was false, consistent with the different embedded panic source paths. No local Rust build was necessary.

`results/failure-missing-loader/` retains one earlier actual failed boot and its `success: false` report: valid CHECK succeeded, BUILD explicitly rejected the incomplete native-toolchain capsule (missing `/usr/lib/ld-linux-x86-64.so.2`), no successful executable was reported, and the guest ended FAIL. The final image supplies that exact loader alias and hashes its bytes. Other interrupted/incomplete runs were not promoted to acceptance. Repeated tool-server reloads required a recorded detached `setsid` supervisor with an outer 1000-second `timeout`; the final supervisor, runner, monitor and QEMU all exited, and owned abandoned scratch files were removed. No ongoing background experiment remains.

Primary references consulted: <https://www.qemu.org/docs/master/system/replay.html> and <https://www.qemu.org/docs/master/system/qemu-manpage.html>. The eight Python tests are synthetic runner/error-path tests (including a real Linux child-process parent-death test), not VM acceptance. Actual acceptance evidence is only the reports and serial logs from real boots.
