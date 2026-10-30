#!/usr/bin/env python3
"""Opt-in NONAUTHORIZING fixed-machine feasibility experiment, not a cache gate."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import stat
import subprocess
import tempfile
import time

HERE = Path(__file__).resolve().parent
VALID = b'fn twice(x: i64) -> i64 { x + x }\nfn main() { assert!(twice(21) == 42); }\n'
INVALID = b'fn main() { let x: i64 = true; }\n'
LIMIT = 4 * 1024 * 1024


def sha(data):
    return hashlib.sha256(data).hexdigest()


class BoundedFailure(RuntimeError):
    def __init__(self, message, output, elapsed):
        super().__init__(message)
        self.output = output.decode('utf-8', errors='replace')
        self.elapsed = elapsed


def bounded(argv, timeout=180, env=None):
    start = time.monotonic()
    parent = os.getpid()
    libc = ctypes.CDLL(None, use_errno=True)
    def arm_parent_death():
        # Linux-only experiment: server reload must not leave an orphan QEMU.
        if libc.prctl(1, signal.SIGKILL, 0, 0, 0) != 0:
            os._exit(126)
        if os.getppid() != parent:
            os._exit(126)
    p = subprocess.Popen([str(x) for x in argv], stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                         start_new_session=True, env=env, preexec_fn=arm_parent_death)
    chunks = bytearray()
    sel = selectors.DefaultSelector()
    sel.register(p.stdout, selectors.EVENT_READ)
    try:
        while sel.get_map():
            if time.monotonic() - start > timeout:
                raise BoundedFailure('process wall timeout', chunks, time.monotonic() - start)
            for key, _ in sel.select(0.1):
                data = os.read(key.fd, 65536)
                if not data:
                    sel.unregister(key.fileobj)
                else:
                    chunks.extend(data)
                    if len(chunks) > LIMIT:
                        raise BoundedFailure('process output limit', chunks[:LIMIT], time.monotonic() - start)
        rc = p.wait(timeout=max(0.1, timeout - (time.monotonic() - start)))
        return {'returncode': rc, 'wall_seconds': time.monotonic() - start,
                'output': chunks.decode('utf-8', errors='replace')}
    finally:
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        p.wait()
        p.stdout.close()
        sel.close()


def newc(entries):
    """Canonical sorted newc, fixed inode order, uid/gid/mtime, no host metadata."""
    out = bytearray()
    for ino, (name, mode, data) in enumerate(sorted(entries) + [('TRAILER!!!', 0, b'')], 1):
        encoded = name.encode() + b'\0'
        fields = [ino, mode, 0, 0, 1, 0, len(data), 0, 0, 0, 0, len(encoded), 0]
        out.extend(b'070701' + ''.join(f'{x:08x}' for x in fields).encode())
        out.extend(encoded)
        out.extend(b'\0' * (-len(out) % 4))
        out.extend(data)
        out.extend(b'\0' * (-len(out) % 4))
    out.extend(b'\0' * (-len(out) % 512))
    return bytes(out)


def parse_guest(output):
    lines = output.replace('\r', '').splitlines()
    names = ['check', 'build', 'run', 'invalid-check', 'invalid-build',
             'missing-source', 'existing-output']
    results = {}
    for line in lines:
        m = re.fullmatch(r'CAPSULE RESULT (\S+) (\d+)', line)
        if m:
            if m[1] in results:
                raise ValueError('duplicate result')
            results[m[1]] = int(m[2])
    if set(results) != set(names):
        raise ValueError('missing or unexpected operation results')
    for name in names:
        code = results[name]
        if (code == 0) != (name in names[:3]) or code in (127, 255):
            raise ValueError('operation failed expectation: ' + name)
    if output.count('E0308: mismatched types:') != 2:
        raise ValueError('invalid inputs did not produce semantic diagnostics')
    if lines.count('CAPSULE DONE PASS') != 1 or 'CAPSULE DONE FAIL' in lines:
        raise ValueError('guest did not pass')
    elfs = [line[len('CAPSULE ELF '):] for line in lines if line.startswith('CAPSULE ELF ')]
    if len(elfs) != 1:
        raise ValueError('missing ELF bytes')
    elf = bytes.fromhex(elfs[0])
    if not elf.startswith(b'\x7fELF') or len(elf) > 1048576:
        raise ValueError('bad ELF')
    return results, elf


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--opt-in-nonauthorizing', action='store_true', required=True)
    parser.add_argument('--compiler', type=Path, default=HERE.parents[1] / 'target/debug/jr-rust')
    parser.add_argument('--kernel', type=Path, default=Path('/usr/lib/modules/7.2.11-arch1-1/vmlinuz'))
    parser.add_argument('--qemu', type=Path, default=Path('/usr/bin/qemu-system-x86_64'))
    parser.add_argument('--repeats', type=int, choices=[2, 3], default=3)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.compiler = args.compiler.resolve(strict=True)
    args.kernel = args.kernel.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=False)
    report = {'nonauthorizing': True, 'success': False, 'repeats': [],
              'success_scope': 'owned operation acceptance and repeated same-path guest ELF equality only',
              'limits': 'No CPU/ASLR/kernel-input closure proof. No global cross-tenant cache authorization.'}
    try:
        with tempfile.TemporaryDirectory(prefix='closed-execution-') as scratch:
            root = Path(scratch)
            init = root / 'init'
            compile_init = bounded(['cc', '-static', '-Os', '-Wall', '-Wextra', '-Werror',
                                    '-o', init, HERE / 'init.c'], 60)
            if compile_init['returncode']:
                raise RuntimeError(compile_init['output'])
            dependencies = bounded(['ldd', args.compiler], 10)
            if dependencies['returncode']:
                raise RuntimeError(dependencies['output'])
            libs = sorted(set(re.findall(r'(?:=>\s+)?(/\S+)\s+\(', dependencies['output'])))
            if not libs:
                raise RuntimeError('expected dynamic interpreter/dependencies')
            files = {'compiler': args.compiler.read_bytes(), 'init': init.read_bytes(),
                     'work/valid.rs': VALID, 'work/invalid.rs': INVALID}
            elf_headers = bounded(['readelf', '-l', args.compiler], 10)
            interp = re.search(r'Requesting program interpreter: ([^\]]+)', elf_headers['output'])
            if not interp:
                raise RuntimeError('missing ELF interpreter')
            files[interp[1].lstrip('/')] = Path(interp[1]).read_bytes()
            for lib in libs:
                files[lib.lstrip('/')] = Path(lib).read_bytes()
            # Existing compiler captures this native tool capsule even for pure Rust BUILD.
            tool_inputs = ['/usr/bin/ld', '/usr/lib/crt1.o', '/usr/lib/crti.o',
                           '/usr/lib/crtn.o', '/usr/lib/libc_nonshared.a',
                           '/usr/lib/ld-linux-x86-64.so.2']
            ld_deps = bounded(['ldd', '/usr/bin/ld'], 10)
            if ld_deps['returncode']:
                raise RuntimeError(ld_deps['output'])
            tool_inputs += re.findall(r'(?:=>\s+)?(/\S+)\s+\(', ld_deps['output'])
            for item in tool_inputs:
                files[item.lstrip('/')] = Path(item).read_bytes()
            directories = {'dev', 'proc', 'tmp', 'empty'}
            for name in files:
                directories.update(str(p) for p in Path(name).parents if str(p) != '.')
            entries = [(name, stat.S_IFDIR | 0o755, b'') for name in directories]
            entries += [(name, stat.S_IFREG | (0o644 if name.endswith('.rs') else 0o755), data)
                        for name, data in files.items()]
            archive = newc(entries)
            archive_path = root / 'initramfs.cpio'
            archive_path.write_bytes(archive)
            machine = 'pc-i440fx-11.1'
            config = ['-machine', machine, '-accel', 'tcg,thread=single', '-cpu',
                      'qemu64,-rdrand,-rdseed', '-smp', '1', '-m', '1024', '-nodefaults',
                      '-no-user-config', '-display', 'none', '-monitor', 'none',
                      '-serial', 'stdio', '-nic', 'none', '-no-reboot',
                      '-seed', '42', '-rtc', 'base=2026-01-01T00:00:00,clock=vm',
                      '-icount', 'shift=3,align=off,sleep=off',
                      '-append', 'console=ttyS0 quiet panic=-1 rdinit=/init nokaslr random.trust_cpu=off random.trust_bootloader=off']
            report.update({'qemu_version': bounded([args.qemu, '--version'], 10)['output'],
                           'qemu_sha256': sha(args.qemu.read_bytes()),
                           'kernel_sha256': sha(args.kernel.read_bytes()),
                           'initramfs_sha256': sha(archive), 'initramfs_bytes': len(archive),
                           'input_sha256': {name: sha(data) for name, data in files.items()},
                           'config': config, 'config_sha256': sha(json.dumps(config).encode()),
                           'entropy': 'RNDADDENTROPY 512 credited bits, 64 public bytes byte[i]=(17*i+3)%256'})
            del archive
            # Same compiler/source bytes, native host paths and host environment differ from guest.
            source = root / 'work'
            source.mkdir()
            (source / 'valid.rs').write_bytes(VALID)
            (source / 'invalid.rs').write_bytes(INVALID)
            env = {'PATH': '/', 'HOME': '/empty', 'LANG': 'C', 'LC_ALL': 'C', 'TZ': 'UTC'}
            native = []
            for i in range(args.repeats):
                binary = root / f'native-{i}.elf'
                commands = [[args.compiler, source / 'valid.rs', '--workers', '1', '--check'],
                            [args.compiler, source / 'valid.rs', '--workers', '1', '-o', binary],
                            [binary]]
                measurements = []
                for cmd in commands:
                    result = bounded(cmd, 90, env)
                    measurements.append(result)
                    if result['returncode']:
                        raise RuntimeError('native command failed: ' + repr(measurements))
                native.append({'commands': measurements, 'elf_sha256': sha(binary.read_bytes()),
                               'elf_bytes': binary.stat().st_size})
            report['native'] = native
            (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
            for i in range(args.repeats):
                log = args.output / f'guest-{i}.log'
                try:
                    run = bounded([args.qemu, *config, '-kernel', args.kernel,
                                   '-initrd', archive_path], 480)
                except BoundedFailure as exc:
                    log.write_text(exc.output)
                    report['failed_guest_wall_seconds'] = exc.elapsed
                    raise
                log.write_text(run['output'])
                if run['returncode']:
                    raise RuntimeError('QEMU failure: ' + run['output'][-4000:])
                results, elf = parse_guest(run['output'])
                (args.output / f'guest-{i}.elf').write_bytes(elf)
                report['repeats'].append({'wall_seconds': run['wall_seconds'],
                                          'results': results, 'elf_sha256': sha(elf),
                                          'elf_bytes': len(elf), 'serial_sha256': sha(run['output'].encode())})
                (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
            hashes = [r['elf_sha256'] for r in report['repeats']]
            report['guest_elf_equal'] = len(set(hashes)) == 1
            report['native_guest_elf_equal'] = set(hashes + [n['elf_sha256'] for n in native]) == {hashes[0]}
            report['success'] = report['guest_elf_equal']
    except Exception as exc:
        report['error'] = str(exc)
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    return 0 if report['success'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
