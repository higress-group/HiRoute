// Shared by bounded local admission and the managed Worker. Release labels are diagnostic.
import {readFileSync, realpathSync, statSync, mkdtempSync, mkdirSync, writeFileSync, rmSync} from 'node:fs';
import {dirname, join, sep} from 'node:path';
import {tmpdir} from 'node:os';
import {pathToFileURL, fileURLToPath} from 'node:url';

export async function loadPiSdk(selectedCli) {
  const cli = realpathSync(selectedCli);
  let root = dirname(cli);
  for (let depth = 0; depth < 8; depth++, root = dirname(root)) {
    const manifest = join(root, 'package.json');
    let pkg;
    try {
      if (statSync(manifest).size > 64 * 1024) throw new Error();
      pkg = JSON.parse(readFileSync(manifest, 'utf8'));
    } catch { continue; }
    if (pkg.name !== '@earendil-works/pi-coding-agent') continue;
    if (typeof pkg.version !== 'string' || !pkg.version || pkg.version.length > 128
        || typeof pkg.bin?.pi !== 'string' || realpathSync(join(root, pkg.bin.pi)) !== cli
        || !cli.startsWith(root + sep)) throw new Error('Pi CLI identity unavailable');
    const entry = typeof pkg.exports === 'string' ? pkg.exports : pkg.exports ? pkg.exports['.'] : pkg.main;
    const path = typeof entry === 'string' ? entry : entry?.import ?? entry?.default;
    if (typeof path !== 'string') throw new Error('Pi SDK export unavailable');
    const sdkPath = realpathSync(join(root, path));
    if (!sdkPath.startsWith(root + sep)) throw new Error('Pi SDK source mismatch');
    return {sdk: await import(pathToFileURL(sdkPath).href), pkg};
  }
  throw new Error('Pi CLI identity unavailable');
}

function methods(object, names) {
  if (names.some(name => typeof object?.[name] !== 'function')) throw new Error('Pi capability unavailable');
}

export function assertPiCapabilities(sdk, scope) {
  if (scope === 'models') {
    methods(sdk.ModelRuntime, ['create']);
    methods(sdk.ModelRuntime?.prototype, ['getModels']);
    return;
  }
  if (!['collaboration', 'worker', 'continue'].includes(scope)) throw new Error('Pi capability scope');
  methods(sdk.SettingsManager, ['fromStorage']);
  methods(sdk.DefaultResourceLoader?.prototype, ['reload', 'getSkills']);
  methods(sdk, ['createReadTool', 'createBashTool']);
  if (scope === 'collaboration') return;
  methods(sdk.SettingsManager, ['create']);
  methods(sdk.SettingsManager?.prototype, ['getGlobalSettings', 'getProjectSettings']);
  assertPiCapabilities(sdk, 'models');
  methods(sdk.ModelRuntime?.prototype, ['registerProvider', 'setRuntimeApiKey']);
  methods(sdk, ['createAgentSession']);
  methods(sdk.SessionManager, scope === 'continue' ? ['open'] : ['create']);
  methods(sdk.SessionManager?.prototype, ['getSessionId', 'getSessionFile', 'buildSessionContext']);
  if (scope === 'worker') methods(sdk.SessionManager?.prototype, ['setSessionFile']);
  methods(sdk.AgentSession?.prototype, ['setModel', 'subscribe', 'prompt', 'clearQueue', 'abort', 'dispose']);
  // The adapter owns a v3 history reader/writer contract; it never asks the SDK to migrate.
  if (sdk.CURRENT_SESSION_VERSION !== 3) throw new Error('Pi session format unavailable');
}

export async function probePiCapabilities(sdk, scope) {
  assertPiCapabilities(sdk, scope);
  const root = mkdtempSync(join(tmpdir(), 'hiroute-pi-cap-'));
  try {
    if (scope !== 'models') {
      const skill = join(root, 'skills/hiroute-capability/SKILL.md');
      mkdirSync(dirname(skill), {recursive:true, mode:0o700});
      writeFileSync(skill, '---\nname: hiroute-capability\ndescription: Local interface check\n---\nLocal receipt.\n', {mode:0o600});
      const settings = JSON.stringify({extensions:[], packages:[], retry:{enabled:false}, cacheWarming:'off'});
      const settingsManager = sdk.SettingsManager.fromStorage({withLock(_scope, fn){return fn(settings);}}, {projectTrusted:true});
      const loader = new sdk.DefaultResourceLoader({cwd:root, agentDir:root, settingsManager, noExtensions:true});
      await loader.reload();
      if (!loader.getSkills()?.skills?.some(value => value.name === 'hiroute-capability'
          && realpathSync(value.filePath) === realpathSync(skill))) throw new Error('Pi Skill loading unavailable');
      for (const [factory, name] of [[sdk.createReadTool, 'read'], [sdk.createBashTool, 'bash']]) {
        const tool = factory(root);
        if (tool.name !== name || typeof tool.execute !== 'function') throw new Error('Pi native tool unavailable');
      }
      return;
    }
    const model = {id:'capability', name:'Local capability', api:'openai-responses',
      contextWindow:16384, maxTokens:4096, input:['text'], reasoning:false};
    const path = join(root, 'models.json');
    writeFileSync(path, JSON.stringify({providers:{'hiroute-capability':{
      api:'openai-responses', baseUrl:'http://127.0.0.1:1/v1', apiKey:'local-noncredential', models:[model]
    }}}), {mode:0o600});
    const credentials = new Map();
    const runtime = await sdk.ModelRuntime.create({modelsPath:path, allowModelNetwork:false,
      refreshOnCreate:false, credentials:{async read(id){return credentials.get(id);},
        async list(){return [];},async modify(id,fn){return fn(credentials.get(id));},async delete(){}}});
    const observed = runtime.getModels('hiroute-capability').find(value => value.id === model.id);
    if (!observed || observed.provider !== 'hiroute-capability' || observed.api !== model.api
        || observed.baseUrl !== 'http://127.0.0.1:1/v1' || observed.contextWindow !== model.contextWindow
        || observed.maxTokens !== model.maxTokens) throw new Error('Pi model configuration unavailable');
  } finally {rmSync(root, {recursive:true, force:true});}
}

if (process.argv[2] === '--check' && fileURLToPath(import.meta.url) === realpathSync(process.argv[1])) {
  process.umask(0o077);
  try {
    const {sdk} = await loadPiSdk(process.argv[3]);
    await probePiCapabilities(sdk, process.argv[4]);
    process.stdout.write('hiroute.pi-sdk-capability/v1:ok\n');
  } catch {process.exitCode = 1;}
}
