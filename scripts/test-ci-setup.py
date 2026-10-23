#!/usr/bin/env python3
"""Hosted prerequisite regressions without root access or package downloads."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]


class ActionTests(unittest.TestCase):
    def test_composite_action_uses_the_bounded_installer(self):
        action = (ROOT / '.github/actions/ci-setup/action.yml').read_text()
        self.assertIn('run: sudo python3 scripts/ci-prerequisites.py', action)
        self.assertNotIn('run: sudo apt-get update', action)


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
        self.assertEqual(install[-6:], ['install', '--yes', 'cmake', 'clang', 'pkg-config', 'libssl-dev'])
        for call in run.call_args_list:
            self.assertTrue(call.kwargs['check'])
            self.assertIn('Acquire::http::Timeout=30', call.args[0])
            self.assertIn('Acquire::https::Timeout=30', call.args[0])
            self.assertIn('Acquire::Retries=2', call.args[0])
            self.assertEqual(call.kwargs['env']['DEBIAN_FRONTEND'], 'noninteractive')
            self.assertEqual(call.kwargs['env']['PATH'], os.environ['PATH'])

    def test_failed_update_including_timeout_does_not_install_or_succeed(self):
        for exit_code in (100, 124):
            with self.subTest(exit_code=exit_code):
                failure = subprocess.CalledProcessError(exit_code, 'apt-get')
                with patch.object(self.ci.subprocess, 'run', side_effect=failure) as run:
                    with self.assertRaises(subprocess.CalledProcessError):
                        self.ci.install_prerequisites(self.apt)
                self.assertEqual(run.call_count, 1)

    def test_failed_install_is_not_waived(self):
        failure = subprocess.CalledProcessError(100, 'apt-get')
        with patch.object(self.ci.subprocess, 'run', side_effect=[None, failure]):
            with self.assertRaises(subprocess.CalledProcessError):
                self.ci.install_prerequisites(self.apt)


if __name__ == '__main__':
    unittest.main()
