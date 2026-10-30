#!/usr/bin/env python3
"""Differential correctness gate for the frozen native proc-macro cache."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

DEFAULT_RUSTC = '/home/jeremy/.jcode/scratch/dead-item-cache-20261030/sysroot/bin/rustc'


def execute(command, cwd, env):
    result = subprocess.run([str(x) for x in command], cwd=cwd, env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return {'status': result.returncode, 'stdout': result.stdout.decode(errors='replace'),
            'stderr': result.stderr.decode(errors='replace')}


def macro_source(kind, revision=0, api='modern'):
    features = {'tracked_env': 'proc_macro_tracked_env', 'tracked_path': 'proc_macro_tracked_path'}
    header = '#![feature(' + features[kind] + ')]\n' if kind in features else ''
    env_call = ('proc_macro::tracked::env_var' if api == 'modern' else 'proc_macro::tracked_env::var')
    path_call = ('proc_macro::tracked::path' if api == 'modern' else 'proc_macro::tracked_path::path')
    if kind == 'env':
        value = 'std::env::var("JR_MACRO_INPUT").unwrap()'
    elif kind == 'tracked_env':
        value = env_call + '("JR_MACRO_INPUT").unwrap()'
    elif kind in ('file', 'tracked_path'):
        track = path_call + '(&path);' if kind == 'tracked_path' else ''
        value = '{ let path = std::env::var("JR_MACRO_PATH").unwrap(); ' + track + ' std::fs::read_to_string(path).unwrap() }'
    else:
        value = 'String::from("constant")'
    return header + '''extern crate proc_macro;
#[proc_macro]
pub fn input(_: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let value = VALUE;
    format!("{:?}", format!("{}:REV", value)).parse().unwrap()
}
'''.replace('VALUE', value).replace('REV', str(revision))


def consumer(body):
    # Every transition edits the consuming function, not comments or metadata.
    return 'fn main() { println!("{}|{}", fixture::input!(), ' + str(body) + '); }\n'


def states(kind):
    result = [{'label': 'warm', 'body': 0, 'input': 'alpha', 'revision': 0},
              {'label': 'body-edit', 'body': 1, 'input': 'alpha', 'revision': 0}]
    if kind != 'constant':
        result.append({'label': 'external-input-and-body-edit', 'body': 2, 'input': 'beta', 'revision': 0})
    result.append({'label': 'macro-rebuild-and-body-edit', 'body': 3, 'input': 'beta' if kind != 'constant' else 'alpha', 'revision': 1})
    return result


def normalize(record, root):
    return {key: value.replace(str(root), '<WORK>') if isinstance(value, str) else value
            for key, value in record.items()}


def equivalent(a, b):
    return a == b


def expected_output(kind, state):
    value = 'constant' if kind == 'constant' else state['input']
    return f"{value}:{state['revision']}|{state['body']}\n"


class Harness:
    def __init__(self, rustc, root):
        self.rustc = str(Path(rustc).resolve())
        self.root = root
        self.env = os.environ.copy()
        self.env.pop('RUSTC_BOOTSTRAP', None)
        self.env.update(JR_MACRO_INPUT='alpha', JR_MACRO_PATH=str(root / 'input.txt'))
        self.observations = []

    def build_macro(self, kind, revision, api='modern'):
        source = self.root / 'macro.rs'
        source.write_text(macro_source(kind, revision, api))
        record = execute([self.rustc, '--crate-name', 'fixture', '--crate-type', 'proc-macro',
                          '--edition=2021', source, '-o', self.root / 'libfixture.so'], self.root, self.env)
        self.observations.append({'operation': 'build-macro', 'kind': kind, 'revision': revision,
                                  'api': api, 'result': record})
        return record

    def compile_consumer(self, incremental, enabled):
        binary = self.root / 'consumer'
        binary.unlink(missing_ok=True)
        command = [self.rustc, '--edition=2021', self.root / 'consumer.rs',
                   '--extern', f"fixture={self.root / 'libfixture.so'}", '-o', binary,
                   '-C', f'incremental={incremental}', '-Zcache-proc-macros=' + ('yes' if enabled else 'no')]
        compile_result = execute(command, self.root, self.env)
        run = execute([binary], self.root, self.env) if compile_result['status'] == 0 else None
        return {'compile': normalize(compile_result, self.root),
                'runtime': normalize(run, self.root) if run is not None else None}

    def scenario(self, kind):
        api = 'modern'
        probe = self.build_macro(kind, 0, api)
        if probe['status'] and kind.startswith('tracked_'):
            api = 'legacy'
            probe = self.build_macro(kind, 0, api)
        if probe['status']:
            return {'kind': kind, 'status': 'unsupported' if kind.startswith('tracked_') else 'setup-failed',
                    'reason': 'Macro fixture did not compile. Full probe diagnostics retained.', 'steps': []}
        steps = []
        revision = 0
        for index, state in enumerate(states(kind)):
            self.env['JR_MACRO_INPUT'] = state['input']
            (self.root / 'input.txt').write_text(state['input'])
            (self.root / 'consumer.rs').write_text(consumer(state['body']))
            if state['revision'] != revision:
                build = self.build_macro(kind, state['revision'], api)
                if build['status']:
                    return {'kind': kind, 'status': 'setup-failed', 'steps': steps}
                revision = state['revision']
            records = {}
            for enabled in (False, True):
                mode = 'on' if enabled else 'off'
                records[mode] = self.compile_consumer(self.root / f'{kind}-{mode}-incremental', enabled)
                records['fresh-' + mode] = self.compile_consumer(self.root / f'{kind}-{mode}-fresh-{index}', enabled)
            # Exercise switching the option in one shared incremental directory too.
            for enabled in (False, True):
                mode = 'on' if enabled else 'off'
                records['shared-' + mode] = self.compile_consumer(self.root / f'{kind}-shared-incremental', enabled)
            reference = records['fresh-off']
            matches = all(equivalent(value, reference) for value in records.values())
            expected = {'status': 0, 'stdout': expected_output(kind, state), 'stderr': ''}
            valid_reference = reference['compile']['status'] == 0 and reference['runtime'] == expected
            steps.append({'state': state, 'records': records, 'equal': matches,
                          'reference_matches_expected': valid_reference,
                          'status': 'pass' if matches and valid_reference else 'fail'})
        return {'kind': kind, 'api': api, 'status': 'pass' if all(s['status'] == 'pass' for s in steps) else 'fail', 'steps': steps}


def run(rustc, root):
    harness = Harness(rustc, root)
    version = execute([harness.rustc, '-vV'], root, harness.env)
    cases = [harness.scenario(kind) for kind in ('constant', 'env', 'tracked_env', 'file', 'tracked_path')]
    return {'schema': 1, 'rustc': harness.rustc,
            'rustc_sha256': hashlib.sha256(Path(harness.rustc).read_bytes()).hexdigest(),
            'version': version, 'cases': cases, 'observations': harness.observations,
            'limitations': ['Untracked external inputs are observational probes, not a supported purity contract.',
                            'Unsupported tracked APIs are not passes.',
                            'This tiny direct-rustc fixture does not validate Cargo integration or arbitrary macro side effects.'],
            'status': 'fail' if any(c['status'] in ('fail', 'setup-failed') for c in cases) else
                      ('incomplete' if any(c['status'] == 'unsupported' for c in cases) else 'pass')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', default=DEFAULT_RUSTC)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--work-dir', type=Path, help='New empty directory for retained fixture and incremental caches')
    args = parser.parse_args()
    root = args.work_dir or Path(tempfile.mkdtemp(prefix='macro-input-'))
    if args.work_dir:
        root.mkdir(parents=True, exist_ok=False)
    root = root.resolve()
    report = run(args.rustc, root)
    report['work_dir'] = str(root)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'output': str(args.output), 'work_dir': str(root)}))
    return 0 if report['status'] == 'pass' else 1


if __name__ == '__main__':
    raise SystemExit(main())
