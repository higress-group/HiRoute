"""OpenCode native locations/records; shared product journeys own business assertions."""
from copy import deepcopy
import json
from pathlib import Path
from native_context_fixture import write_new, protect_configuration

def prepare_project_boundary(product):
    """Real Git stop, with unsafe foreign ancestors that native must not load."""
    import subprocess
    def git(cwd, *args):
        subprocess.run(['git', *args], cwd=cwd, env=product.env, check=True,
                       capture_output=True, timeout=10)
    import os
    linked = os.environ.get('HIROUTE_PRODUCT_AGENT_PROTOCOL', 'responses') == 'responses'
    repository = product.root / 'git-repository' if linked else product.project
    repository.mkdir(mode=0o700, exist_ok=True)
    git(repository, 'init', '-q')
    if linked:
        git(repository, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
            '-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-qm', 'fixture')
        # Product has not installed any project fixtures yet; keep all later
        # user/Skill paths unchanged while making cwd an actual linked worktree.
        product.project.rmdir()
        git(repository, 'worktree', 'add', '--detach', str(product.project), 'HEAD')
    foreign_config = product.root / 'opencode.json'
    write_new(foreign_config, '{"$schema":""}\n')
    imported = product.root / 'foreign-tool-imported'
    tool = product.root / '.opencode/tools/foreign.ts'
    tool.parent.mkdir(mode=0o700, parents=True)
    write_new(tool, 'import { writeFileSync } from "node:fs";\nwriteFileSync(' +
              json.dumps(str(imported)) + ', "imported");\nthrow new Error("foreign tool imported");\n')
    return dict(kind='linked-worktree' if linked else 'repository', imported=str(imported),
                protected={str(path): path.read_bytes() for path in (foreign_config, tool)})

def assert_project_admission(product, fixture, upstream, boundary):
    """False stops fail through public Worker admission before DB/tool/model work."""
    import shutil
    from delegation_product import worker_cli, wait_for_worker_result
    marker = product.project / '.git'
    saved = product.project / '.git-confirmed-fixture'
    marker.rename(saved)
    shadow_git = product.root / 'bin/git'
    assert not shadow_git.exists()
    events = upstream.controls / 'native-context-events.jsonl'
    before = events.read_bytes() if events.exists() else b''
    results = []
    try:
        for kind in ('empty', 'corrupt', 'broken-worktree', 'unavailable-git'):
            if kind == 'empty':
                marker.mkdir(mode=0o700)
            elif kind == 'unavailable-git':
                saved.rename(marker)
                write_new(shadow_git, '#!/bin/sh\nexit 127\n', executable=True)
            else:
                write_new(marker, 'not a gitdir\n' if kind == 'corrupt' else 'gitdir: missing-worktree\n')
            try:
                command = ('worker exec --plan ' + product.plan_id + ' --cwd ' + str(product.project) +
                           ' --run-timeout 30 --no-wait --submission-key opencode-false-git-' +
                           kind + ' --file - --output json')
                status, response = worker_cli(product, command, 'Return done.', success=False)
                if status == 0:
                    try:
                        result = wait_for_worker_result(product, response['data']['run_id'], expected='failed', timeout=40)
                    finally:
                        # Even a different terminal error must not hide native
                        # startup side effects in this pre-launch refusal case.
                        assert not Path(fixture['database']).exists(), 'unconfirmed Git stop launched native database initialization'
                    assert not result.get('result'), 'unconfirmed Git stop returned a result'
                else:
                    assert response.get('error', {}).get('code') == 'CAPABILITY_UNAVAILABLE', response
                assert not Path(fixture['database']).exists(), 'unconfirmed Git stop launched native database initialization'
                assert not Path(boundary['imported']).exists(), 'unconfirmed Git stop imported a foreign tool'
                assert (events.read_bytes() if events.exists() else b'') == before, 'unconfirmed Git stop made a model request'
                for path, original in boundary['protected'].items():
                    assert Path(path).read_bytes() == original, 'unconfirmed Git stop rewrote borrowed configuration/tool'
                results.append(kind)
            finally:
                if kind == 'unavailable-git':
                    shadow_git.unlink()
                    marker.rename(saved)
                elif marker.is_dir():
                    shutil.rmtree(marker)
                else:
                    marker.unlink()
    finally:
        saved.rename(marker)
    return dict(positive=boundary['kind'], rejected=results,
                foreign_tool_imported=False, native_database_created=False, model_requests=0)

def select_context(product):
    home=Path(product.env['HOME'])
    for key,folder in [('XDG_CONFIG_HOME','.config'),('XDG_DATA_HOME','.local/share'),('XDG_CACHE_HOME','.cache'),('XDG_STATE_HOME','.local/state')]:
        root=home/folder
        root.mkdir(mode=0o700,parents=True,exist_ok=True)
        product.env[key]=str(root)
    root=Path(product.env['XDG_CONFIG_HOME'])/'opencode'
    root.mkdir(mode=0o700,exist_ok=True)
    product.env.update(OPENCODE_DISABLE_MODELS_FETCH='true',OPENCODE_DISABLE_AUTOUPDATE='true',OPENCODE_DISABLE_LSP_DOWNLOAD='true',OPENCODE_DISABLE_FFF='true',OPENCODE_PURE='true',OPENCODE_DISABLE_DEFAULT_PLUGINS='true')
    product.env['OPENCODE_DB']=str(Path(product.env['XDG_DATA_HOME'])/'opencode/opencode.db')
    return root

def prepare_context(product,fixture):
    root=Path(fixture['config'])
    fixture['product_storage']=str(product.storage)
    fixture['database']=product.env['OPENCODE_DB']
    import secrets
    nonce=secrets.token_hex(10)
    fixture['edit']={'path':str(product.project/'opencode-owned-edit.txt'),'before':'before-'+nonce,'after':'edited-'+nonce}
    write_new(Path(fixture['edit']['path']),fixture['edit']['before']+'\n')
    write_new(root/'opencode.json',json.dumps({'$schema':'https://opencode.ai/config.json','model':'ambient/wrong-native-model','small_model':'ambient/wrong-native-model','provider':{'ambient':{'npm':'@ai-sdk/openai','options':{'baseURL':'http://127.0.0.1:9/v1','apiKey':'wrong-native-key'},'models':{'wrong-native-model':{'limit':{'context':32768,'output':4096}}}}},'share':'disabled','autoupdate':False}))
    protect_configuration(fixture,[root/'opencode.json'])

def exact_history(fixture,cancelled_session_id=None):
    import sqlite3
    matches=[]
    for marker in (Path(fixture['product_storage'])/'delegation-workers/sessions').glob('*/.hiroute-native-root-v1.json'):
        value=json.loads(marker.read_text())
        if value['task_id']!=fixture['task_id']: continue
        assert value['harness']=='opencode'
        binding={'native_session_id':cancelled_session_id} if cancelled_session_id else json.loads((marker.parent/'native-session-binding.json').read_text())
        database=Path(fixture['database'])
        assert database.is_file() and not database.is_symlink()
        # Only the selected synthetic session's existence/identity; production never parses its history.
        with sqlite3.connect(database.as_uri()+'?mode=ro',uri=True) as db:
            assert db.execute('SELECT id FROM session WHERE id=?',(binding['native_session_id'],)).fetchone()==(binding['native_session_id'],)
        matches.append((str(database),binding['native_session_id']))
    assert len(matches)==1,'expected exactly one bound OpenCode task'
    return matches[0]

class OpenCodeModelSettings:
    def __init__(self,config,foreign_endpoint):
        self.harness,self.auth_owned='opencode',None
        self.path=Path(config)/'opencode.json'
        assert not self.path.exists(),'model journey requires its dedicated empty context'
        self.original=None
        self.native_default='fixture-native/native-default'
        self.path.write_text(json.dumps({'$schema':'https://opencode.ai/config.json','model':self.native_default,'small_model':self.native_default,'provider':{'fixture-native':{'npm':'@ai-sdk/openai','options':{'baseURL':foreign_endpoint,'apiKey':'synthetic-unselected-native-key'},'models':{'native-default':{'limit':{'context':100000,'output':4096}}}}},'share':'disabled','autoupdate':False,'compaction':{'auto':False},'agent':{'title':{'disable':True},'summary':{'disable':True}}}))
        self.path.chmod(0o600)
        self.baseline=self.read()
    def read(self):
        assert self.path.is_file() and not self.path.is_symlink()
        value=json.loads(self.path.read_text())
        value['providers']=value.pop('provider',{})
        for provider in value['providers'].values():
            provider['models']=[dict(model,id=key) for key,model in provider['models'].items()]
        value['model']={'name':value['model']}
        return value
    def assert_preserved(self,provider_id,selected_default=None):
        actual=self.read()
        for name in list(actual['providers']):
            if name==provider_id or name.startswith(provider_id+'-'): actual['providers'].pop(name)
        expected=deepcopy(self.baseline)
        if selected_default: expected['model']['name']=selected_default
        assert actual==expected,'OpenCode save changed daily defaults/unowned fields'
    def add_user_edit(self,provider_id):
        self.assert_preserved(provider_id)
        text=self.path.read_text();end=text.rfind('}')
        self.path.write_text(text[:end].rstrip()+',"instructions":["./user-rule.md"]'+text[end:])
        self.baseline['instructions']=['./user-rule.md']
    def select_default(self,expected,replacement):
        text=self.path.read_text();needle=json.dumps(expected)
        assert self.read()['model']['name']==expected
        # Only the root value is replaced; owned provider bytes stay unchanged.
        start=text.index('"model"');at=text.index(needle,start)
        self.path.write_text(text[:at]+json.dumps(replacement)+text[at+len(needle):])
    def close(self):
        assert self.read()==self.baseline
        self.path.unlink()

def main_route(product,endpoint,model,token,fixture):
    config=Path(fixture['config'])/'opencode.json'
    value=json.loads(config.read_text())
    value['provider']['hiroute-main-acceptance']={'npm':'@ai-sdk/openai','options':{'baseURL':endpoint,'apiKey':token,'store':False},'models':{model:{'limit':{'context':100000,'output':4096}}}}
    value['model']=value['small_model']='hiroute-main-acceptance/'+model
    value['permission']={'*':'allow'}
    value['agent']={'title':{'disable':True},'summary':{'disable':True}}
    config.write_text(json.dumps(value));config.chmod(0o600)
    protect_configuration(fixture,[config])
    return 'hiroute-main-acceptance/'+model

def cancelled_history(fixture,run_id):
    from native_task_witness import cancelled_session
    return exact_history(fixture,cancelled_session(fixture,run_id,'opencode'))
