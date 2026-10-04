"""Shared product-entry mechanics; native argv, configuration and verdicts stay in leaves."""
import os
import signal
import subprocess


def settings_status(product, context):
    return product.preview('agents connect status', {
        'schema_version': {'major': 2, 'minor': 0}, 'context_id': context})


def apply_settings(product, spec, key, facet):
    """Apply the previewed intent and read its authoritative state through the public CLI."""
    assert facet in ('model', 'collaboration'), 'unknown settings facet'
    command = 'agents restore' if spec[facet]['intent'] == 'restore' else 'agents connect'
    preview = product.preview(command + ' preview', {'spec': spec})
    assert preview['applicable'], 'settings Preview is blocked; inspect private diagnostics'
    body = {'spec': preview['spec'], 'accept_digest': preview['accept_digest'],
            'dependency_digest': preview['dependency_digest'], 'expected_revisions': preview['expected_revisions'],
            'idempotency_key': key}
    resident = preview.get('resident_service', {})
    if resident.get('login_item_required'):
        body['login_item'] = {'before': 'not_registered', 'after': 'enabled', 'created': True}
    elif resident.get('login_item_removal_required'):
        body['login_item'] = {'before': 'enabled', 'after': 'not_registered', 'created': False}
    _, applied = product.cli(command + ' apply', body)
    assert applied['data']['state'] == 'succeeded', 'settings operation did not succeed'
    return preview, settings_status(product, spec['context_id'])


def run_native_command(product, command, *, timeout, label, env=None):
    """Capture private output and bound the owned process; success still needs a native oracle."""
    process = subprocess.Popen(command, env=dict(product.env if env is None else env), cwd=product.project,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    try:
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                stdout, stderr = process.communicate(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                stdout, stderr = process.communicate(timeout=3)
            product.outputs.extend((stdout, stderr))
            raise AssertionError(f'{label} exceeded its deadline') from None
        product.outputs.extend((stdout, stderr))
        assert process.returncode == 0, f'{label} failed; inspect private diagnostics'
        return stdout
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=3)
