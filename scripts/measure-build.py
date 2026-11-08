#!/usr/bin/env python3
"""Measure an isolated offline release build without running the resulting executable."""
import argparse
import json
import hashlib
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def capture(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--template', choices=['basic', 'gateway', 'worker'], default='basic')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if args.out.exists() or args.out.is_symlink():
        parser.error('output already exists')
    if not args.out.parent.is_dir():
        parser.error('output parent must already exist')
    features = {'basic': '', 'gateway': 'gateway', 'worker': 'hot-reload'}[args.template]
    # A new target is cold for compiled Cargo artifacts; registry/OS caches may be warm.
    base = ROOT / 'target/build-measurements'
    base.mkdir(parents=True, exist_ok=True)
    target = Path(tempfile.mkdtemp(prefix=args.template + '-', dir=base))
    example = {'basic': 'ping_bot', 'gateway': 'gateway_bot', 'worker': 'hot_reload_bot'}[args.template]
    command = ['cargo', 'build', '--locked', '--offline', '--release', '--no-default-features',
               '--example', example, '--target-dir', str(target)]
    if features:
        command += ['--features', features]
    env = os.environ.copy()
    for name in ('YAPPER_BOT_TOKEN', 'YAPIN_WEBHOOK_SECRET', 'WEBHOOK_SECRET'):
        env.pop(name, None)
    samples = []
    for phase in ('fresh-target', 'reused-target'):
        started = time.perf_counter()
        subprocess.run(command, cwd=ROOT, env=env, check=True)
        samples.append({'phase': phase, 'seconds': time.perf_counter() - started})
    binary = target / 'release/examples' / (example + ('.exe' if os.name == 'nt' else ''))
    data = dict(format=1, commit=capture('git', 'rev-parse', 'HEAD'),
                dirty=bool(capture('git', 'status', '--porcelain', '--untracked-files=no')),
                rustc=capture('rustc', '-Vv'), host=platform.platform(),
                cargo_lock_sha256=hashlib.sha256((ROOT / 'Cargo.lock').read_bytes()).hexdigest(),
                template=args.template, example=example, features=features.split(',') if features else [],
                binary_bytes=binary.stat().st_size, binary=str(binary.relative_to(ROOT)),
                build_command=command, samples=samples,
                limitations='Local build only; OS/registry caches may be warm. No executable or API traffic was run. Build scripts/macros are trusted executable code.')
    # Do not overwrite an existing result, including one created during the measurement.
    with args.out.open('x') as stream:
        json.dump(data, stream, indent=2)
        stream.write('\n')
    print(json.dumps({key: data[key] for key in ('template', 'binary_bytes', 'samples')}))


if __name__ == '__main__':
    main()
