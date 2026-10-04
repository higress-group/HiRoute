"""Two independent model-role oracles for real Qoder Skill → public CLI → Worker.

Only native tool results and the Worker's disk artifact can complete this journey.
This module does not start HiRoute, impersonate an Agent, or write its installed Skill.
"""
import json
from pathlib import Path
import shlex

from native_context_fixture import decision, qoder_skill_expansions, tool_results


CASES = ('agent.collaboration.user-skill', 'agent.collaboration.public-worker-delegation',
         'agent.collaboration.disable-owned-skill')


def successful_envelope(output):
    """Qoder Bash may surround stdout with native timing/status text."""
    assert isinstance(output, str), 'expected a native Bash text result'
    decoder = json.JSONDecoder()
    matches = []
    for index, char in enumerate(output):
        if char != '{':
            continue
        try:
            value, _ = decoder.raw_decode(output[index:])
        except ValueError:
            continue
        if isinstance(value, dict) and 'schema_version' in value and 'status' in value:
            matches.append(value)
    assert len(matches) == 1 and matches[0]['status'] == 'succeeded', \
        'expected one successful public CLI envelope in the correlated tool result'
    return matches[0]


def output_envelope(output):
    return successful_envelope(output)['data']


def worker_decision(fixture, body):
    results = tool_results(body, 'qoder')
    artifact = fixture['artifact']
    if 'delegated_artifact' in results:
        assert artifact['receipt'] in results['delegated_artifact'], 'Worker tool did not return artifact receipt'
        path = Path(artifact['path'])
        assert path.is_file() and not path.is_symlink() and path.read_text().strip() == artifact['receipt'], \
            'Worker did not produce the independent disk artifact'
        return dict(kind='text', text=artifact['receipt'], continued=False)
    action = decision(fixture, body)
    if action['kind'] != 'text':
        return action
    command = ('/usr/bin/printf "%s\\n" ' + shlex.quote(artifact['receipt']) + ' > ' +
               shlex.quote(artifact['path']) + ' && /bin/cat ' + shlex.quote(artifact['path']))
    return dict(kind='tool', id='delegated_artifact', name='Bash',
                arguments={'command': command, 'timeout': 10000})


class MainAgentOracle:
    def __init__(self, fixture, skill, cli, worker_events):
        self.fixture = fixture
        self.skill = Path(skill)
        self.skill_body = self.skill.read_text().split('---', 2)[-1].strip()
        assert self.skill_body and 'hiroute worker plans' in self.skill_body
        self.cli = str(Path(cli).resolve(strict=True))
        self.worker_events = worker_events
        self.accepted = None
        self.user_skill_proved = False
        self.completed = False
        self.requests = 0

    def reply(self, _unused, body):
        self.requests += 1
        assert self.requests <= 32, 'main Agent exceeded the bounded public observation budget'
        rendered = json.dumps(body)
        assert self.fixture['main_marker'] in rendered, 'Worker request reached the main Agent source'
        results = tool_results(body, 'qoder')
        names = {item.get('name') for item in body.get('tools', [])}
        assert {'Skill', 'Bash'} <= names, 'native main Agent Skill/Bash tools unavailable'
        if 'main_skill' not in results:
            assert 'hiroute-collaboration' in rendered, 'installed user collaboration Skill not discovered'
            return dict(kind='tool', id='main_skill', name='Skill',
                        arguments={'skill': 'hiroute-collaboration'})
        # Native Qoder returns a correlated launch receipt, then expands the body
        # into a user message. Exact directory excludes a same-named project/plugin.
        contents = qoder_skill_expansions(body, self.skill.parent)
        assert any(text.strip() == self.skill_body for text in contents), \
            'actual installed user Skill directory and body were not loaded'
        self.user_skill_proved = True
        if 'main_plans' not in results:
            return self.bash('main_plans', shlex.quote(self.cli) + ' worker plans --output json')
        directory = output_envelope(results['main_plans'])
        plans = [plan for plan in directory['plans'] if plan['agent_plan_id'] == self.fixture['plan_id']]
        assert len(plans) == 1 and plans[0]['harness'] == 'qoder_cli' and plans[0]['availability'] == 'ready', \
            'the public directory did not offer the published Qoder Worker Plan'
        if 'main_exec' not in results:
            prompt = 'Use ' + ' and '.join(item['name'] for item in self.fixture['skills']) + \
                     ' to complete the delegated native receipt task.'
            command = ('/usr/bin/printf %s ' + shlex.quote(prompt) + ' | ' + shlex.quote(self.cli) +
                       ' worker exec --plan ' + shlex.quote(plans[0]['agent_plan_id']) + ' --cwd ' +
                       shlex.quote(self.fixture['project']) + ' --run-timeout 120 --wait-timeout 120' +
                       ' --submission-key qoder-main-agent-delegation --file - --output json')
            return self.bash('main_exec', command, 150000)
        envelope = successful_envelope(results['main_exec'])
        accepted = envelope['data']
        # Retain the actual public result even on failure for scoped diagnosis;
        # completion still requires success and the independent disk artifact.
        self.accepted = accepted
        assert accepted.get('task_id') and accepted.get('run_id'), 'public Worker acceptance has no identity'
        wait_index = 1
        while 'main_wait_' + str(wait_index) in results:
            envelope = successful_envelope(results['main_wait_' + str(wait_index)])
            self.assert_same_worker(envelope['data'], accepted)
            wait_index += 1
        current = envelope['data']
        self.accepted = current
        if current['run_state'] not in ('succeeded', 'failed', 'cancelled', 'unknown'):
            actions = [action['input'] for action in envelope.get('next_actions', [])
                       if action.get('command_id') == 'worker.wait'
                       and action.get('input', {}).get('run_id') == accepted['run_id']]
            assert (len(actions) == 1 and type(actions[0].get('after_revision')) is int
                    and actions[0]['after_revision'] >= 0), \
                'active public Worker result must provide its exact wait cursor'
            command = (shlex.quote(self.cli) + ' worker wait --run ' + shlex.quote(accepted['run_id']) +
                       ' --after-revision ' + str(actions[0]['after_revision']) +
                       ' --wait-timeout 20 --output json')
            return self.bash('main_wait_' + str(wait_index), command, 25000)
        assert current['run_state'] == 'succeeded', 'main Agent did not observe successful public Worker execution'
        if 'result' not in current:
            if 'main_result' not in results:
                return self.bash('main_result', shlex.quote(self.cli) + ' worker result --run ' +
                                 shlex.quote(accepted['run_id']) + ' --output json')
            current = output_envelope(results['main_result'])
            self.assert_same_worker(current, accepted)
            assert current['run_state'] == 'succeeded', 'public Worker result is not successful'
        receipt = self.fixture['artifact']['receipt']
        assert receipt in current.get('result', ''), 'public Worker result lost its tool artifact'
        assert Path(self.fixture['artifact']['path']).read_text().strip() == receipt, 'disk artifact missing'
        attempts = self.worker_events()
        assert attempts and all(event['state'] == 'green' for event in attempts), 'Worker source is incomplete'
        assert any(event.get('call_id') == 'delegated_artifact' for event in attempts), \
            'no real Worker tool request produced the artifact'
        self.accepted, self.completed = current, True
        return dict(kind='text', text='MAIN-AGENT-COMPLETED-' + receipt, continued=False)

    @staticmethod
    def assert_same_worker(current, accepted):
        assert all(current.get(key) == accepted[key] for key in ('task_id', 'run_id')), \
            'public observation switched the accepted Worker identity'

    @staticmethod
    def bash(call_id, command, timeout=10000):
        return dict(kind='tool', id=call_id, name='Bash', arguments={'command': command, 'timeout': timeout})


def main_route_settings(endpoint, alias, credential_env):
    """Independent native-main fixture settings; Worker uses the production renderer."""
    provider = 'hiroute-main-acceptance'
    selected = provider + '/' + alias
    own_alias = {'modelConfig': {'model': selected}}
    return selected, {
        'general': {'enableAutoUpdate': False, 'sessionRetention': {'enabled': False},
                    'plan': {'modelRouting': False}},
        'disableAllHooks': True, 'promptSuggestionEnabled': False, 'model': {'name': selected},
        'modelConfigs': {'aliases': {selected: own_alias}, 'customAliases': {selected: own_alias},
                         'overrides': [], 'customOverrides': []},
        'providers': {provider: {'protocol': 'openai-responses', 'baseUrl': endpoint,
            'apiKey': '${' + credential_env + '}', 'model': alias,
            'models': [{'model': alias, 'contextWindow': 100000, 'capabilities': {'tools': True}}],
            'routing': {purpose: alias for purpose in ('utility', 'session_title', 'summary', 'compact', 'subagent')}}},
    }
