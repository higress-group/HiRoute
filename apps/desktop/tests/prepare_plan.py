#!/usr/bin/env python3
"""Prepare a Plan through actual Local Control and protected role-all launcher.
Only an unmanaged, isolated Agent input and the current listener/owned-run fixture records are
written directly; ReleaseFacts stay daemon-embedded.
No network model call, user configuration edit, business DB/SecretStore/publication writes.
This is preparation evidence; it is not Desktop GUI acceptance.
"""
import hashlib, json, os, pathlib, secrets, select, socket, stat, statistics, subprocess, sys, time
REPO = pathlib.Path(__file__).resolve().parents[3]
BENCH_RUNS = int(os.environ.get('HIROUTE_PLAN_BENCHMARK_RUNS', '1'))
if BENCH_RUNS < 1 or BENCH_RUNS > 10: raise SystemExit('Benchmark runs must be 1..10')
BENCHMARK = 'HIROUTE_PLAN_BENCHMARK_RUNS' in os.environ
def diagnostic_decode_count(root):
 log=root/'diagnostics/daemon/current.jsonl'
 if not log.exists(): return 0
 return sum(1 for line in log.read_text().splitlines()
            if json.loads(line).get('event',{}).get('publication_timing',{}).get('stage')=='operation_decode')
# Native library tests also run in fresh managed checkouts. Build the actual daemon
# prerequisite before launching the isolated production path; never borrow old binaries.
build = subprocess.run(
 ['cargo', 'build', '--locked', '-p', 'hiroute-daemon', '--bin', 'hirouted'],
 cwd=REPO, capture_output=True, text=True)
if build.returncode:
 sys.stderr.write(build.stderr)
 raise SystemExit(build.returncode)
ROOT = pathlib.Path(sys.argv[1]).resolve()
ROOT.mkdir(mode=0o700, parents=True, exist_ok=True)
if any(ROOT.iterdir()): raise SystemExit('Preparation requires an empty isolated directory')
PROJECT = ROOT / 'agent-input'; (PROJECT / '.claude').mkdir(parents=True, mode=0o700)
SENTINEL = 'hr02-isolated-not-a-provider-credential-' + secrets.token_hex(16)
settings = PROJECT / '.claude/settings.json'
settings.write_text(json.dumps({'env': {'ANTHROPIC_BASE_URL': 'https://open.bigmodel.cn/api/anthropic', 'ANTHROPIC_AUTH_TOKEN': SENTINEL, 'ANTHROPIC_MODEL': 'glm-5.3'}}))
settings.chmod(0o600)
for directory in [ROOT/'storage', ROOT/'run']:
 directory.mkdir(mode=0o700, exist_ok=True)
environment=dict(os.environ)
for key in list(environment):
 if key.startswith('ANTHROPIC_') or key.startswith('CLAUDE_CODE_') or key == 'CLAUDE_CONFIG_DIR': del environment[key]
# The daemon and CLI must never observe the developer's real HOME: the user-layer Claude
# settings live there, and their mode or content would leak into the isolated scan.
environment['HOME']=str(PROJECT)
environment['HIROUTE_RUNTIME_DIR']=str(ROOT/'run')
shutdown_r,shutdown_w=os.pipe(); capability_r,capability_w=os.pipe(); ack_r,ack_w=os.pipe()
reservation=socket.socket();reservation.bind(('127.0.0.1',0));port=reservation.getsockname()[1];reservation.close()
record=ROOT/'gateway-listener.json'
record.write_text(json.dumps({'schema_version':'hiroute.gateway-listener/v1',
 'desired':{'address':'127.0.0.1','port_mode':'automatic','port':port},
 'applied':{'address':'127.0.0.1','port':port,'applied_at_unix':int(time.time())}}))
record.chmod(0o600)
fds=[shutdown_r,capability_r,ack_w]
daemon_args=[str(REPO/'target/debug/hirouted'),'--role','all','--storage-root',str(ROOT/'storage'),'--runtime-root',str(ROOT/'run'),'--listen',f'127.0.0.1:{port}','--lkg',str(ROOT/'gateway.lkg'),'--shutdown-fd',str(fds[0]),'--capability-fd',str(fds[1]),'--capability-ack-fd',str(fds[2])]
if BENCHMARK: daemon_args += ['--diagnostics-root',str(ROOT/'diagnostics'),'--diagnostic-level-override','debug']
daemon=subprocess.Popen(daemon_args,cwd=PROJECT,env=environment,pass_fds=fds,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
os.close(shutdown_r);os.close(capability_r);os.close(ack_w)
def call(command, payload=None, grant=None):
 # These preparation-only operations remain Planned; the installed CLI must reject them.
 operation={
  ('compute','scan'):'ScanCompute',
  ('routing','preview'):'PreviewAgentPlanChange',
  ('routing','apply'):'ApplyAgentPlanChange',
  ('routing','list'):'ListAgentPlanCatalog',
 }[tuple(command)]
 protected=None if grant is None else {'principal_kind':'interactive_user','capability':grant}
 return control(operation,payload if payload is not None else {},protected)['data']
def encoded(value):
 return json.dumps(value,sort_keys=True,separators=(',',':')).encode()
def control(operation,payload,grant=None):
 request={'schema_version':{'major':2,'minor':0},'request_id':'desktop-e2e-'+secrets.token_hex(12),'operation_id':operation,'payload':payload}
 if grant is not None: request['protected_grant']=grant
 with socket.socket(socket.AF_UNIX) as sock:
  sock.settimeout(35);sock.connect(str(ROOT/'run/hiroute/control.sock'));stream=sock.makefile('rwb')
  stream.write(encoded({'api_version':{'major':2,'minor':0},'machine_schema_version':{'major':2,'minor':0},'client_name':'desktop-e2e','client_version':'0.1.0'})+b'\n');stream.flush()
  hello=json.loads(stream.readline());assert 'local-control-v2' in hello['capabilities'],hello
  stream.write(encoded(request)+b'\n');stream.flush();raw=stream.readline()
 assert SENTINEL.encode() not in raw,'Secret escaped protected Local Control response'
 response=json.loads(raw)
 if response.get('error') is not None: raise RuntimeError(f'{operation}: {response}')
 return response
def register_grant(operation,digest,revisions,principal):
 token=secrets.token_hex(32);identity=secrets.token_hex(32)
 frame={'schema':'hiroute.protected-apply-grant/v2','registration_id':identity,'capability':token,'principal_kind':principal,'workspace_id':'personal/default','operation_kind':operation,'accepted_digest':digest,'expected_revisions':revisions,'expires_at_unix':int(time.time())+60}
 frame_bytes=json.dumps(frame).encode()+b'\n'
 assert os.write(capability_w,frame_bytes)==len(frame_bytes)
 data=b''
 while not data.endswith(b'\n'):
  if not select.select([ack_r],[],[],2)[0]: raise RuntimeError('capability acknowledgement timeout')
  data+=os.read(ack_r,1)
 received=json.loads(data)
 assert received == {'schema':'hiroute.protected-apply-ack/v2','registration_id':identity,'registered':True}
 return token
def desktop_grant(operation,digest,revisions):
 return {'principal_kind':'desktop','capability':register_grant(operation,digest,revisions,'desktop')}
def apply(command, preview, payload):
 payload.update(accept_digest=preview['change_digest'],expected_revisions=preview['expected_revisions'],idempotency_key=secrets.token_hex(32))
 result=call(command,payload)
 assert result['state']=='succeeded',result
 return result
try:
 if not select.select([daemon.stdout],[],[],30)[0]: raise RuntimeError('daemon startup deadline')
 ready=daemon.stdout.readline()
 if not ready: raise RuntimeError(daemon.stderr.read(4096).decode())
 startup=json.loads(ready)
 if 'process_id' not in startup: raise RuntimeError(f"daemon startup failed: {startup.get('code', 'unknown')}")
 assert startup['process_id']==daemon.pid
 # This preparation daemon stands in for the previously owned run. Seed its exact endpoint
 # identities while they are live, so Desktop restart can verify the old served address.
 endpoint_dir=ROOT/'run/hiroute'
 endpoint_paths=[endpoint_dir/'control.sock',endpoint_dir/'agent-grant-v1.sock']
 identities=[]
 for path in [endpoint_dir,*endpoint_paths]:
  info=path.lstat()
  assert (stat.S_ISDIR(info.st_mode) if path==endpoint_dir else stat.S_ISSOCK(info.st_mode)),path
  identities.append({'device':info.st_dev,'inode':info.st_ino})
 receipt={'version':1,'pid':daemon.pid,'directory':identities[0],
  'sockets':identities[1:],'gateway_address':f'127.0.0.1:{port}'}
 fd=os.open(ROOT/'desktop.lock',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
 with os.fdopen(fd,'w') as owned_lock: json.dump(receipt,owned_lock);owned_lock.flush();os.fsync(owned_lock.fileno())
 scan=call(['compute','scan'])
 eligible=[item for item in scan['items'] if item.get('inventory_eligible') and item.get('model_configuration_id')=='model.zhipu.glm-5.3' and item.get('discovery')]
 if len(eligible)!=1: raise RuntimeError(f'Isolated registered Agent discovery unavailable: {scan}')
 item=eligible[0]
 prepare={'discovery':item['discovery'],'prepare_id':'prepare/desktop-e2e/source'}
 status=control('GetClientServiceStatus',{})['data']
 prepare_digest='sha256:'+hashlib.sha256(encoded(prepare)).hexdigest()
 candidate=control('PrepareDiscoveredModelConnection',prepare,desktop_grant('PrepareDiscoveredModelConnection',prepare_digest,status['revisions']))['data']
 selectable=[model['model_ref'] for model in candidate['models'] if model['selectable']]
 if not selectable: raise RuntimeError(f'Prepared discovery has no selectable model: {candidate}')
 snapshot=control('ListCompute',{})['data']
 change={'schema':'hiroute.compute-management-change/v2','subject':{'kind':'candidate','candidate':candidate['candidate']},'expected_revisions':snapshot['revisions'],'selected_model_refs':selectable,'intent':'save_ready','key_edits':[]}
 preview=control('PreviewComputeSave',{'change':change})['data']
 body={'spec':preview['spec'],'accept_digest':preview['accept_digest'],'expected_revisions':preview['expected_revisions'],'idempotency_key':secrets.token_hex(32)}
 applied=control('ApplyComputeSave',body)
 assert applied['data']['state']=='succeeded',applied
 saved=control('GetComputeSaveResult',{'operation':applied['operation']})['data']
 assert saved['disposition']=='saved' and saved['management_state']=='ready' and len(saved['bindings'])==1,saved
 binding=dict(saved['bindings'][0],source_id=saved['source_id'],source_revision=saved['saved_revision'])
 projection={'binding':binding}
 (ROOT/'compute-projection.json').write_text(json.dumps(projection,indent=2))
 print('compute projection created',flush=True)
 print(json.dumps(binding),flush=True)
 before_plan_decodes=diagnostic_decode_count(ROOT) if BENCHMARK else 0
 editor={'schema':'hiroute.plan-editor/v2','display_name':'Desktop acceptance A','purpose':'Desktop isolated reversible rename','mode':'fixed_model',
         'candidates':[{'binding_id':binding['binding_id']}], 'delegation_enabled':False,
         'smart':{'economy':[],'primary':[],'primary_fallback':False,'reselect_on_user_message':False,'classifier':{'kind':'local_rules'},'complex_keywords':[]},
         'free':{'candidates':[],'primary':[],'primary_fallback':False},'requirements':{},
         'limits':{'maximum_attempts':1,'request_timeout_ms':30000,'attempt_timeout_ms':30000}}
 durations_ms=[]
 for index in range(BENCH_RUNS):
  current_editor=dict(editor)
  if BENCHMARK: current_editor['display_name']=f'Desktop acceptance {index}'
  creation_key=f'desktop-acceptance-{index}' if BENCHMARK else 'desktop-acceptance'
  change={'schema':'hiroute.plan-content-change/v2','target':{'intent':'create','creation_key':creation_key},'editor':current_editor,'consumed_draft':None}
  preview=call(['routing','preview'],{'change':change})
  started=time.perf_counter_ns()
  operation=apply(['routing','apply'],preview,{'change':change})
  durations_ms.append(round((time.perf_counter_ns()-started)/1_000_000,3))
 baseline=call(['routing','list'])
 (ROOT/'baseline.json').write_text(json.dumps(baseline,indent=2))
 print(json.dumps({'state':'prepared','operation_id':operation['operation_id'],'plans':len(baseline['plans']),'root':str(ROOT)}),flush=True)
 if BENCHMARK: print(json.dumps({'benchmark':'plan_apply','runs':BENCH_RUNS,'apply_ms':durations_ms,'median_ms':statistics.median(durations_ms)}),flush=True)
finally:
 os.close(shutdown_w)
 try: daemon.wait(timeout=35)
 except subprocess.TimeoutExpired: daemon.kill();daemon.wait()
 os.close(capability_w);os.close(ack_r)
if BENCHMARK:
 stages={}
 diagnostic_log=ROOT/'diagnostics/daemon/current.jsonl'
 if diagnostic_log.exists():
  for line in diagnostic_log.read_text().splitlines():
   event=json.loads(line).get('event',{}).get('publication_timing')
   if event is not None:
    stages.setdefault(event['stage'],[]).append(event['elapsed_us'])
 print(json.dumps({'benchmark':'diagnostic_stages','counts':{stage:len(values) for stage,values in sorted(stages.items())},'median_us':{stage:statistics.median(values) for stage,values in sorted(stages.items())}}),flush=True)
 plan_decodes=len(stages.get('operation_decode',[]))-before_plan_decodes
 print(json.dumps({'benchmark':'plan_operation_decode','runs':BENCH_RUNS,'count':plan_decodes}),flush=True)
 if os.environ.get('HIROUTE_PLAN_BENCHMARK_EXPECT_ZERO_DECODE')=='1':
  assert plan_decodes==0,f'Plan apply decoded full Operation {plan_decodes} times'
