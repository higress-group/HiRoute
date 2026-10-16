"""External Codex double for the installed CLI flow; never native-consumer evidence.

Exercise the existing isolated two-round Skill challenge through real HTTP and CLI execution.
The separate native acceptance uses actual Codex discovery and tools.
"""
from pathlib import Path


def support_local_skill_challenge(product):
    path = product.fixture_bin / 'codex'
    source = path.read_text()
    anchor = "endpoint = urlparse(provider['base_url'])\n"
    assert source.count(anchor) == 1
    source = source.replace(anchor, r'''
# The test double implements only the existing isolated local Skill check.
from pathlib import Path
import subprocess
skill = Path(os.environ['HOME']) / '.agents/skills/hiroute-probe/SKILL.md'
if probe and skill.is_file():
    endpoint = urlparse(provider['base_url'])
    assert endpoint.hostname == '127.0.0.1'
    inputs = [{'role': 'user', 'content': skill.read_text()}]
    for _ in range(2):
        conn = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=20)
        try:
            conn.request('POST', endpoint.path.rstrip('/') + '/responses',
                         body=json.dumps({'model': model, 'stream': True, 'input': inputs,
                              'tools': [{'type': 'function', 'name': 'exec_command', 'parameters': {}}]}),
                         headers={'Content-Type': 'application/json', **provider['http_headers']})
            response = conn.getresponse()
            assert response.status == 200
            events = [json.loads(line[6:]) for line in response.read().decode().splitlines()
                      if line.startswith('data: ') and line != 'data: [DONE]']
            completed = next(event['response'] for event in events if event['type'] == 'response.completed')
        finally:
            conn.close()
        calls = [item for item in completed['output'] if item['type'] == 'function_call']
        if not calls:
            raise SystemExit(0)
        assert len(calls) == 1
        call = calls[0]
        output = subprocess.check_output(['/bin/sh', '-c', json.loads(call['arguments'])['cmd']], text=True)
        inputs.append({'type': 'function_call_output', 'call_id': call['call_id'], 'output': output})
    raise SystemExit(1)
''' + anchor)
    path.write_text(source)
