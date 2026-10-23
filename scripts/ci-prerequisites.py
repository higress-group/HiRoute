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


def install_prerequisites(apt_root=Path('/etc/apt')):
    configure_ubuntu_sources(apt_root)
    apt = ['apt-get', '-o', 'Acquire::Retries=2', '-o', 'Acquire::http::Timeout=30',
           '-o', 'Acquire::https::Timeout=30', '-o', 'DPkg::Lock::Timeout=60']
    stages = (
        ('update', '180s', ['-o', 'APT::Update::Error-Mode=any', 'update']),
        ('install', '300s', ['install', '--yes', 'cmake', 'clang', 'pkg-config', 'libssl-dev']),
    )
    for name, limit, arguments in stages:
        print(f'Native prerequisites: {name} (limit {limit})', flush=True)
        # GNU timeout bounds the entire apt process group, including downloads.
        subprocess.run(['timeout', '--kill-after=10s', limit, *apt, *arguments],
                       check=True, env={**os.environ, 'DEBIAN_FRONTEND': 'noninteractive'})


if __name__ == '__main__':
    try:
        install_prerequisites()
    except subprocess.CalledProcessError as error:
        print(f'Native prerequisite stage failed (exit {error.returncode})', file=sys.stderr)
        sys.exit(1)
