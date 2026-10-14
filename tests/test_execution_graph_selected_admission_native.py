#!/usr/bin/env python3
"""Separate actual selected-admission gate, not part of the legacy native nine.

JR_SELECTED_RUSTC selects an already rebuilt private driver. This harness compiles
only fixtures. JR_SELECTED_STAGE0 and JR_SELECTED_METADATA_SYSROOT may point at
matching preserved metadata. Missing/mismatched metadata fails, never skips.
Selection is hard-coded in Callbacks::config. Capture uses the EXISTING sealed
reserved-fd-v2 transport with schema4/query and schema2/pool receipts. No new wire,
resource graph, full mapping, or default-promotion claim is made here.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import time
import unittest


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def reserve_ticket(data):
    """Map only our own immutable ticket, never overwrite a foreign fd 198."""
    fd = os.memfd_create('jr-query-binding', os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    try:
        os.write(fd, data)
        fcntl.fcntl(fd, fcntl.F_ADD_SEALS,
                    fcntl.F_SEAL_SEAL | fcntl.F_SEAL_SHRINK |
                    fcntl.F_SEAL_GROW | fcntl.F_SEAL_WRITE)
        reserved = fcntl.fcntl(fd, fcntl.F_DUPFD_CLOEXEC, 198)
        if reserved != 198:
            os.close(reserved)
            raise RuntimeError('foreign fd 198 occupied, refusing overwrite')
        return reserved
    finally:
        os.close(fd)


def prepare_capture(directory):
    """Encode unchanged toolchain/shared/query_binding.rs v1 ticket bytes."""
    directory.mkdir(mode=0o700)
    directory_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        strings = ['jr-rustc-query-binding-v1', 'selected-run', 'selected-invocation',
                   'selected-parent', 'selected-task', 'selected-entry', 'selected-query']
        data = b'JRQBFD01' + struct.pack('<IIi', 1, 4, directory_fd)
        for value in strings:
            encoded = value.encode('ascii')
            data += struct.pack('<H', len(encoded)) + encoded
        return reserve_ticket(data), directory_fd
    except BaseException:
        os.close(directory_fd)
        raise


MACRO = r'''extern crate proc_macro;
use proc_macro::TokenStream;
use std::hash::{Hash, Hasher};
#[proc_macro]
pub fn observe(_: TokenStream) -> TokenStream {
    let args: Vec<String> = std::env::args().collect();
    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    vars.sort();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    vars.hash(&mut hash);
    let private = args.iter().any(|a| a.contains("jr-query-binding-fd")
        || a.contains("frontend-admission") || a.contains("ticketed"));
    format!("const OBSERVED_ARGS: &str = {:?}; const OBSERVED_ENV: u64 = {}; const PRIVATE_MODE_ARG: bool = {};",
            format!("{:?}", args), hash.finish(), private).parse().unwrap()
}
'''
INPUT = '''observer::observe!();
fn selected_target() -> u64 { 42 }
fn main() {
    println!("value={}; private_mode_arg={}", selected_target(), PRIVATE_MODE_ARG);
    println!("argv={}", OBSERVED_ARGS);
    println!("env_fingerprint={}", OBSERVED_ENV);
}
'''


@unittest.skipUnless(os.environ.get('JR_SELECTED_RUSTC'), 'requires rebuilt JR_SELECTED_RUSTC')
class NativeSelectedAdmission(unittest.TestCase):
    def logged(self, command, label, *, pass_fds=(), expected=0):
        command = [str(a) for a in command]
        started = time.monotonic()
        try:
            result = subprocess.run(command, cwd=self.root, env=self.env,
                                    pass_fds=pass_fds, capture_output=True,
                                    text=True, timeout=120)
        except subprocess.TimeoutExpired as error:
            (self.root / (label + '.json')).write_text(json.dumps({
                'command': command, 'status': 'insufficientexercise',
                'reason': 'deadline is not evidence of Pending or query wait',
                'stdout': str(error.stdout), 'stderr': str(error.stderr)}, indent=2))
            self.fail('insufficientexercise: deadline, raw evidence in ' + str(self.root))
        (self.root / (label + '.json')).write_text(json.dumps({
            'command': command, 'exit_code': result.returncode,
            'elapsed_s': time.monotonic() - started,
            'stdout': result.stdout, 'stderr': result.stderr}, indent=2))
        if expected is not None:
            self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
        return result

    def setUp(self):
        base = os.environ.get('JR_SELECTED_EVIDENCE_DIR') or os.environ.get('JCODE_SCRATCH_DIR')
        if base:
            Path(base).mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix='native-selected-admission-', dir=base))
        print(json.dumps({'evidence_dir': str(self.root), 'retained': True}), flush=True)
        self.rustc = Path(os.environ['JR_SELECTED_RUSTC']).resolve(strict=True)
        self.stage = self.rustc.parent.parent
        self.sysroot = Path(os.environ.get('JR_SELECTED_RUN_SYSROOT', self.stage)).resolve(strict=True)
        host = self.stage.parent
        self.stage0 = Path(os.environ.get('JR_SELECTED_STAGE0', host / 'stage0/bin/rustc')).resolve(strict=True)
        self.metadata = Path(os.environ.get('JR_SELECTED_METADATA_SYSROOT', host / 'stage0-sysroot')).resolve(strict=True)
        drivers = list((self.stage / 'lib').glob('librustc_driver-*.so'))
        self.assertEqual(len(drivers), 1, 'require unique already rebuilt driver')
        driver = drivers[0]
        metadata = list(self.metadata.glob('lib/rustlib/*/lib/' + driver.with_suffix('.rmeta').name))
        self.assertEqual(len(metadata), 1, 'matching full private metadata required')
        self.env = os.environ.copy()
        # Sanitize once, never differently for recording off/on. No policy switch.
        for key in ('JUSTRUST_EXECUTION_GRAPH_DIR', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER',
                    'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_MAKEFLAGS', 'MAKEFLAGS', 'MFLAGS'):
            self.env.pop(key, None)
        self.env['RUSTC_BOOTSTRAP'] = '1'
        self.env['LD_LIBRARY_PATH'] = os.pathsep.join([
            str(driver.parent), str(metadata[0].parent), str(self.metadata / 'lib'),
            self.env.get('LD_LIBRARY_PATH', '')])
        fixture = Path(__file__).resolve().parent / 'fixtures/execution_graph_selected_admission.rs'
        smoke = fixture.with_name('execution_graph_selected_admission_smoke.rs')
        shutil.copyfile(fixture, self.root / 'callback-source.rs')
        shutil.copyfile(smoke, self.root / 'smoke-source.rs')
        shutil.copyfile(__file__, self.root / 'harness-source.py')
        artifacts = [self.rustc, self.stage0, driver, metadata[0], fixture, smoke, Path(__file__)]
        (self.root / 'private-artifacts.json').write_text(json.dumps({
            str(path): {'sha256': sha256(path), 'mtime_ns': path.stat().st_mtime_ns}
            for path in artifacts}, indent=2))
        self.callback = self.root / 'native-selected-callback'
        self.callback_build = [self.stage0, '--edition=2024', '--sysroot', self.metadata,
                               '-Ldependency=' + str(metadata[0].parent),
                               '-Ldependency=' + str(driver.parent),
                               '--extern', 'rustc_driver=' + str(metadata[0]),
                               '--extern', 'rustc_driver=' + str(driver), '-Cprefer-dynamic']
        if 'smoke' not in self._testMethodName:
            self.logged(self.callback_build + [fixture, '-o', self.callback], 'callback-build')
        else:
            cfg = {'test_smoke_default': [],
                   'test_smoke_serial_factory_not_called': ['--cfg=selected_serial_probe'],
                   'test_smoke_callback_panic_unwinds': ['--cfg=selected_callback_panic']}[self._testMethodName]
            self.logged(self.callback_build + cfg + [smoke, '-o', self.callback], 'smoke-build')
        macro = self.root / 'observer.rs'
        macro.write_text(MACRO)
        self.macro = self.root / 'libobserver.so'
        self.logged([self.rustc, '--sysroot', self.sysroot, '--crate-name=observer',
                     '--crate-type=proc-macro', macro, '-o', self.macro], 'macro-build')
        (self.root / 'build-products.json').write_text(json.dumps({
            'callback_sha256': sha256(self.callback),
            'macro_sha256': sha256(self.macro),
            'macro_source_sha256': sha256(macro)}, indent=2))
        self.source = self.root / 'input.rs'
        self.source.write_text(INPUT)
        self.artifact = self.root / 'input-bin'
        self.command = [self.callback, '--sysroot', self.sysroot, '-Zthreads=2',
                        '--edition=2024', '--crate-name=selected_input',
                        '--extern', 'observer=' + str(self.macro),
                        self.source, '-o', self.artifact]

    def observe(self, result):
        prefix = 'selected-admission-evidence: '
        rows = [json.loads(line[len(prefix):]) for line in result.stderr.splitlines()
                if line.startswith(prefix)]
        self.assertEqual(len(rows), 1, 'insufficientexercise: no positive controller evidence\n' + result.stderr)
        row = rows[0]
        for fact in ('actual_query_release', 'producer_completed', 'producer_parked_outside_locks',
                     'query_ready_reacquire', 'proxy_pending', 'returned_after_supply'):
            self.assertIs(row[fact], True, row)
        self.assertIs(row['returned_before_supply'], False, row)
        self.assertEqual(row['event_capacity'], 0)
        self.assertGreater(row['resumed_grants'], row['pending_grants'])
        self.assertGreater(row['resumed_acks'], row['pending_acks'])
        return row

    def inspect_receipts(self, sink):
        receipts = [json.loads(p.read_text()) for p in sink.glob('*.json')]
        entries = [r for r in receipts if r['producer'] == 'rustc-query-admission-entry']
        queries = [r for r in receipts if r['producer'] == 'rustc-query-admission']
        pools = [r for r in receipts if r['producer'] == 'rustc-thread-pool-join']
        self.assertEqual(len(entries), 1, receipts)
        self.assertEqual(len(queries), 1, receipts)
        self.assertEqual(len(pools), 1, receipts)
        query, pool = queries[0], pools[0]
        self.assertEqual(entries[0]['query_capture_ids'], ['selected-query'])
        self.assertEqual(query['capture_id'], 'selected-query')
        self.assertEqual(query['schema'], 4)
        self.assertEqual(pool['schema'], 2)
        self.assertEqual(pool['query_capture_id'], query['capture_id'])
        self.assertEqual(query['task_bindings']['pool']['capture_id'], pool['capture_id'])
        self.assertGreater(sum(e['kind'] == 'registered' for e in query['wait_events']), 0,
                           'insufficientexercise: capture lacks actual query wait')
        for receipt in receipts:
            self.assertFalse(receipt['full_pipeline_complete'])
        self.assertFalse(any(p.name.startswith('resource') for p in sink.iterdir()))
        return {p.name: sha256(p) for p in sink.glob('*.json')}

    def test_selected_actual_pending_and_macro_process_neutrality(self):
        off = self.logged(self.command, 'recording-off')
        off_facts = self.observe(off)
        self.assertIs(off_facts['qualified_context_observed'], False)
        off_hash = sha256(self.artifact)
        off_output = self.logged([self.artifact], 'execute-off').stdout
        ticket, directory = prepare_capture(self.root / 'capture')
        try:
            on = self.logged(self.command, 'recording-on', pass_fds=(ticket, directory))
        finally:
            os.close(ticket)
            os.close(directory)
        on_facts = self.observe(on)
        self.assertIs(on_facts['qualified_context_observed'], True)
        on_hash = sha256(self.artifact)
        on_output = self.logged([self.artifact], 'execute-on').stdout
        self.assertIn('value=42; private_mode_arg=false\n', off_output)
        self.assertEqual(off_output, on_output, 'proc macro observed argv/env difference')
        self.assertEqual(off_hash, on_hash, 'recording changed executable bytes')
        receipt_hashes = self.inspect_receipts(self.root / 'capture')
        (self.root / 'comparison.json').write_text(json.dumps({
            'off': off_facts, 'on': on_facts, 'identical_command': [str(a) for a in self.command],
            'identical_environment': True, 'artifact_sha256': on_hash,
            'macro_output': on_output, 'receipt_sha256': receipt_hashes,
            'full_mapping_claim': False, 'default_promotion': False}, indent=2))

    def smoke_pair(self, *, serial=False, panic=False):
        command = [a for a in self.command if not (serial and a == '-Zthreads=2')]
        off = self.logged(command, 'smoke-off', expected=None if panic else 0)
        if not panic:
            self.assertIn('selected-factory-smoke: calls=0', off.stderr)
            off_hash = sha256(self.artifact)
            off_output = self.logged([self.artifact], 'smoke-execute-off').stdout
        ticket, directory = prepare_capture(self.root / 'smoke-capture')
        try:
            on = self.logged(command, 'smoke-on', pass_fds=(ticket, directory),
                             expected=None if panic else 0)
        finally:
            os.close(ticket)
            os.close(directory)
        receipts = [json.loads(p.read_text()) for p in (self.root / 'smoke-capture').glob('*.json')]
        queries = [r for r in receipts if r['producer'] == 'rustc-query-admission']
        self.assertEqual(len(queries), 1, receipts)
        self.assertEqual(queries[0]['schema'], 4)
        self.assertEqual(queries[0]['capture_id'], 'selected-query')
        for receipt in receipts:
            self.assertFalse(receipt['full_pipeline_complete'])
        if panic:
            for result in (off, on):
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('selected-admission-intentional-callback-panic', result.stderr)
            self.assertFalse(queries[0]['capture_complete'])
            self.assertNotEqual(queries[0]['interface_outcome'], 'returned')
            # This is callback unwind coverage, NOT blocked-query panic coverage.
        else:
            self.assertIn('selected-factory-smoke: calls=0', on.stderr)
            self.assertEqual(off_hash, sha256(self.artifact))
            self.assertEqual(off_output, self.logged([self.artifact], 'smoke-execute-on').stdout)
            self.assertIn('value=42; private_mode_arg=false', off_output)
        (self.root / 'smoke-comparison.json').write_text(json.dumps({
            'command': [str(a) for a in command], 'serial': serial,
            'callback_panic_only_not_blocked_query_panic': panic,
            'identical_environment': True,
            'artifact_sha256': None if panic else sha256(self.artifact),
            'receipt_sha256': {p.name: sha256(p) for p in (self.root / 'smoke-capture').glob('*.json')},
            'full_mapping_claim': False, 'default_promotion': False}, indent=2))
        if serial:
            self.assertFalse(any(r['producer'] == 'rustc-thread-pool-join' for r in receipts))

    def test_smoke_default(self):
        self.smoke_pair()

    def test_smoke_serial_factory_not_called(self):
        self.smoke_pair(serial=True)

    def test_smoke_callback_panic_unwinds(self):
        self.smoke_pair(panic=True)

    def test_foreign_reserved_descriptor_fails_open(self):
        foreign = os.open(self.root / 'foreign', os.O_RDWR | os.O_CREAT | os.O_EXCL, 0o600)
        os.write(foreign, b'not a native binding ticket')
        reserved = fcntl.fcntl(foreign, fcntl.F_DUPFD_CLOEXEC, 198)
        try:
            self.assertEqual(reserved, 198, 'foreign slot occupied, cannot run probe')
            result = self.logged(self.command, 'foreign-fd', pass_fds=(reserved,))
            self.observe(result)
            self.assertEqual(os.pread(foreign, len(b'not a native binding ticket'), 0),
                             b'not a native binding ticket')
        finally:
            os.close(reserved)
            os.close(foreign)

    def test_type_error_preserves_diagnostics_with_selected_policy(self):
        self.source.write_text(INPUT + '\nfn bad() -> u32 { "wrong type" }\n')
        off = self.logged(self.command, 'error-off', expected=None)
        self.assertNotEqual(off.returncode, 0, off.stdout + off.stderr)
        self.observe(off)
        ticket, directory = prepare_capture(self.root / 'error-capture')
        try:
            on = self.logged(self.command, 'error-on', pass_fds=(ticket, directory), expected=None)
        finally:
            os.close(ticket)
            os.close(directory)
        self.assertEqual(on.returncode, off.returncode)
        self.observe(on)
        strip = lambda text: '\n'.join(line for line in text.splitlines()
                                      if not line.startswith('selected-admission-evidence: '))
        self.assertIn('mismatched types', off.stderr)
        self.assertEqual(strip(off.stderr), strip(on.stderr))


if __name__ == '__main__':
    unittest.main(verbosity=2)
