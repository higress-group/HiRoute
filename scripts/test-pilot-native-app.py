#!/usr/bin/env python3
"""Real local process/entry regressions with fixture executables, not macOS CU acceptance."""
import hashlib
import importlib.util
import json
import os
import platform
import sys
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import types
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('native', Path(__file__).with_name('pilot-native-app.py'))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)
pilot = native.pilot

FIXTURE = r'''
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
volatile sig_atomic_t stopping = 0;
void stop(int sig) { stopping = 1; }
int main(int argc, char **argv) {
    signal(SIGTERM, stop);
    if (argc > 1) { while (!stopping) pause(); return 0; }
    char sibling[4096]; snprintf(sibling, sizeof(sibling), "%s", argv[0]);
    char *slash = strrchr(sibling, '/'); strcpy(slash + 1, "hirouted");
    pid_t child = fork();
    if (!child) { execl(sibling, sibling, "--daemon", NULL); _exit(2); }
    char evidence[4096]; snprintf(evidence, sizeof(evidence), "%s/observed", getenv("HIROUTE_DESKTOP_TEST_ROOT"));
    FILE *out = fopen(evidence, "a");
    fprintf(out, "%s\n%s\n%s\n%s\n", getenv("HOME"), getenv("HIROUTE_DESKTOP_TEST_ROOT"),
            getenv("CODEX_HOME"), getenv("OPENAI_API_KEY") ? "credential-leaked" : "no-credential");
    fclose(out);
    while (!stopping) pause();
    kill(child, SIGTERM); waitpid(child, NULL, 0); return 0;
}
'''


@unittest.skipUnless(shutil.which('cc'), 'a C compiler is required for real fixture process identity')
class NativeEntry(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.compiler_temp = tempfile.TemporaryDirectory()
        cls.binary = Path(cls.compiler_temp.name) / 'fixture'
        subprocess.run(['cc', '-x', 'c', '-', '-o', str(cls.binary)], input=FIXTURE, text=True, check=True)

    @classmethod
    def tearDownClass(cls):
        cls.compiler_temp.cleanup()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.account = self.base / 'account'
        self.account.mkdir(mode=0o700)
        self.state = self.account / '.cache/hiroute/local-rust'
        self.run = 'b' * 32
        checkout = self.state / 'checkouts' / self.run
        debug = checkout / 'target/debug'
        debug.mkdir(parents=True)
        result_dir = self.state / 'runs' / self.run
        result_dir.mkdir(parents=True)
        for directory in (self.state, self.state / 'runs', self.state / 'checkouts', result_dir):
            directory.chmod(0o700)
        configs = checkout / 'apps/desktop/src-tauri'
        configs.mkdir(parents=True)
        (configs / 'tauri.conf.json').write_text(json.dumps({'app': {'windows': [{'label': 'main'}]}}))
        overlay = {'app': {'windows': [{'label': 'main', 'incognito': True}]}}
        (configs / 'tauri.pilot.conf.json').write_text(json.dumps({'app': {}}))
        overlay['build'] = {'frontendDist': str(checkout / 'apps/desktop/dist')}
        (checkout / '.gitignore').write_text('target/\n')
        def git(*args):
            return subprocess.check_output(['git', '-C', str(checkout), *args], text=True).strip()
        git('init', '-q')
        git('config', 'user.name', 'Fixture')
        git('config', 'user.email', 'fixture@example.invalid')
        git('config', 'core.hooksPath', '/dev/null')
        git('add', '.')
        git('commit', '-qm', 'fixture')
        sha = git('rev-parse', 'HEAD')
        for name in ('hiroute-desktop', 'hirouted', 'hiroute'):
            shutil.copyfile(self.binary, debug / name)
            (debug / name).chmod(0o700)
        self.root = self.base / 'hrp-native'
        self.root.mkdir(mode=0o700)
        self.session = self.root / 'session.json'
        self.record = dict(id=self.run, kind='managed', checkout=str(checkout), sha=sha,
                           command=['cargo', 'build', '--locked', '-p', 'hiroute-desktop', '-p', 'hiroute-daemon',
                                    '-p', 'hiroute-cli', '--features', 'hiroute-desktop/desktop-pilot',
                                    '--bin', 'hiroute-desktop', '--bin', 'hirouted', '--bin', 'hiroute'],
                           status='terminal', process_exit=0, keep=True, scenario='unassessed',
                           identity=pilot.directory_identity(checkout), target_identity=pilot.directory_identity(debug.parent),
                           debug_identity=pilot.directory_identity(debug), pilot_reuse={'fixture': True},
                           pilot_frontend={'tauri_config_sha256': hashlib.sha256(json.dumps(
                               overlay, separators=(',', ':'), sort_keys=True).encode()).hexdigest()},
                           pilot_leases={'lease': {'state': 'open', 'sessions': {'case': {'session': str(self.session)}}}})
        self.record_path = result_dir / 'result.json'
        self.save_record()
        self.patch = patch.object(pilot, 'LOCAL_RUST_ROOT', self.state)
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.build = pilot.verify_managed_pilot_build(self.run, str(debug / 'hiroute-desktop'), sha)
        self.prepare_entry()
        self.processes = []
        self.addCleanup(self.cleanup_processes)

    def prepare_entry(self):
        directories = {name: pilot.make_subdirectory(self.root, name) for name in ('runtime', 'tmp', 'data', 'working', 'logs')}
        log = directories['logs'] / 'desktop.log'
        log.touch(mode=0o600)
        with patch.dict(os.environ, {'OPENAI_API_KEY': 'fixture-secret'}):
            self.session, self.row = native.prepare(
                types.SimpleNamespace(managed_lease='lease', diagnostic_level='debug'), self.build, self.root,
                directories['runtime'], directories['tmp'], directories['data'], directories['working'], log, None)

    def save_record(self):
        self.record_path.write_text(json.dumps(self.record))

    def cleanup_processes(self):
        for process in self.processes:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)
            for stream in (process.stdout, process.stderr):
                if stream is not None:
                    stream.close()

    def open_entry(self):
        environment = dict(os.environ, HOME=str(self.account), OPENAI_API_KEY='fixture-secret')
        process = subprocess.Popen([str(Path(self.row['native_app']['bundle']) / 'Contents/MacOS/pilot-launch')],
                                   env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.processes.append(process)
        return process

    def ready(self):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            row, members = native.observe(self.session)
            if len(members) == 2 and (Path(row['data_root']) / 'observed').exists():
                return row
            time.sleep(0.02)
        self.fail('Fixture process did not become ready')

    def test_plain_reopen_retains_profile_and_owner_cleanup_only_stops_this_instance(self):
        unrelated = subprocess.Popen(['sleep', '60'], start_new_session=True)
        self.processes.append(unrelated)
        first = self.open_entry()
        row = self.ready()
        self.assertEqual(row['pid'], first.pid)
        data = Path(row['data_root'])
        (data / 'retained-setting').write_text('same-profile')
        self.assertEqual((data / 'observed').read_text().splitlines(), [row['process_home'], row['data_root'],
                         str(Path(row['process_home']) / '.codex'), 'no-credential'])
        # Duplicate ordinary open cannot create a second Desktop or advance generation.
        duplicate = self.open_entry()
        self.assertEqual(duplicate.wait(timeout=10), 0)
        self.assertEqual(native.observe(self.session)[0]['native_app']['generation'], 1)
        # A normal application exit leaves the entry reusable. Observation never launches.
        os.kill(first.pid, signal.SIGTERM)
        first.wait(timeout=5)
        self.assertEqual(native.observe(self.session)[1], [])
        self.assertEqual(native.observe(self.session)[0]['native_app']['generation'], 1)
        second = self.open_entry()
        row = self.ready()
        self.assertEqual(row['pid'], second.pid)
        self.assertEqual(row['native_app']['generation'], 2)
        self.assertEqual((data / 'retained-setting').read_text(), 'same-profile')
        self.assertEqual((data / 'observed').read_text().splitlines()[-4:], [row['process_home'], row['data_root'],
                         str(Path(row['process_home']) / '.codex'), 'no-credential'])
        native.stop(self.session)
        second.wait(timeout=5)
        self.assertEqual(native.observe(self.session)[0]['native_app']['state'], 'enabled')
        third = self.open_entry()
        self.ready()
        pilot.stop_session(self.session, row, revoke=True)
        third.wait(timeout=5)
        self.assertFalse(Path(row["app"]).exists())
        self.assertTrue(Path(self.build["artifacts"]["app"]["path"]).is_file())
        refused = self.open_entry()
        self.assertEqual(refused.wait(timeout=10), 2)
        self.assertIn(b'retired', refused.stderr.read())
        self.assertIsNone(unrelated.poll())
        self.assertEqual(native.observe(self.session)[1], [])

    def test_replaced_deployed_artifact_refused_before_first_launch(self):
        artifact = Path(self.row['app'])
        artifact.unlink()
        shutil.copyfile(self.binary, artifact)
        artifact.chmod(0o700)
        refused = self.open_entry()
        self.assertEqual(refused.wait(timeout=10), 2)
        self.assertIn(b'replaced or modified', refused.stderr.read())
        self.assertEqual(native.observe(self.session)[0]['pid'], 0)

    def test_replaced_source_artifact_and_modified_wrapper_refused(self):
        for path in (Path(self.build['artifacts']['cli']['path']),
                     Path(self.row['native_app']['bundle']) / 'Contents/Info.plist'):
            with self.subTest(path=path.name):
                original = path.read_bytes()
                original_times = (path.stat().st_atime_ns, path.stat().st_mtime_ns)
                path.write_bytes(original + b'changed')
                refused = self.open_entry()
                self.assertEqual(refused.wait(timeout=10), 2)
                self.assertEqual(native.observe(self.session)[0]['pid'], 0)
                path.write_bytes(original)
                os.utime(path, ns=original_times)

    def test_two_native_apps_keep_distinct_executable_identities_and_retire_independently(self):
        first_session = self.session
        self.open_entry()
        first = self.ready()
        self.root = self.base / 'hrp-other'
        self.root.mkdir(mode=0o700)
        self.session = self.root / 'session.json'
        self.record['pilot_leases']['lease']['sessions']['other'] = {'session': str(self.session)}
        self.save_record()
        self.prepare_entry()
        for role, source in self.build['artifacts'].items():
            one, two = first['artifacts'][role], self.row['artifacts'][role]
            self.assertEqual(len({source['inode'], one['inode'], two['inode']}), 3)
            self.assertEqual(source['sha256'], one['sha256'])
            self.assertEqual(source['sha256'], two['sha256'])
        self.open_entry()
        second = self.ready()
        # On macOS this also exercises proc_pidpath after another App has opened.
        self.assertEqual(native.observe(first_session)[0]['pid'], first['pid'])
        native.stop(first_session, revoke=True)
        self.assertEqual(native.observe(self.session)[0]['pid'], second['pid'])
        self.assertTrue(native.observe(self.session)[1])
        for artifact in self.build['artifacts'].values():
            pilot.artifact_is_unchanged(artifact)
        native.stop(self.session, revoke=True)
        self.assertFalse(Path(first['app']).exists())
        self.assertFalse(Path(second['app']).exists())

    def test_staged_cpa_preserves_compiled_pin_and_is_retired_with_owned_entry(self):
        source = Path(self.build['artifacts']['daemon']['path']).with_name('cliproxyapi')
        shutil.copyfile(self.binary, source)
        source.chmod(0o700)
        artifact = pilot.executable_artifact(source)
        machine = platform.machine().lower()
        manifest = {'schema': 'hiroute.desktop.cpa-artifacts/v1', 'artifacts': [{
            'os': 'macos' if sys.platform == 'darwin' else sys.platform,
            'arch': {'arm64': 'aarch64', 'amd64': 'x86_64'}.get(machine, machine),
            'binary_name': 'cliproxyapi', 'sha256': artifact['sha256'], 'size': artifact['size']}]}
        contents = json.dumps(manifest).encode()
        self.record_path.with_name('cpa-manifest.json').write_bytes(contents)
        self.record['pilot_reuse']['recipe'] = {'cpa_manifest': {'sha256': hashlib.sha256(contents).hexdigest()}}
        self.save_record()
        shutil.rmtree(self.root)  # This test has not launched the owned fixture yet.
        self.root.mkdir(mode=0o700)
        self.prepare_entry()
        deployed = Path(self.row['artifacts']['cpa']['path'])
        self.assertNotEqual(deployed.stat().st_ino, source.stat().st_ino)
        self.assertEqual(pilot.file_sha256(deployed), artifact["sha256"])
        native.verify(self.row, self.session)
        self.record_path.with_name('cpa-manifest.json').write_bytes(contents + b' ')
        with self.assertRaisesRegex(ValueError, 'compiled build input'):
            native.verify(self.row, self.session)
        self.record_path.with_name('cpa-manifest.json').write_bytes(contents)
        native.stop(self.session, revoke=True)
        self.assertFalse(deployed.exists())
        self.assertTrue(source.is_file())
        source.write_bytes(b'replacement')
        with self.assertRaisesRegex(ValueError, 'compiled manifest hash/size'):
            native.staged_cpa(self.build)

    def test_closed_lease_or_missing_incognito_attestation_refused(self):
        for mutate in ('lease', 'config'):
            with self.subTest(mutate=mutate):
                self.record['pilot_leases']['lease']['state'] = 'closed' if mutate == 'lease' else 'open'
                if mutate == 'config':
                    self.record['pilot_frontend']['tauri_config_sha256'] = 'wrong'
                self.save_record()
                refused = self.open_entry()
                self.assertEqual(refused.wait(timeout=10), 2)
                self.assertEqual(native.observe(self.session)[0]['pid'], 0)

    def test_unknown_process_in_group_prevents_signalling(self):
        process = self.open_entry()
        row = self.ready()
        # Existing strict cleanup protects against a stale/reused process identity.
        row['process_identities'][0]['start_time'] = 'wrong'
        pilot.write_private_json(self.session, row)
        with self.assertRaisesRegex(ValueError, 'reused'):
            native.stop(self.session, revoke=True)
        self.assertIsNone(process.poll())
        row['process_identities'][0]['start_time'] = pilot.process_start_time(process.pid)
        pilot.write_private_json(self.session, row)
        native.stop(self.session, revoke=True)
        process.wait(timeout=5)


if __name__ == '__main__':
    unittest.main()
