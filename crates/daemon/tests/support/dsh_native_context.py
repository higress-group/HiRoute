"""DSH's fixture-owned public patch and opaque history; common journeys own assertions."""
import json
from pathlib import Path
from native_context_fixture import write_new, protect_configuration

def read_patch(path):
    # The fixture produces the public JSON subset of YAML. This is deliberately
    # not a second parser for arbitrary user YAML or a DSH package-layout reader.
    text = Path(path).read_text()
    if text.strip() in ('', '[]'): return []
    rows, current = [], []
    for line in text.splitlines():
        if line.startswith('- '):
            if current: rows.append(json.loads('\n'.join(current)))
            current = [line[2:]]
        elif line.startswith('  '): current.append(line[2:])
        elif line.strip() and not line.startswith('#'): raise AssertionError('unexpected fixture YAML')
    if current: rows.append(json.loads('\n'.join(current)))
    return rows

def encode_patch(rows):
    return ''.join('- '+json.dumps(row,indent=2).replace('\n','\n  ')+'\n' for row in rows) or '[]\n'

def prepare_context(product, fixture):
    root = Path(fixture['config'])
    fixture['product_storage'] = str(product.storage)
    write_new(root/'cordis.patch.yml','[]\n')
    write_new(root/'profiles/acp/cordis.patch.yml',encode_patch([
        {'id':'agent-default-model','config':{'provider':'ambient','model':'wrong-native-model'}},
        {'id':'llm-pi-ai','config':{'providers':{'ambient':{'api':'openai-responses',
            'baseURL':'http://127.0.0.1:9/v1','models':[{'id':'wrong-native-model'}]}}}},
    ]))
    protect_configuration(fixture,[root/'cordis.patch.yml',root/'profiles/acp/cordis.patch.yml'])

def history_directory(fixture):
    return Path(fixture['product_storage'])/'delegation-workers/sessions'

def exact_history(fixture, cancelled_session_id=None):
    found = []
    for marker in history_directory(fixture).glob('*/.hiroute-native-root-v1.json'):
        value = json.loads(marker.read_text())
        if value['task_id'] != fixture['task_id']: continue
        assert value['harness'] == 'deepseek_harness'
        native_id = cancelled_session_id or json.loads((marker.parent/'native-session-binding.json').read_text())['native_session_id']
        paths = list((marker.parent/'native-dsh/sessions').rglob('*.jsonl'))
        assert len(paths) == 1 and paths[0].is_file() and not paths[0].is_symlink()
        # Only synthetic bytes are inspected. Production delegates format interpretation to ACP.
        assert fixture['receipt'] in paths[0].read_text() or fixture.get('boundary'), 'missing task receipt'
        found.append((str(paths[0]),native_id))
    assert len(found) == 1, 'expected one task-owned DSH history'
    return found[0]

def cancelled_history(fixture, run_id):
    from native_task_witness import cancelled_session
    return exact_history(fixture, cancelled_session(fixture,run_id,'deepseek_harness'))

class DshModelSettings:
    """Fixture leaf for DSH's public Web composition; shared journey owns behavior."""
    def __init__(self, config, foreign_endpoint):
        import secrets
        self.harness, self.auth_owned = 'dsh', None
        self.path = Path(config)/'profiles/web/cordis.patch.yml'
        assert not self.path.exists(), 'model fixture requires its own empty DSH context'
        self.original = None
        self.native_default = 'fixture-native/native-default'
        self.baseline = {'providers':{'fixture-native':{'api':'openai-responses',
            'baseURL':foreign_endpoint,'models':[{'id':'native-default','input':['text'],
                'contextWindow':100000,'maxTokens':4096}],
            'fixtureUnknownProviderKey':{'preserve':['native','user']}}},
            'model':{'name':self.native_default},
            'hirouteAcceptance':{'unknown':['preserve',secrets.token_hex(8)]}}
        self.write(self.baseline)

    def read(self):
        assert self.path.is_file() and not self.path.is_symlink()
        value = {}
        for row in read_patch(self.path):
            if row['id']=='llm-pi-ai': value['providers']=row['config']['providers']
            elif row['id']=='agent-default-model':
                value['model']={'name':row['config']['provider']+'/'+row['config']['model']}
            elif row['id']=='fixture-preserved': value.update(row['config'])
        return value

    def write(self, value):
        provider,model = value['model']['name'].split('/',1)
        rows = [{'id':'llm-pi-ai','config':{'providers':value['providers']}},
                {'id':'agent-default-model','config':{'provider':provider,'model':model}},
                {'id':'fixture-preserved','config':{k:v for k,v in value.items() if k not in ('providers','model')}}]
        self.path.write_text(encode_patch(rows))
        self.path.chmod(0o600)

    def assert_preserved(self, provider_id, selected_default=None):
        from additional_model_fixture import OwnedModelSettings
        return OwnedModelSettings.assert_preserved(self,provider_id,selected_default)

    def add_user_edit(self, provider_id):
        import secrets
        self.assert_preserved(provider_id)
        rows = read_patch(self.path)
        row = next(r for r in rows if r['id']=='fixture-preserved')
        row['config']['hirouteLaterEdit']={'edited_after_apply':secrets.token_hex(8)}
        # Replace only the unrelated row: managed provider bytes remain untouched.
        old = encode_patch([next(r for r in read_patch(self.path) if r['id']=='fixture-preserved')])
        self.path.write_text(self.path.read_text().replace(old,encode_patch([row]),1))
        self.baseline['hirouteLaterEdit']=row['config']['hirouteLaterEdit']

    def select_default(self, expected, replacement):
        rows = read_patch(self.path)
        row = next(r for r in rows if r['id']=='agent-default-model')
        assert row['config']['provider']+'/'+row['config']['model']==expected
        old = encode_patch([row])
        row['config']['provider'],row['config']['model']=replacement.split('/',1)
        self.path.write_text(self.path.read_text().replace(old,encode_patch([row]),1))

    def close(self):
        from additional_model_fixture import OwnedModelSettings
        return OwnedModelSettings.close(self)


def main_route_patch(product, endpoint, model):
    """Only the independent main Agent source is seeded; Worker uses production overlay."""
    rows = [{'id':'llm-pi-ai','config':{'providers':{'hiroute-main-acceptance':{
        'api':'openai-responses','baseURL':endpoint,'apiKeyEnv':'HIROUTE_DSH_MAIN_TOKEN',
        'models':[{'id':model,'input':['text'],'contextWindow':100000,'maxTokens':2048}]}}}},
        {'id':'agent-default-model','config':{'provider':'hiroute-main-acceptance','model':model}},
        {'id':'session-log-deepseek','config':{'enabled':False}},
        {'id':'session-telemetry-otel','disabled':True}]
    patch = product.root/'dsh-main.patch.yml'
    write_new(patch,encode_patch(rows))
    return patch
