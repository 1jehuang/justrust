#!/usr/bin/env python3
"""Opt-in actual native Cargo/rustc raw-argv neutrality acceptance.

Requires JUSTRUST_NATIVE_CARGO and JUSTRUST_NATIVE_RUSTC. Fixtures and raw
receipts survive under JUSTRUST_NATIVE_EVIDENCE_DIR (or JCODE_SCRATCH_DIR).
No toolchain builds, wrappers, cache switches, or existing source edits.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

from test_execution_graph_task_binding_native import assert_binding_inventory


@unittest.skipUnless(os.environ.get('JUSTRUST_NATIVE_CARGO') and
                     os.environ.get('JUSTRUST_NATIVE_RUSTC'),
                     'requires actual native Cargo and rustc')
class CompilerArgvNative(unittest.TestCase):
    def test_proc_macro_cannot_observe_private_binding_argument(self):
        base = os.environ.get('JUSTRUST_NATIVE_EVIDENCE_DIR') or os.environ.get('JCODE_SCRATCH_DIR')
        if base:
            Path(base).mkdir(parents=True, exist_ok=True)
        root = Path(tempfile.mkdtemp(prefix='compiler-argv-native-', dir=base))
        print('EVIDENCE_DIR=' + str(root), flush=True)
        env = os.environ.copy()
        for key in ('JUSTRUST_EXECUTION_GRAPH_DIR', 'RUSTC_WRAPPER',
                    'RUSTC_WORKSPACE_WRAPPER', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS',
                    'CARGO_TARGET_DIR', 'CARGO_BUILD_RUSTFLAGS'):
            env.pop(key, None)
        env['RUSTC'] = str(Path(env['JUSTRUST_NATIVE_RUSTC']).resolve(strict=True))
        if os.environ.get('JUSTRUST_NATIVE_SYSROOT'):
            env['RUSTFLAGS'] = '--sysroot=' + str(Path(os.environ['JUSTRUST_NATIVE_SYSROOT']).resolve(strict=True))
        env['CARGO_HOME'] = str(root / 'cargo-home')
        Path(env['CARGO_HOME']).mkdir(mode=0o700)
        cargo = str(Path(env['JUSTRUST_NATIVE_CARGO']).resolve(strict=True))
        version = subprocess.run([env['RUSTC'], '-vV'], env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(version.returncode, 0, version.stdout + version.stderr)
        self.assertEqual([line for line in version.stdout.splitlines()
                          if line.startswith('jr-query-admission-binding:')],
                         ['jr-query-admission-binding: reserved-fd-v2,query-schema=4'])
        (root / 'Cargo.toml').write_text('[workspace]\nmembers = ["app", "observer"]\nresolver = "2"\n')
        for crate in ('app', 'observer'):
            (root / crate / 'src').mkdir(parents=True)
        (root / 'app/Cargo.toml').write_text('[package]\nname="argv-app"\nversion="0.1.0"\nedition="2021"\n[dependencies]\nobserver={path="../observer"}\n')
        (root / 'observer/Cargo.toml').write_text('[package]\nname="observer"\nversion="0.1.0"\nedition="2021"\n[lib]\nproc-macro=true\n')
        (root / 'observer/src/lib.rs').write_text('''extern crate proc_macro;
use proc_macro::TokenStream;
#[proc_macro]
pub fn observe(_: TokenStream) -> TokenStream {
    let args: Vec<String> = std::env::args().collect();
    let seen = args.iter().any(|a| a.starts_with("-Zjr-query-binding-fd="));
    format!("const BINDING_VISIBLE: bool = {}; const COMPILER_ARGS: &str = {:?};", seen, format!("{:?}", args)).parse().unwrap()
}
''')
        source = root / 'app/src/main.rs'
        source.write_text('observer::observe!();\nfn main() { println!("binding_visible={}", BINDING_VISIBLE); println!("compiler_args={}", COMPILER_ARGS); }\n')
        sink = root / 'capture'
        sink.mkdir(mode=0o700)
        outputs = {}
        hashes = {}
        for label in ('off', 'on'):
            if label == 'on':
                # Change only mtime, leaving source, flags, profile and target identical.
                time.sleep(1.1)
                os.utime(source, None)
            run_env = env.copy()
            if label == 'on':
                run_env['JUSTRUST_EXECUTION_GRAPH_DIR'] = str(sink)
            command = [cargo, 'build', '--offline', '-vv', '-p', 'argv-app']
            p = subprocess.run(command, cwd=root, env=run_env, capture_output=True, text=True, timeout=240)
            (root / (label + '-build.stdout')).write_text(p.stdout)
            (root / (label + '-build.stderr')).write_text(p.stderr)
            (root / (label + '-command.json')).write_text(json.dumps({'command': command, 'rustc': env['RUSTC'], 'cargo_home': env['CARGO_HOME'], 'capture': str(sink) if label == 'on' else None, 'exit_code': p.returncode}, indent=2))
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            executable = root / 'target/debug/argv-app'
            hashes[label] = hashlib.sha256(executable.read_bytes()).hexdigest()
            binary = subprocess.run([str(executable)], cwd=root, env=env, capture_output=True, text=True, timeout=30)
            (root / (label + '-app.stdout')).write_text(binary.stdout)
            (root / (label + '-app.stderr')).write_text(binary.stderr)
            self.assertEqual(binary.returncode, 0, binary.stderr)
            outputs[label] = binary.stdout
            print(label.upper() + ': ' + binary.stdout, flush=True)
        (root / 'comparison.json').write_text(json.dumps(outputs, indent=2))
        self.assertIn('binding_visible=false', outputs['off'])
        entries = [json.loads(path.read_text()) for path in sink.glob('*.json')
                   if json.loads(path.read_text()).get('producer') == 'rustc-query-admission-entry']
        self.assertTrue(entries, 'capture-on must exercise actual compiler binding, not unsupported fallback')
        for entry in entries:
            self.assertEqual(entry['query_receipt_schema'], 4)
            for capture_id in entry['query_capture_ids']:
                query = json.loads((sink / (capture_id + '.json')).read_text())
                assert_binding_inventory(self, query)
        self.assertIn('binding_visible=false', outputs['on'],
                      'Recording changed proc-macro-visible process argv and application output. Evidence: ' + str(root))
        self.assertEqual(outputs['off'], outputs['on'], 'complete proc-macro-visible argv changed')
        self.assertEqual(hashes['off'], hashes['on'], 'recording changed executable bytes')


if __name__ == '__main__':
    unittest.main()
