#!/usr/bin/env python3
"""Check hosted Ubuntu build prerequisites or install them with bounded network operations."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys


def prerequisites_available():
    for command in (('cmake', '--version'), ('clang', '--version'), ('pkg-config', '--exists', 'openssl')):
        try:
            subprocess.run(command, check=True, timeout=10,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except (OSError, subprocess.SubprocessError):
            print(f'Native prerequisite check failed: {" ".join(command)}', flush=True)
            return False
    print('Native prerequisites: using preinstalled build tools', flush=True)
    return True


def configure_ubuntu_sources(apt_root):
    # Runner mirror lists can retry Azure indefinitely before reaching Ubuntu.
    # Change only known Ubuntu URIs; retain suites, components and signing keys.
    replacements = (
        (r'mirror\+file:/etc/apt/apt-mirrors\.txt(?=\s|$)', 'https://archive.ubuntu.com/ubuntu'),
        (r'mirror\+file:/etc/apt/apt-security-mirrors\.txt(?=\s|$)', 'https://security.ubuntu.com/ubuntu'),
        (r'https?://azure\.archive\.ubuntu\.com/ubuntu(?=[/\s]|$)', 'https://archive.ubuntu.com/ubuntu'),
        (r'http://archive\.ubuntu\.com/ubuntu(?=[/\s]|$)', 'https://archive.ubuntu.com/ubuntu'),
        (r'http://security\.ubuntu\.com/ubuntu(?=[/\s]|$)', 'https://security.ubuntu.com/ubuntu'),
    )
    sources = [apt_root / 'sources.list']
    sources.extend(sorted((apt_root / 'sources.list.d').glob('*.list')))
    sources.extend(sorted((apt_root / 'sources.list.d').glob('*.sources')))
    for source in sources:
        if not source.is_file():
            continue
        before = source.read_text()
        after = before
        for pattern, replacement in replacements:
            after = re.sub(pattern, replacement, after)
        if after != before:
            source.write_text(after)
            print(f'Using official Ubuntu HTTPS sources in {source.name}', flush=True)


def run_apt_stage(name, limit, command):
    print(f'Native prerequisites: {name} (limit {limit})', flush=True)
    # GNU timeout bounds the entire apt process group, including downloads.
    subprocess.run(['timeout', '--kill-after=10s', limit, *command],
                   check=True, env={**os.environ, 'DEBIAN_FRONTEND': 'noninteractive'})


def install_prerequisites(apt_root=Path('/etc/apt')):
    configure_ubuntu_sources(apt_root)
    apt = ['apt-get', '-o', 'Acquire::Retries=2', '-o', 'Acquire::http::Timeout=30',
           '-o', 'Acquire::https::Timeout=30', '-o', 'DPkg::Lock::Timeout=60']
    update = [*apt, '-o', 'APT::Update::Error-Mode=any', '-o', 'Acquire::Languages=none', 'update']
    try:
        run_apt_stage('update', '180s', update)
    except subprocess.CalledProcessError as error:
        if error.returncode not in (100, 124):  # apt-get error or GNU timeout
            raise
        print(f'Native prerequisite update failed (exit {error.returncode}). Retrying once.', flush=True)
        run_apt_stage('update retry', '180s', update)
    run_apt_stage('install', '300s', [*apt, 'install', '--yes', 'cmake', 'clang', 'pkg-config', 'libssl-dev'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='check build tools without running APT')
    if parser.parse_args().check:
        sys.exit(0 if prerequisites_available() else 1)
    try:
        install_prerequisites()
    except subprocess.CalledProcessError as error:
        print(f'Native prerequisite stage failed (exit {error.returncode})', file=sys.stderr)
        sys.exit(1)
