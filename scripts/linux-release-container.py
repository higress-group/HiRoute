#!/usr/bin/env python3
"""Native container build and separate minimum-runtime acceptance for Linux releases."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile

REPO = Path(__file__).resolve().parent.parent
PROXY_NAMES = ('HTTP_PROXY', 'HTTPS_PROXY', 'FTP_PROXY', 'ALL_PROXY', 'NO_PROXY',
               'http_proxy', 'https_proxy', 'ftp_proxy', 'all_proxy', 'no_proxy')


def run(target, source_repo):
    policy = json.loads((REPO / 'scripts/linux-release-baseline.json').read_text())
    arch = {'arm64': 'aarch64'}.get(platform.machine(), platform.machine())
    if platform.system() != 'Linux' or target != f'{arch}-unknown-linux-gnu':
        raise ValueError('Linux release requires a native architecture host')
    if any(key in os.environ for key in ('CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET_DIR')):
        raise ValueError('external Cargo targets are forbidden')
    if not shutil.which('docker'):
        raise ValueError('Linux release requires Docker for the pinned glibc 2.31 build and runtime images')
    sccache_executable = shutil.which('sccache')
    if not sccache_executable:
        raise ValueError('Linux release requires sccache on the host PATH')
    jobs = os.environ.get('CARGO_BUILD_JOBS', '2')
    if not jobs.isdecimal() or int(jobs) < 1:
        raise ValueError('CARGO_BUILD_JOBS must be a positive integer')
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    if subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=normal'], cwd=REPO, text=True):
        raise ValueError('build requires a clean committed candidate')
    root = REPO / 'target/release-candidate' / revision / target
    root.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix='container-', dir=root))
    result = {'revision': revision, 'target': target, 'status': 'failed', 'baseline': policy,
              'execution_host': {'system': platform.system(), 'architecture': platform.machine(), 'libc': platform.libc_ver()}}
    proxy_names = [name for name in PROXY_NAMES if name in os.environ]
    result['forwarded_build_proxy_names'] = proxy_names

    def command(args):
        with (output / 'build.log').open('a') as log:
            completed = subprocess.run(list(map(str, args)), cwd=REPO, stdout=log, stderr=subprocess.STDOUT)
        completed.check_returncode()

    try:
        dockerfile = REPO / 'scripts/linux-release.Dockerfile'
        result['dockerfile_sha256'] = hashlib.sha256(dockerfile.read_bytes()).hexdigest()
        recipe = hashlib.sha256(dockerfile.read_bytes() + json.dumps(policy, sort_keys=True).encode()).hexdigest()
        image = f'hiroute-release-{target}:{recipe[:16]}'
        docker_build = ['docker', 'build', '--platform', policy['targets'][target]['platform'],
                        '--provenance=false', '--network', 'host',
                        '--build-arg', 'BASE_IMAGE=' + policy['build_image'],
                        '--build-arg', 'APT_SNAPSHOT=' + policy['apt_snapshot']]
        # Docker resolves each value from its inherited environment. Standard
        # proxy build args are excluded from image history/cache provenance.
        for name in proxy_names:
            docker_build.extend(['--build-arg', name])
        command(docker_build + ['-f', dockerfile, '-t', image, REPO / 'scripts'])
        result['build_image_id'] = subprocess.check_output(['docker', 'image', 'inspect', '--format', '{{.Id}}', image], text=True).strip()
        if not re.fullmatch(r'sha256:[a-f0-9]{64}', result['build_image_id']):
            raise ValueError('Docker returned an invalid build image digest')
        cargo = Path(os.environ.get('CARGO_HOME', Path.home() / '.cargo')).resolve()
        rustup = Path(os.environ.get('RUSTUP_HOME', Path.home() / '.rustup')).resolve()
        go = Path(subprocess.check_output(['go', 'env', 'GOROOT'], text=True).strip()).resolve()
        sccache = Path(sccache_executable).resolve(strict=True)
        git_dir = Path(subprocess.check_output(['git', 'rev-parse', '--path-format=absolute', '--git-common-dir'], cwd=REPO, text=True).strip())
        source_repo = source_repo.resolve(strict=True)
        cpa_git = Path(subprocess.check_output(['git', '-C', str(source_repo), 'rev-parse', '--path-format=absolute', '--git-common-dir'], text=True).strip())
        cache = Path.home() / '.cache/hiroute/release-sccache'
        cache.mkdir(parents=True, exist_ok=True)
        common = ['docker', 'run', '--rm', '--init', '--platform', policy['targets'][target]['platform'],
                  '--user', f'{os.getuid()}:{os.getgid()}']
        build = common + ['--network', 'host'] + [value for name in proxy_names for value in ('-e', name)] + [
                          '-w', str(REPO), '-v', f'{REPO}:{REPO}', '-v', f'{git_dir}:{git_dir}:ro',
                          '-v', f'{source_repo}:{source_repo}:ro', '-v', f'{cpa_git}:{cpa_git}:ro',
                          '-v', f'{cargo}:/opt/cargo', '-v', f'{rustup}:/opt/rustup:ro',
                          '-v', f'{go}:/opt/go:ro', '-v', f'{sccache}:/usr/local/bin/sccache:ro',
                          '-v', f'{cache}:/sccache', '-e', 'SCCACHE_DIR=/sccache',
                          '-e', 'SCCACHE_SERVER_UDS=/tmp/hiroute-release-sccache.sock',
                          '-e', 'HOME=/tmp/hiroute-build-home', '-e', 'CARGO_HOME=/opt/cargo',
                          '-e', 'RUSTUP_HOME=/opt/rustup', '-e', 'CARGO_INCREMENTAL=0',
                          '-e', 'CARGO_BUILD_JOBS=' + jobs, '-e', 'CARGO_NET_GIT_FETCH_WITH_CLI=true',
                          '-e', 'PATH=/usr/local/bin:/opt/cargo/bin:/opt/go/bin:/usr/bin:/bin',
                          '-e', 'HIROUTE_LINUX_BASELINE_IMAGE=' + policy['build_image'],
                          '-e', 'HIROUTE_RELEASE_RESULT=' + str(output / 'package.json'),
                          result['build_image_id'], 'python3', 'scripts/build-release.py',
                          '--target', target, '--cpa-source-repo', source_repo]
        spec = importlib.util.spec_from_file_location('local_rust', REPO / 'scripts/local-rust.py')
        local_rust = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(local_rust)
        # Use the caller's existing lock even though the build has a private HOME.
        with local_rust.Store().locked(REPO):
            stamp = REPO / 'target/.linux-release-baseline.json'
            # Automatic attestations regenerate containerd image IDs. Build
            # without those attestations and retain our measured provenance.
            identity = {'schema': 'hiroute.linux-release-target-baseline/v1',
                        'image_digest': result['build_image_id'], 'recipe_sha256': recipe, 'target': target}
            if stamp.exists():
                if json.loads(stamp.read_text()) != identity:
                    raise ValueError('checkout target belongs to a different Linux build baseline; use a fresh worktree')
            elif any((REPO / 'target' / path).exists() for path in ('release', target + '/release')):
                raise ValueError('unattested release outputs already exist; use a fresh worktree')
            stamp.write_text(json.dumps(identity) + '\n')
            command(build)
        package_path = Path(json.loads((output / 'package.json').read_text())['result_path'])
        package = json.loads(package_path.read_text())
        if package['status'] != 'awaiting_runtime' or package['revision'] != revision:
            raise ValueError('container package identity/status mismatch')
        assets = Path(package['assets'])
        manifest = assets / package['website_artifact']['manifest_filename']
        archive = assets / package['website_artifact']['filename']
        runtime = output / 'runtime'
        runtime.mkdir()
        inputs = output / 'runtime-input'
        inputs.mkdir()
        for name in ('linux-release-runtime.py', 'install-standalone.py', 'package-standalone.py',
                     'linux-release-baseline.json'):
            shutil.copyfile(REPO / 'scripts' / name, inputs / name)
        for path in (manifest, archive, package_path.parent / 'linux-abi.json'):
            shutil.copyfile(path, inputs / path.name)
        command(common + ['--network', 'none', '-w', '/input', '-v', f'{inputs}:/input:ro',
                          '-v', f'{runtime}:/output', policy['runtime_image'],
                          'python3', '/input/linux-release-runtime.py', '--manifest', '/input/' + manifest.name,
                          '--archive', '/input/' + archive.name, '--abi', '/input/linux-abi.json',
                          '--output', '/output'])
        evidence = json.loads((runtime / 'result.json').read_text())
        if evidence['status'] != 'green' or evidence['archive_sha256'] != package['website_artifact']['sha256']:
            raise ValueError('minimum runtime did not accept the exact archive')
        result.update(package, status='completed', minimum_runtime=evidence,
                      baseline=policy, package_result=str(package_path))
        package.update(status='completed', minimum_runtime=evidence, build_environment=result['build_image_id'])
        package_path.write_text(json.dumps(package, indent=2) + '\n')
        return result
    except Exception as error:
        result['error'] = str(error)
        raise
    finally:
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(f'Linux container evidence: {output}', file=__import__('sys').stderr)
