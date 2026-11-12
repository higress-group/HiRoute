#!/usr/bin/env python3
"""Hosted prerequisite regressions without root access or package downloads."""
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]


class ActionTests(unittest.TestCase):
    def run_native_setup(self, missing=None):
        action = (ROOT / '.github/actions/ci-setup/action.yml').read_text()
        step = action.split('name: Native build prerequisites\n', 1)[1]
        command = step.split('      run: ', 1)[1].splitlines()[0]
        with tempfile.TemporaryDirectory() as temp:
            bin_dir = Path(temp)
            (bin_dir / 'python3').symlink_to(sys.executable)
            commands = {
                'cmake': 'exit 0',
                'clang': 'exit 0',
                'pkg-config': 'test "$2" != openssl' if missing == 'openssl' else 'exit 0',
                'sudo': 'echo "package mirror unavailable" >&2; exit 100',
            }
            for name, script in commands.items():
                if name != missing:
                    path = bin_dir / name
                    path.write_text(f'#!/bin/sh\n{script}\n')
                    path.chmod(0o755)
            return subprocess.run(['/bin/bash', '-e', '-o', 'pipefail', '-c', command],
                                  cwd=ROOT, env={'PATH': temp}, capture_output=True,
                                  text=True, timeout=15)

    def test_composite_action_uses_the_bounded_installer(self):
        action = (ROOT / '.github/actions/ci-setup/action.yml').read_text()
        self.assertIn('sudo python3 scripts/ci-prerequisites.py', action)
        self.assertNotIn('run: sudo apt-get update', action)

    def test_preinstalled_tools_do_not_require_package_mirrors(self):
        result = self.run_native_setup()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_missing_prerequisite_preserves_install_failure(self):
        for missing in ('cmake', 'clang', 'pkg-config', 'openssl'):
            with self.subTest(missing=missing):
                result = self.run_native_setup(missing)
                self.assertEqual(result.returncode, 100, result.stderr)
                self.assertIn('package mirror unavailable', result.stderr)


class PrerequisiteTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('ci_prerequisites', ROOT / 'scripts/ci-prerequisites.py')
        self.ci = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.ci)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.apt = Path(self.temp.name)
        (self.apt / 'sources.list.d').mkdir()

    def source(self, relative, text):
        path = self.apt / relative
        path.write_text(text)
        return path

    def test_mirror_indirection_uses_official_https_and_preserves_admission(self):
        source = self.source('sources.list.d/ubuntu.sources',
                             'Types: deb\nURIs: mirror+file:/etc/apt/apt-mirrors.txt\n'
                             'Suites: noble noble-updates noble-backports\nComponents: main universe\n'
                             'Architectures: amd64\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n\n'
                             'Types: deb\nURIs: mirror+file:/etc/apt/apt-security-mirrors.txt\n'
                             'Suites: noble-security\nComponents: main universe\n'
                             'Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n')
        before = source.read_text()
        self.ci.configure_ubuntu_sources(self.apt)
        expected = before.replace('mirror+file:/etc/apt/apt-mirrors.txt', 'https://archive.ubuntu.com/ubuntu')
        expected = expected.replace('mirror+file:/etc/apt/apt-security-mirrors.txt', 'https://security.ubuntu.com/ubuntu')
        self.assertEqual(source.read_text(), expected)

    def test_direct_mirrors_work_in_both_source_formats(self):
        for relative, text in (
            ('sources.list', 'deb [arch=amd64] http://azure.archive.ubuntu.com/ubuntu/ noble main\n'),
            ('sources.list.d/ubuntu.list', 'deb https://azure.archive.ubuntu.com/ubuntu noble-updates main\n'),
            ('sources.list.d/ubuntu.sources', 'URIs: http://archive.ubuntu.com/ubuntu http://security.ubuntu.com/ubuntu\n'),
        ):
            with self.subTest(relative=relative):
                source = self.source(relative, text)
                self.ci.configure_ubuntu_sources(self.apt)
                self.assertNotIn('azure.archive', source.read_text())
                self.assertNotIn('http://', source.read_text())
                self.assertEqual(source.read_text().replace('https://archive.ubuntu.com/ubuntu', 'UBUNTU')
                                 .replace('https://security.ubuntu.com/ubuntu', 'UBUNTU'),
                                 text.replace('http://azure.archive.ubuntu.com/ubuntu', 'UBUNTU')
                                 .replace('https://azure.archive.ubuntu.com/ubuntu', 'UBUNTU')
                                 .replace('http://archive.ubuntu.com/ubuntu', 'UBUNTU')
                                 .replace('http://security.ubuntu.com/ubuntu', 'UBUNTU'))

    def test_unrelated_sources_and_backup_files_remain_unchanged(self):
        files = [self.source(name, text) for name, text in (
            ('sources.list.d/third-party.sources', 'URIs: https://packages.example.invalid/ubuntu\nSigned-By: vendor.gpg\n'),
            ('sources.list.d/ubuntu.list.save', 'deb http://azure.archive.ubuntu.com/ubuntu noble main\n'),
        )]
        before = [path.read_bytes() for path in files]
        self.ci.configure_ubuntu_sources(self.apt)
        self.assertEqual([path.read_bytes() for path in files], before)

    def test_configuration_is_idempotent(self):
        source = self.source('sources.list', 'deb http://azure.archive.ubuntu.com/ubuntu noble main\n')
        self.ci.configure_ubuntu_sources(self.apt)
        with patch.object(Path, 'write_text', side_effect=AssertionError('unchanged source was rewritten')):
            self.ci.configure_ubuntu_sources(self.apt)
        self.assertIn('https://archive.ubuntu.com/ubuntu', source.read_text())

    def test_install_preserves_packages_and_enforces_network_and_stage_bounds(self):
        with patch.object(self.ci.subprocess, 'run') as run:
            self.ci.install_prerequisites(self.apt)
        update, install = [call.args[0] for call in run.call_args_list]
        self.assertEqual(update[:3], ['timeout', '--kill-after=10s', '180s'])
        self.assertEqual(install[:3], ['timeout', '--kill-after=10s', '300s'])
        self.assertIn('APT::Update::Error-Mode=any', update)
        self.assertIn('Acquire::Languages=none', update)
        self.assertEqual(install[-6:], ['install', '--yes', 'cmake', 'clang', 'pkg-config', 'libssl-dev'])
        for call in run.call_args_list:
            self.assertTrue(call.kwargs['check'])
            self.assertIn('Acquire::http::Timeout=30', call.args[0])
            self.assertIn('Acquire::https::Timeout=30', call.args[0])
            self.assertIn('Acquire::Retries=2', call.args[0])
            self.assertEqual(call.kwargs['env']['DEBIAN_FRONTEND'], 'noninteractive')
            self.assertEqual(call.kwargs['env']['PATH'], os.environ['PATH'])

    def test_failed_update_retries_before_installing(self):
        for exit_code in (100, 124):
            with self.subTest(exit_code=exit_code):
                failure = subprocess.CalledProcessError(exit_code, 'apt-get')
                with patch.object(self.ci.subprocess, 'run', side_effect=[failure, None, None]) as run:
                    self.ci.install_prerequisites(self.apt)
                first, retry, install = [call.args[0] for call in run.call_args_list]
                self.assertEqual(first[-1], 'update')
                self.assertEqual(retry, first)
                self.assertIn('install', install)

    def test_persistent_update_failure_does_not_install_or_succeed(self):
        for exit_code in (100, 124):
            with self.subTest(exit_code=exit_code):
                failure = subprocess.CalledProcessError(exit_code, 'apt-get')
                with patch.object(self.ci.subprocess, 'run', side_effect=failure) as run:
                    with self.assertRaises(subprocess.CalledProcessError):
                        self.ci.install_prerequisites(self.apt)
                self.assertEqual(run.call_count, 2)
                self.assertTrue(all(call.args[0][-1] == 'update' for call in run.call_args_list))

    def test_failed_install_is_not_waived(self):
        failure = subprocess.CalledProcessError(100, 'apt-get')
        with patch.object(self.ci.subprocess, 'run', side_effect=[None, failure]) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                self.ci.install_prerequisites(self.apt)
        self.assertEqual(run.call_count, 2)


if __name__ == '__main__':
    unittest.main()
