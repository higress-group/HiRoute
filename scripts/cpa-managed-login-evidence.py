#!/usr/bin/env python3
"""Closed, read-only evidence for the private managed-login acceptance harness.

This module does not read provider credentials, write files, launch processes or
manage authentication. Client output remains in memory; only enumerated facts
and numbers may leave its projection. Observation uses the production read API.
"""
import hashlib
import json
import re
import time


class EvidenceFailure(Exception):
    pass


ERROR_VALUES = frozenset((
    'invalid_request_error', 'invalid_request', 'input_rejected', 'bad_request',
    'authentication_error', 'permission_error', 'not_found_error', 'model_not_found',
    'rate_limit_error', 'api_error', 'overloaded_error', 'server_error',
    'unsupported_parameter', 'insufficient_quota', 'non_json', 'source_unavailable',
    'SOURCE_UNAVAILABLE', 'ROUTE_UNAVAILABLE', 'PROVIDER_REQUEST_REJECTED',
    'UPSTREAM_ERROR', 'INTERNAL_ERROR', 'INVALID_ARGUMENTS', 'DAEMON_UNAVAILABLE',
    'CAPABILITY_DENIED', 'CAPABILITY_UNAVAILABLE', 'RESOURCE_NOT_FOUND',
    'SCHEMA_INCOMPATIBLE', 'AGENT_AUTH_PRECEDENCE_CONFLICT', 'INTERNAL',
))
PARAMETERS = frozenset((
    'thinking', 'thinking.type', 'thinking.budget_tokens', 'output_config',
    'output_config.effort', 'effort', 'context_management', 'max_tokens',
    'max_output_tokens', 'model', 'tools', 'tool_choice', 'messages', 'system',
    'metadata', 'stream', 'reasoning', 'reasoning.effort', 'temperature', 'top_p',
    'anthropic-beta',
))
PHASES = frozenset(('admission', 'prepare', 'request', 'response', 'dispatch', 'upstream'))
CLIENT_TYPES = frozenset(('result', 'error', 'turn.failed', 'turn.completed',
                          'item.completed', 'system', 'assistant', 'user', 'stream_event'))
CLIENT_SUBTYPES = frozenset(('success', 'error_during_execution', 'error_max_turns',
                             'error_max_budget_usd', 'error_max_structured_output_retries'))
STAGES = frozenset(('request', 'start', 'status', 'callback', 'cancel', 'forget', 'stop', 'lifecycle',
    'retarget', 'expiry', 'smoke', 'native', 'daemon-start', 'restart', 'observation',
    'observation-baseline', 'credentials', 'check-save-publish-connect',
    'fresh-check-save-publish-connect', 'managed-nonstream', 'managed-stream',
    'real-native-client-dialogue', 'managed-disable', 'managed-disabled-rejection',
    'managed-reenable', 'managed-after-restart', 'initial-stream', 'initial-nonstream',
    'disable', 'reenable', 'restart-restore', 'after-restart', 'disabled-rejection',
    'original-expiry-crossing', 'forgotten-rejection'))
METRICS = ('input', 'output', 'cache_read')
MACHINE_STATUSES = frozenset(('succeeded', 'accepted', 'usage_error', 'conflict', 'denied',
    'not_found', 'unavailable', 'action_required', 'needs_attention', 'internal_error'))


def require(condition, code):
    if not condition:
        raise EvidenceFailure(code)


def closed(value, choices):
    return value if isinstance(value, str) and value in choices else 'other'


def stage(value):
    if isinstance(value, str) and value in STAGES:
        return value
    if isinstance(value, str):
        for provider in ('claude', 'codex'):
            if value.startswith(provider + '-') and value[len(provider) + 1:] in STAGES:
                return value
    return 'unknown'


def _events(raw):
    events = []
    for line in raw[-2 * 1024 * 1024:].splitlines():
        if len(line) > 262144:
            continue
        try:
            event = json.loads(line)
        except (ValueError, RecursionError):
            continue
        if isinstance(event, dict):
            events.append(event)
    return events[-2048:]


def native_success(raw, provider, marker):
    """Project success facts without exporting answer text or arbitrary usage fields."""
    events = _events(raw)
    terminal_type = 'turn.completed' if provider == 'codex' else 'result'
    terminal = next((event for event in reversed(events)
                     if event.get('type') == terminal_type), None)
    failed = any(event.get('type') in ('turn.failed', 'error') for event in events)
    if provider == 'codex':
        answers = [event['item'].get('text') for event in events
                   if event.get('type') == 'item.completed' and isinstance(event.get('item'), dict)
                   and event['item'].get('type') == 'agent_message']
    else:
        failed = failed or terminal is None or terminal.get('is_error') is not False
        answers = [terminal.get('result')] if terminal else []
    usage = (terminal or {}).get('usage') or {}
    usage = usage if isinstance(usage, dict) else {}
    projected = {key: value for key, value in usage.items()
                 if key in ('input_tokens', 'output_tokens', 'cache_read_input_tokens',
                            'cache_creation_input_tokens') and type(value) is int and value >= 0}
    details = usage.get('input_tokens_details')
    cached = (details.get('cached_tokens') if isinstance(details, dict) else None)
    if provider == 'codex' and type(usage.get('cached_input_tokens')) is int:
        cached = usage['cached_input_tokens']
    if type(cached) is int and cached >= 0:
        projected['input_tokens_details'] = {'cached_tokens': cached}
    return {'terminal_present': terminal is not None, 'turn_failed': failed,
            'answer_verified': any(isinstance(text, str) and text.strip().strip('.').upper() == marker
                                   for text in answers), 'usage': projected}


def native_projection(stdout, stderr, provider, exit_code):
    """Never retain raw errors; explicit parameters and mere mentions stay separate."""
    events = _events(stdout)
    statuses, codes, parameters, mentions, categories = set(), set(), set(), set(), set()
    budget = [128]

    def visit(value, depth=0, error_object=False):
        if depth > 8 or budget[0] <= 0:
            return
        budget[0] -= 1
        if isinstance(value, dict):
            if error_object:
                for key in ('code', 'type'):
                    if isinstance(value.get(key), str):
                        codes.add(closed(value[key], ERROR_VALUES))
                for key in ('param', 'parameter'):
                    if value.get(key) is not None:
                        parameters.add(closed(value[key], PARAMETERS))
            for key in ('status', 'status_code', 'http_status'):
                if type(value.get(key)) is int and 400 <= value[key] <= 599:
                    statuses.add(value[key])
            for key in ('error', 'errors', 'message', 'result'):
                if key in value:
                    visit(value[key], depth + 1, key in ('error', 'errors') or error_object)
            if error_object:
                for key in ('content', 'text'):
                    if key in value:
                        visit(value[key], depth + 1, True)
        elif isinstance(value, list):
            for item in value[:16]:
                visit(item, depth + 1, error_object)
        elif isinstance(value, str):
            text = value[:65536]
            statuses.update(int(match) for match in re.findall(
                r'(?i)(?:API Error|HTTP(?: Error)?|status(?:_code)?)[\s:=]+([45][0-9]{2})\b', text))
            lower = text.lower()
            mentions.update(parameter for parameter in PARAMETERS if re.search(
                r'(?<![a-z0-9_])' + re.escape(parameter) + r'(?![a-z0-9_])', lower))
            unsupported = any(word in lower for word in ('not support', 'unsupported', 'does not support'))
            if unsupported and 'thinking' in lower:
                categories.add('adaptive_thinking_unsupported' if 'adaptive' in lower else 'thinking_unsupported')
            if unsupported and 'effort' in lower:
                categories.add('effort_unsupported')
            if unsupported and 'context_management' in lower:
                categories.add('context_management_unsupported')
            if 'model' in lower and any(word in lower for word in ('not found', 'does not exist')):
                categories.add('model_not_found')
            # A native error often wraps the provider's JSON in "API Error: 400 ...".
            # Decode only in memory; even a provider message/account field is never copied.
            decoder = json.JSONDecoder()
            for match in list(re.finditer(r'\{', text))[:8]:
                try:
                    nested, _ = decoder.raw_decode(text[match.start():])
                except (ValueError, RecursionError):
                    continue
                visit(nested, depth + 1, True)
                break

    public_errors = []
    for event in events:
        if (event.get('schema') == 'hiroute.machine-envelope/v2'
                or event.get('schema_version') == {'major': 2, 'minor': 0}):
            error = event.get('error')
            if isinstance(error, dict) and event.get('status') in MACHINE_STATUSES:
                public_errors.append({'before_spawn': True, 'status': event['status'],
                                      'code': closed(error.get('code'), ERROR_VALUES)})
                visit(error, error_object=True)
        if event.get('type') in ('error', 'turn.failed') or (
                event.get('type') == 'result' and (event.get('is_error') is True or exit_code not in (None, 0))):
            visit(event)
        if event.get('type') == 'assistant' and event.get('error'):
            codes.add(closed(event['error'], ERROR_VALUES))
            visit(event.get('message'), error_object=True)
    if stderr:
        visit(stderr[-65536:].decode('utf-8', errors='replace'))
    return {'provider': closed(provider, ('codex', 'claude')), 'exit_code': exit_code,
            'stdout_bytes': len(stdout), 'stderr_bytes': len(stderr),
            'event_types': sorted({closed(event.get('type'), CLIENT_TYPES) for event in events}),
            'terminal_subtypes': sorted({closed(event.get('subtype'), CLIENT_SUBTYPES) for event in events
                                         if event.get('type') == 'result'}),
            'terminal_error_flags': [event['is_error'] if type(event.get('is_error')) is bool else None
                                     for event in events if event.get('type') == 'result'],
            'http_statuses': sorted(statuses), 'error_codes': sorted(codes),
            'public_cli_errors': public_errors[-8:],
            'explicit_error_parameters': sorted(parameters), 'parameter_mentions': sorted(mentions),
            'message_categories': sorted(categories), 'raw_output_retained': False}


def gateway_projection(failures):
    results = []
    for item in failures[-32:]:
        if not isinstance(item, dict):
            continue
        error = item.get('error') if isinstance(item.get('error'), dict) else {}
        projected = {'scenario': stage(item.get('scenario')),
                     'provider': closed(item.get('provider'), ('claude', 'codex')),
                     'error': {key: closed(error[key], PHASES if key == 'phase' else ERROR_VALUES)
                               for key in ('code', 'type', 'phase') if key in error}}
        for key in ('http_status', 'new_upstream_wire_requests'):
            if type(item.get(key)) is int and item[key] >= 0:
                projected[key] = item[key]
        results.append(projected)
    return results


def failure_projection(product, current_stage, gateway_start=0):
    result = {'stage': stage(current_stage), 'gateway_failures': gateway_projection(
        getattr(product, 'gateway_failures', [])[gateway_start:])}
    native = getattr(product, 'native_client_evidence', None)
    if native is not None:
        result['native_client'] = native
    return result


def read_observation(product, timeout=30):
    query = {'schema': 'hiroute.observation.query/v2', 'intent': {
        'view': 'home_value', 'query': {'period': 'seven_days', 'session_id': None, 'currency': None}}}
    encoded = json.dumps(query, sort_keys=True, separators=(',', ':')).encode()
    deadline = time.monotonic() + timeout
    while True:
        revisions = product.control('GetClientServiceStatus', {})['data']['revisions']
        grant = product.grant('GetValueV2', {'change_digest': 'sha256:' + hashlib.sha256(encoded).hexdigest(),
            'expected_revisions': revisions}, 'managed-value-' + str(time.monotonic_ns()))
        value = product.cli('value show', query, grant)[1]['data']
        totals = {item['metric']: item['known_sum'] for item in value['usage']
                  if item.get('metric') in METRICS}
        pending = value.get('pending_requests')
        require(all(type(totals.get(key)) is int and totals[key] >= 0 for key in METRICS)
                and type(pending) is int and pending >= 0, 'observation_response_invalid')
        if pending == 0:
            return {'totals': totals, 'pending_requests': pending}
        require(time.monotonic() < deadline, 'observation_pending_did_not_converge')
        time.sleep(.1)


def observation_delta(baseline, current, results):
    rows = [item['usage'] for item in results if isinstance(item.get('usage'), dict)]
    require(rows, 'observation_selected_usage_missing')
    fields = {'input': 'input_tokens', 'output': 'output_tokens', 'cache_read': 'cache_read_input_tokens'}
    require(all(type(row.get(field, 0)) is int and row.get(field, 0) >= 0
                for row in rows for field in fields.values()), 'observation_expected_usage_invalid')
    expected = {metric: sum(row.get(field, 0) for row in rows) for metric, field in fields.items()}
    require(expected['input'] > 0 and expected['output'] > 0, 'observation_selected_usage_invalid')
    require(baseline.get('pending_requests') == 0 and current.get('pending_requests') == 0,
            'observation_pending_requests')
    delta = {key: current['totals'][key] - baseline['totals'][key] for key in METRICS}
    return {'state': 'green' if delta == expected else 'red', 'scope': 'increment_since_baseline',
            'baseline': baseline, 'current': current, 'expected': expected, 'delta': delta,
            'parsed_response_count': len(rows), 'pending_requests': current['pending_requests']}


def reconcile_observation(product, baseline, results, timeout=30):
    deadline = time.monotonic() + timeout
    while True:
        result = observation_delta(baseline, read_observation(product), results)
        if result['state'] == 'green' or time.monotonic() >= deadline:
            return result
        time.sleep(.1)
