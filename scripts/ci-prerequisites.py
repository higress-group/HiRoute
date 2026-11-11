#!/usr/bin/env python3
"""Install hosted Ubuntu build prerequisites with bounded network operations."""
import os
from pathlib import Path
import re
import subprocess
import sys


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
    try:
        install_prerequisites()
    except subprocess.CalledProcessError as error:
        print(f'Native prerequisite stage failed (exit {error.returncode})', file=sys.stderr)
        sys.exit(1)
