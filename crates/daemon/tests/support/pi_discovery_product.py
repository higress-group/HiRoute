"""Import a native Agent's effective static source through real Scan → Prepare → Save.

Only native configuration and the deterministic upstream are fixtures. Protected
input, candidate state, save conflict and restart all use public product entries.
"""
from publication_process import judgment_fixture
import hashlib
import json
import os
import secrets
from pathlib import Path
import subprocess
import sys

from model_connections_product import control, desktop_grant, save_compute_candidate
from native_context_fixture import NativeContextUpstream, digest, write_new
from native_context_product import report_failure, selected_installation
from publication_product import Product, encoded
from publication_process import plan_change
from agent_product_support import apply_settings, expose_native_installation
from additional_model_fixture import PersistedRouteOracle, read_persisted_route

HARNESS = 'pi'

NATIVE_PREFIX = '\ufeff// Native effective endpoint differs from provider default\n'

CASES = ('agent.sources.effective-static-import', 'agent.sources.changed-source-rejected',
         'agent.sources.imported-route-usable')


def use_imported_route(product, binary, source, saved):
    """The saved source must reach Plan authoring and an ordinary native call."""
    product.editor = {'schema':'hiroute.plan-editor/v2', 'display_name':'Imported '+HARNESS+' route',
        'purpose':'Use the imported static API', 'mode':'fixed_model',
        'candidates':[{'binding_id':saved['binding_id'], 'reasoning':{'kind':'toggle','enabled':False}}],
        'delegation_enabled':False,
        'smart':{'economy':[],'primary':[],'judgment': judgment_fixture(),'reselect_on_user_message':False,
                 'classifier':{'kind':'local_rules'},'complex_keywords':[]},
        'free':{'candidates':[],'primary':[],'primary_fallback':False},
        'requirements':{}, 'limits':{'maximum_attempts':1,'request_timeout_ms':30000,'attempt_timeout_ms':30000}}
    change = plan_change(product, 'create', 'pi-imported-route')
    preview = product.preview('routing preview', {'change':change})
    product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change':change}, 'pi-imported-route')
    plan = preview['plan_head']
    agent = next(row for row in product.preview('agents scan')['agents'] if row['agent_id']=='agent_'+HARNESS+'_default')
    product.agent_connection = 'agent-connection/' + agent['context_id']
    product.additional_harness = HARNESS
    spec = {'schema_version':{'major':2,'minor':0}, 'context_id':agent['context_id'],
        'model':{'intent':'configure','settings':{'mode':HARNESS+'_additional','allowed_plan_ids':[plan['reference']['plan_id']]}},
        'collaboration':{'intent':'keep'}, 'access_token':{'intent':'keep'}}
    configured, status = apply_settings(product, spec, 'pi-imported-model-route', 'model')
    product.bearer(product.agent_connection)
    oracle = PersistedRouteOracle(source.model)
    source.reply = oracle.reply
    try:
        namespace = configured['model_effect']['provider_id']
        if HARNESS=='opencode':
            providers=json.loads((Path(product.env['XDG_CONFIG_HOME'])/'opencode/opencode.json').read_text())['provider']
            providers={name:dict(p,models=[dict(m,id=k) for k,m in p['models'].items()]) for name,p in providers.items()}
        elif HARNESS=='dsh':
            from dsh_native_context import read_patch
            providers = next(r['config']['providers'] for r in read_patch(
                Path(product.env['DSH_HOME'])/'profiles/web/cordis.patch.yml') if r['id']=='llm-pi-ai')
        else:
            text = (Path(product.env['PI_CODING_AGENT_DIR']) / 'models.json').read_text()
            if text.startswith(NATIVE_PREFIX):
                text = text[len(NATIVE_PREFIX):]
            providers = json.loads(text)['providers']
        ids = [name for name, provider in providers.items() if name.startswith(namespace + '-') and any(m.get('id') == plan['model_alias'] for m in provider.get('models', []))]
        assert len(ids) == 1
        selector = ids[0] + '/' + plan['model_alias']
        return read_persisted_route(product, binary, selector, source, oracle, 'pi-imported-source-route')
    finally:
        spec['model'] = {'intent':'restore','restore_point_ref':status['restore_point_ref']}
        apply_settings(product, spec, 'pi-imported-model-restore', 'model')


def prepare(product, discovery, identity):
    payload = {'discovery': discovery, 'prepare_id': identity}
    revisions = control(product, 'GetClientServiceStatus', {})['data']['revisions']
    grant = desktop_grant(product, 'PrepareDiscoveredModelConnection',
        'sha256:' + hashlib.sha256(encoded(payload)).hexdigest(), revisions, identity + '-grant-' + secrets.token_hex(6))
    return control(product, 'PrepareDiscoveredModelConnection', payload, grant)['data']


def scan(product):
    items = control(product, 'ScanCompute', {})['data']['items']
    sources = [item for item in items if item['agent_id'] == 'agent_'+HARNESS+'_default'
        and item.get('native_provider_id') == 'native-source']
    assert len(sources) == 1 and sources[0]['inventory_eligible'], 'static native source not importable'
    return sources[0]['discovery']


def opencode_local_credential_failures(models_path, controls):
    """Native-readable keys outside the protected import reader's supported subset."""
    value = json.loads(models_path.read_text())
    template = value['provider']['native-source']
    linked = controls / 'linked.key'
    target = controls / 'link-target.key'
    public = controls / 'public.key'
    oversized = controls / 'oversized.key'
    write_new(target, 'synthetic-linked-key')
    linked.symlink_to(target)
    write_new(public, 'synthetic-public-key')
    public.chmod(0o644)
    write_new(oversized, 'x' * (1024 * 1024 + 1))
    for name, key in [('linked', linked), ('public', public), ('oversized', oversized)]:
        provider = json.loads(json.dumps(template))
        provider['options']['apiKey'] = '{file:' + str(key) + '}'
        value['provider']['a-broken-' + name] = provider
    models_path.write_text(json.dumps(value))
    return (linked, target, public, oversized)


def run(repository, candidate):
    global HARNESS
    HARNESS = os.environ.get('HIROUTE_PRODUCT_WORKER_HARNESS','pi')
    assert HARNESS in ('pi','dsh','opencode')
    repo = Path(repository).resolve()
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() == candidate
    product = Product(repo)
    source = None
    stage = 'native-static-source'
    report = dict(scenario=HARNESS+'-static-source-import', candidate=candidate, worker_harness=HARNESS,
        required_cases=list(CASES), selected_cases=list(CASES), cases=[],
        evidence_limit='Real static import/save/restart followed by ordinary native routing; deterministic source; no OAuth')
    try:
        product.enable_debug_diagnostics()
        binary, _, node = selected_installation(HARNESS)
        expose_native_installation(product, HARNESS, binary, node)
        if HARNESS=='opencode':
            from opencode_native_context import select_context
            config=select_context(product)
        else:
            config = Path(product.env['HOME']) / ('selected-'+HARNESS+'-config')
            config.mkdir(mode=0o700)
        if HARNESS!='opencode':
            product.env['DSH_HOME' if HARNESS=='dsh' else 'PI_CODING_AGENT_DIR'] = str(config)
        controls = product.root / 'source'
        controls.mkdir(mode=0o700)
        write_new(controls / 'native-context.json', '{}')
        source = NativeContextUpstream(controls)
        credential_context_paths = ()
        if HARNESS=='opencode':
            models_path=config/'opencode.json'
            auth_path=Path(product.env['XDG_DATA_HOME'])/'opencode/auth.json'
            # This journey counts one ordinary model call. Worker auxiliary routing
            # is exercised independently with the native title/compaction agents enabled.
            write_new(models_path,json.dumps({'$schema':'https://opencode.ai/config.json','agent':{'title':{'disable':True},'summary':{'disable':True}},'provider':{'native-source':{'npm':'@ai-sdk/openai','options':{'baseURL':source.base_url},'models':{source.model:{'limit':{'context':16384,'output':4096}}}}}}))
            write_new(auth_path,json.dumps({'native-source':{'type':'api','key':source.token}}))
            changed_auth=json.dumps({'native-source':{'type':'api','key':'changed-native-key'}})
            credential_context_paths = opencode_local_credential_failures(models_path, controls)
        elif HARNESS=='dsh':
            from dsh_native_context import encode_patch, source_credentials
            models_path = config/'profiles/web/cordis.patch.yml'
            provider = {'api':'openai-responses','baseURL':source.base_url,'apiKeyEnv':'IMPORT_KEY',
                'models':[{'id':source.model,'contextWindow':16384,'maxTokens':4096,'input':['text']}]}
            write_new(models_path,encode_patch([{'id':'llm-pi-ai','config':{'providers':{'native-source':provider}}}]))
            auth_path, credential_context_paths = source_credentials(config, source.token)
            product.env.pop('IMPORT_KEY', None)
            changed_auth = json.dumps({'version':1,'refs':{'IMPORT_KEY':'changed-native-key'}})
        else:
            models_path,auth_path = config/'models.json',config/'auth.json'
            provider = {'api':'openai-responses','baseUrl':'http://127.0.0.1:9/v1','apiKey':'provider-fallback',
                'models':[{'id':source.model,'baseUrl':source.base_url,'contextWindow':16384,'maxTokens':4096,'input':['text']}]}
            write_new(models_path,NATIVE_PREFIX+json.dumps({'providers':{'native-source':provider}}))
            write_new(auth_path,json.dumps({'native-source':{'type':'api_key','key':'${IMPORT_KEY}','env':{'IMPORT_KEY':source.token}}}))
            changed_auth = json.dumps({'native-source':{'type':'api_key','key':'changed-native-key'}})
        before = {path:digest(path) for path in (models_path,auth_path,*credential_context_paths)}
        original_auth = auth_path.read_bytes()
        product.secrets.update((source.token, 'provider-fallback'))
        product.start()
        stage = 'public-scan-prepare-save'
        if HARNESS == 'opencode':
            items = control(product, 'ScanCompute', {})['data']['items']
            blocked = [item for item in items if item['agent_id']=='agent_opencode_default'
                and str(item.get('native_provider_id','')).startswith('a-broken-')]
            assert len(blocked) == 3 and all(not row['inventory_eligible'] and not row.get('discovery') for row in blocked), 'one credential failure hid independent native sources'
            report['provider_local_credential_failures_isolated'] = ['symlink', 'unsafe_permissions', 'oversized']
        discovery = scan(product)
        candidate_view = prepare(product, discovery, 'pi-source-prepare')
        assert prepare(product, discovery, 'pi-source-prepare') == candidate_view, 'prepare retry changed its candidate'
        models = candidate_view['models']
        assert len(models) == 1 and models[0]['upstream_model_id'] == source.model and models[0]['selectable']
        saved = save_compute_candidate(product, candidate_view, models[0]['model_ref'], 'pi-source-save')
        assert {path:digest(path) for path in before} == before, 'import rewrote native user files'
        product.stop()
        product.start()
        snapshot = control(product, 'ListCompute', {})['data']
        assert saved['source_id'] in json.dumps(snapshot), 'saved Pi source disappeared after restart'
        report['cases'].append({'id':CASES[0],'state':'green'})
        stage = 'source-rotated-before-save'
        prepared = prepare(product, scan(product), 'pi-source-stale-prepare')
        revision = control(product, 'ListCompute', {})['data']['revisions']
        change = {'schema':'hiroute.compute-management-change/v2',
            'subject':{'kind':'candidate','candidate':prepared['candidate']},'expected_revisions':revision,
            'selected_model_refs':[prepared['models'][0]['model_ref']], 'intent':'save_ready','key_edits':[]}
        preview = control(product, 'PreviewComputeSave', {'change':change})['data']
        auth_path.write_text(changed_auth)
        control(product, 'ApplyComputeSave', {'spec':preview['spec'],'accept_digest':preview['accept_digest'],
            'expected_revisions':preview['expected_revisions'],'idempotency_key':'pi-source-stale-save'},
            expected_error='CHANGE_PREVIEW_STALE')
        assert source.request_count() == 0, 'passive discovery/save issued an inference request'
        report['cases'].append({'id':CASES[1],'state':'green'})
        auth_path.write_bytes(original_auth)
        stage = 'use-imported-source'
        report['native_route'] = use_imported_route(product, binary, source, saved)
        assert source.request_count() == 1, 'imported route did not make exactly one native model request'
        assert {path:digest(path) for path in before} == before, 'model route restore changed borrowed files'
        report['cases'].append({'id':CASES[2],'state':'green'})
        product.stop()
        report.update(state='green', diagnostics=product.diagnostics_snapshot(), passive_model_posts=0, model_posts=1,
            native_files_preserved_on_import=True, saved_source_survived_restart=True,
            binaries={name:digest(product.bin / name) for name in ('hiroute','hirouted')})
    except Exception:
        report_failure(product, report, stage)
        raise
    finally:
        product.close()
        if source:
            source.close()
    print(json.dumps(report), flush=True)


if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2])
