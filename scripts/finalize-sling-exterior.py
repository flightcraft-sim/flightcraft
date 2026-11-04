#!/usr/bin/env python3
"""Validate/install a completed Sling Blender job, capture actual engine proofs, and record both diffs.

Run on the host after the confined Blender job. --wait watches for its final validation receipt.
No Blender launch, network access, commit, or merge is performed here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value):
    temporary = path.with_name(path.name + '.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n')
    temporary.replace(path)


def changed_files(root):
    # -z preserves spaces and non-ASCII filenames without git's display quoting.
    raw = subprocess.check_output(['git', '-C', str(root), 'status', '--porcelain=v1', '-z'])
    fields = iter(raw.decode('utf-8').split('\0'))
    result = []
    for field in fields:
        if not field:
            continue
        entry = {'status': field[:2], 'path': field[3:]}
        if 'R' in field[:2] or 'C' in field[:2]:
            entry['previous_path'] = next(fields)
        result.append(entry)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cycles', type=Path, required=True)
    parser.add_argument('--pack', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--report-prefix', type=Path, required=True)
    parser.add_argument('--wait', action='store_true')
    parser.add_argument('--timeout-seconds', type=float, default=8 * 3600)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    cycles, pack, output, prefix = (p.resolve() for p in (args.cycles, args.pack, args.output, args.report_prefix))
    if not (0 < args.timeout_seconds <= 24 * 3600):
        parser.error('--timeout-seconds must be between 0 and 86400')
    prefix.parent.mkdir(parents=True, exist_ok=True)
    receipt_path = prefix.with_name(prefix.name + '-proof.json')
    status = dict(status='waiting_for_blender', engine=str(root), pack=str(pack), cycles=str(cycles),
                  output=str(output), source_sha256=sha256(pack / 'scripts/blender-exterior.py'),
                  engine_changed_files=changed_files(root), pack_changed_files=changed_files(pack),
                  phases=[], pending=['Blender output', 'validated install', 'nine engine views', '68-frame motion sheet'])
    write_json(receipt_path, status)
    deadline = time.monotonic() + args.timeout_seconds
    try:
        print('Waiting for completed Blender output: ' + str(cycles), flush=True)
        while True:
            try:
                blender = json.loads((cycles / 'validation.json').read_text())
            except (FileNotFoundError, json.JSONDecodeError):
                blender = None
            if blender is not None:
                if blender.get('source_sha256') != status['source_sha256']:
                    raise ValueError('Blender output source hash differs from current pack source')
                if blender.get('geometry_revision') != 'v10-smooth-canopy-attached-latch':
                    raise ValueError('Requires V10 smooth canopy and attached latch output')
                break
            if not args.wait or time.monotonic() >= deadline:
                raise FileNotFoundError('Completed V10 validation.json is not available')
            time.sleep(5)
        status['blender'] = blender
        status['pending'].remove('Blender output')
        env = os.environ.copy()
        env.setdefault('CARGO_TARGET_DIR', '/var/mnt/inland/builds/cargo-target/flightcraft/fc-plane')
        env.setdefault('CARGO_BUILD_JOBS', '4')
        env.setdefault('CCACHE_DIR', '/tmp/fc-canopy-ccache')
        env.setdefault('PKG_CONFIG_PATH', '/home/ray/builds/pkgconfig-shim')
        env['FLIGHTCRAFT_SLING_PACK'] = str(pack)
        env['FLIGHTCRAFT_CYCLES_OUTPUT'] = str(cycles)
        env['FLIGHTCRAFT_EXTERIOR_PROOF'] = str(output)
        env.pop('FLIGHTCRAFT_PROOF_REGISTRATION', None)
        # The inherited desktop runtime can be read-only in a confined job.
        # Offscreen proofs use their own writable, private runtime directory.
        env['XDG_RUNTIME_DIR'] = '/tmp/fc-plane-runtime'
        runtime = Path(env['XDG_RUNTIME_DIR'])
        runtime.mkdir(parents=True, exist_ok=True)
        runtime.chmod(0o700)
        driver = Path('/usr/share/vulkan/icd.d/lvp_icd.x86_64.json')
        if driver.is_file():
            env.setdefault('VK_DRIVER_FILES', str(driver))

        def run(phase, command):
            status['status'] = phase
            write_json(receipt_path, status)
            log = prefix.with_name(prefix.name + '-' + phase + '.log')
            print(phase + ': ' + str(log), flush=True)
            with log.open('w') as stream:
                subprocess.run(command, cwd=root, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True)
            status['phases'].append({'phase': phase, 'log': str(log), 'passed': True})

        def render_test(name):
            return ['cargo', '+1.98.0', 'test', '--offline', '-j', '4', '-p', 'flightcraft-render',
                    '--lib', name, '--', '--ignored', '--nocapture']

        run('install', [sys.executable, str(pack / 'scripts/install-blender-exterior.py'), str(cycles)])
        status['pending'].remove('validated install')
        run('motion-check', render_test('sling_canopy_moves_only_its_descendants_at_every_lod'))
        run('matched-cameras', render_test('matched_sling_exterior_cameras'))
        comparison = prefix.with_name(prefix.name + '-comparison.html')
        run('comparison', [sys.executable, str(pack / 'scripts/compare-exterior.py'), str(cycles), str(output), str(comparison)])
        status['pending'].remove('nine engine views')
        run('motion-sheet', render_test('sling_exterior_motion_proof'))
        motion = root / 'out/plane/motion'
        frames = json.loads((motion / 'manifest.json').read_text())['frames']
        canopy = [f for f in frames if f['file'].startswith('canopy-')]
        if len(frames) != 68 or len(canopy) != 8 or canopy[0]['canopy_open_fraction'] != 0 or canopy[-1]['canopy_open_fraction'] != 1:
            raise ValueError('Motion proof must include eight closed-to-open canopy frames, 68 total')
        cameras = json.loads((cycles / 'cameras.json').read_text())
        if len(cameras) != 9 or json.loads((output / 'cameras.json').read_text()) != cameras:
            raise ValueError('Nine exact matched closed/open cameras required')
        pictures = [output / ('engine-' + name + '.png') for name in cameras]
        pictures += [cycles / ('cycles-' + name + '.png') for name in cameras]
        pictures += [motion / 'contact-sheet.png'] + [motion / f['file'] for f in frames]
        status['images'] = [{'path': str(p), 'sha256': sha256(p)} for p in pictures]
        status['motion_manifest'] = str(motion / 'manifest.json')
        status['motion_contact_sheet'] = str(motion / 'contact-sheet.png')
        status['comparison'] = str(comparison)
        status['pending'].remove('68-frame motion sheet')
        status['installed'] = json.loads((pack / 'docs/exterior-blender-validation.json').read_text())
        status['engine_changed_files'] = changed_files(root)
        status['pack_changed_files'] = changed_files(pack)
        changed = prefix.with_name(prefix.name + '-changed-files.txt')
        separate_lists = {}
        for name, worktree, entries in (
                ('engine', root, status['engine_changed_files']), ('pack', pack, status['pack_changed_files'])):
            path = prefix.with_name(prefix.name + '-' + name + '-changed-files.txt')
            path.write_text(str(worktree) + '\n' + '\n'.join(i['status'] + ' ' + i['path'] for i in entries) + '\n')
            separate_lists[name] = str(path)
        changed.write_text('\n\n'.join(str(worktree) + '\n' + '\n'.join(i['status'] + ' ' + i['path'] for i in entries)
                                      for worktree, entries in ((root, status['engine_changed_files']), (pack, status['pack_changed_files']))) + '\n')
        status['changed_file_list'] = str(changed)
        status['changed_file_lists'] = separate_lists
        status['status'] = 'installed_and_captured'
        write_json(receipt_path, status)
        print('Installed and captured. Receipt: ' + str(receipt_path), flush=True)
    except Exception as error:
        status['status'] = 'failed'
        status['error'] = str(error)
        status['engine_changed_files'] = changed_files(root)
        status['pack_changed_files'] = changed_files(pack)
        write_json(receipt_path, status)
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
