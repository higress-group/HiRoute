// HiRoute's ACP transport for the explicitly selected official Pi npm SDK.
// The daemon owns admission, credentials, process groups, retention and task state.
import {readFileSync, realpathSync, lstatSync, openSync, readSync, closeSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createInterface} from 'node:readline';

process.umask(0o077);
const output = value => process.stdout.write(JSON.stringify(value) + '\n');
const reply = (request, result) => output({jsonrpc:'2.0',id:request.id,result});
const reject = request => output({jsonrpc:'2.0',id:request.id,error:{code:-32000,message:'Pi managed operation unavailable'}});
let session;
let active = false;
let cancelled = false;

function messageValid(message) {
  if (!message || !Number.isFinite(message.timestamp)) return false;
  const content=message.content;
  if (message.role==='system') return typeof content==='string'
    && (content.length>0 || message.sections && typeof message.sections==='object');
  if (message.role==='user' && typeof content==='string') return true;
  if (!['user','assistant','toolResult'].includes(message.role) || !Array.isArray(content)) return false;
  for (const part of content) {
    if (!part || typeof part.type!=='string') return false;
    if (part.type==='text') {if (typeof part.text!=='string') return false;}
    else if (part.type==='thinking') {if (typeof part.thinking!=='string') return false;}
    else if (part.type==='image') {if (typeof part.data!=='string' || typeof part.mimeType!=='string') return false;}
    else if (part.type==='toolCall') {
      if (typeof part.id!=='string' || typeof part.name!=='string' || !part.arguments
          || typeof part.arguments!=='object' || Array.isArray(part.arguments)) return false;
    } else return false;
  }
  if (message.role==='assistant') return typeof message.api==='string' && typeof message.provider==='string'
    && typeof message.model==='string' && message.usage && typeof message.usage==='object'
    && ['stop','length','toolUse','error','aborted'].includes(message.stopReason);
  if (message.role==='toolResult') return typeof message.toolCallId==='string'
    && typeof message.toolName==='string' && typeof message.isError==='boolean';
  return true;
}

function ownedFile(path, root) {
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || realpathSync(path) !== path
      || dirname(path) !== root || stat.uid !== process.getuid() || (stat.mode & 0o077) !== 0) throw new Error();
  if (stat.size>64*1024*1024) throw new Error();
  const lines=readFileSync(path,'utf8').split('\n');
  if(lines.pop()!=='')throw new Error();
  const entries=lines.map(line=>JSON.parse(line));
  if(entries.length>100000 || lines.some(line=>Buffer.byteLength(line)>8*1024*1024))throw new Error();
  if(entries.length<2 || !entries.slice(1).some(entry=>entry.type==='message' || entry.type==='compaction'))throw new Error();
  const seen=new Set();
  for(const entry of entries.slice(1)) {
    if(typeof entry.id!=='string' || !entry.id || seen.has(entry.id) || !Number.isFinite(Date.parse(entry.timestamp))
       || entry.parentId!==null && !seen.has(entry.parentId))throw new Error();
    if(entry.type==='message') {if (!messageValid(entry.message))throw new Error();}
    else if(entry.type==='model_change') {if (typeof entry.provider!=='string' || typeof entry.modelId!=='string')throw new Error();}
    else if(entry.type==='thinking_level_change') {if (typeof entry.thinkingLevel!=='string')throw new Error();}
    else if(entry.type==='compaction') {
      if(typeof entry.summary!=='string' || !entry.summary || !seen.has(entry.firstKeptEntryId)
         || !Number.isFinite(entry.tokensBefore) || entry.tokensBefore<0)throw new Error();
    } else if(entry.type==='usage') {if (typeof entry.kind!=='string' || typeof entry.provider!=='string'
         || typeof entry.model!=='string' || !entry.usage || typeof entry.usage!=='object')throw new Error();}
    else throw new Error();
    seen.add(entry.id);
  }
  const fd = openSync(path, 'r');
  try {
    const bytes = Buffer.alloc(8192); const count = readSync(fd, bytes, 0, bytes.length, 0);
    const end = bytes.subarray(0,count).indexOf(10); if (end < 0) throw new Error();
    const header = JSON.parse(bytes.subarray(0,end).toString('utf8'));
    if (header.type !== 'session' || header.version!==3 || typeof header.id !== 'string' || !header.id
        || !Number.isFinite(Date.parse(header.timestamp)) || header.cwd !== process.cwd()) throw new Error();
    return header;
  } finally {closeSync(fd);}
}

try {
  const route = JSON.parse(process.env.HIROUTE_PI_ROUTE);
  const nodeVersion=process.versions.node.split(".").map(Number);
  if (nodeVersion.some((part,index)=> index===0 ? part<route.minimumNode[0] : nodeVersion[0]===route.minimumNode[0] && index===1 && part<route.minimumNode[1])) throw new Error();
  const cli = realpathSync(process.argv[2]);
  const packageRoot = dirname(dirname(dirname(cli)));
  const pkg = JSON.parse(readFileSync(join(packageRoot,'package.json'),'utf8'));
  if (pkg.name !== route.cliPackage || pkg.version !== route.cliVersion
      || realpathSync(join(packageRoot,pkg.bin.pi)) !== cli) throw new Error();
  const sdk = await import(pathToFileURL(join(packageRoot,'dist/index.js')).href);
  const {createAgentSession,ModelRuntime,SettingsManager,SessionManager,DefaultResourceLoader} = sdk;
  const agentDir = resolve(process.env.PI_CODING_AGENT_DIR);
  const root = realpathSync(process.env.HIROUTE_PI_SESSION_ROOT);
  const file = join(root,'native-pi.jsonl');
  const token = process.env.HIROUTE_RUN_TOKEN;
  delete process.env.HIROUTE_RUN_TOKEN;
  delete process.env.HIROUTE_PI_ROUTE;
  const modelId = route.provider + '/' + route.model.id;
  const credentials=new Map();
  const runtime = await ModelRuntime.create({modelsPath:null,credentials:{
    async read(id){return credentials.get(id);},async list(){return [];},
    async modify(id,fn){const next=await fn(credentials.get(id));if(next!==undefined)credentials.set(id,next);return credentials.get(id);},
    async delete(id){credentials.delete(id);}
  }});
  runtime.registerProvider(route.provider,{api:'openai-responses',baseUrl:route.endpoint,models:[route.model]});
  await runtime.setRuntimeApiKey(route.provider,token);
  const model = runtime.getModels(route.provider).find(item=>item.id===route.model.id);
  if (!model) throw new Error();

  // Read native resource settings without writing them. Snapshot each scope, so reload cannot
  // undo host overrides or turn relative project Skill paths into user paths.
  const raw = SettingsManager.create(process.cwd(),agentDir,{projectTrusted:true});
  const snapshots = new Map(['global','project'].map(scope=>{
    const source = scope==='global' ? raw.getGlobalSettings() : raw.getProjectSettings();
    return [scope, JSON.stringify({...source,extensions:[],httpProxy:undefined,
      cacheWarming:'off',retry:{enabled:false},
      compaction:{enabled:true,reserveTokens:route.model.maxTokens,
        keepRecentTokens:Math.min(Math.floor(route.model.contextWindow/4),20000),modelOverrides:{}}})];
  }));
  const settingsManager = SettingsManager.fromStorage({withLock(scope,fn){
    const next=fn(snapshots.get(scope)); if (next!==undefined) snapshots.set(scope,next);
  }},{projectTrusted:true});
  const resourceLoader = new DefaultResourceLoader({cwd:process.cwd(),agentDir,settingsManager,noExtensions:true});
  await resourceLoader.reload();

  function configuration() {return [{id:'model',name:'Model',type:'select',category:'model',
    currentValue:session.model.provider+'/'+session.model.id,options:[{value:modelId,name:'Frozen Plan'}]}];}
  function ready() {return {sessionId:session.sessionId,_meta:{agentSessionId:session.sessionId},
    configOptions:configuration(),modes:{currentModeId:'approve-all',availableModes:[{id:'approve-all',name:'Approve all'}]}};}
  function requireSession(params) {if (!session || params.sessionId!==session.sessionId) throw new Error();}
  async function dispatch(request) {
    const p=request.params||{};
    try {
      if (request.method==='initialize') {if (p.protocolVersion!==1) throw new Error();
        reply(request,{protocolVersion:1,agentInfo:{name:'hiroute-pi-sdk',version:pkg.version},agentCapabilities:{loadSession:true}});return;}
      if (request.method==='session/new' || request.method==='session/load') {
        if (session || realpathSync(p.cwd)!==process.cwd() || p.mcpServers?.length) throw new Error();
        let manager;
        if (request.method==='session/load') {
          const header=ownedFile(file,root); if (header.id!==p.sessionId) throw new Error();
          manager=SessionManager.open(file,root);
          if (manager.getSessionId()!==header.id || manager.getSessionFile()!==file || manager.buildSessionContext().messages.length===0) throw new Error();
        } else {
          try {lstatSync(file);throw new Error('already-exists');} catch(error) {if (error.code!=='ENOENT') throw error;}
          manager=SessionManager.create(process.cwd(),root); manager.setSessionFile(file);
        }
        ({session}=await createAgentSession({cwd:process.cwd(),agentDir,modelRuntime:runtime,model,
          thinkingLevel:'off',settingsManager,resourceLoader,sessionManager:manager,
          tools:['read','bash','edit','write','grep','find','ls']}));
        // SDK load may restore its previous model. Frozen route selection is explicit.
        await session.setModel(model);
        if (session.model?.provider!==route.provider || session.model?.id!==route.model.id) throw new Error();
        session.subscribe(event=>{
          if (active && event.type==='message_update' && event.assistantMessageEvent?.type==='text_delta') {
            output({jsonrpc:'2.0',method:'session/update',params:{sessionId:session.sessionId,
              update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text:event.assistantMessageEvent.delta}}}});
          }
        });
        reply(request,ready());return;
      }
      requireSession(p);
      if (request.method==='session/set_mode') {if (p.modeId!=='approve-all') throw new Error();reply(request,{});return;}
      if (request.method==='session/set_config_option') {
        if (p.configId!=='model' || p.value!==modelId) throw new Error();
        reply(request,{configOptions:configuration()});return;
      }
      if (request.method==='session/cancel') {cancelled=true;session.clearQueue();await session.abort();return;}
      if (request.method==='session/prompt') {
        if (active || !Array.isArray(p.prompt) || p.prompt.some(part=>part.type!=='text')) throw new Error();
        const text=p.prompt.map(part=>part.text).join('\n'); if (!text || text.length>256*1024) throw new Error();
        active=true;cancelled=false;
        try {
          await session.prompt(text);
          const last=[...session.messages].reverse().find(item=>item.role==='assistant');
          if (!cancelled && last?.stopReason!=='stop') throw new Error();
          const header=ownedFile(file,root);if (header.id!==session.sessionId) throw new Error();
          reply(request,{stopReason:cancelled?'cancelled':'end_turn'});
        } finally {active=false;}
        return;
      }
      throw new Error();
    } catch {if (request.id!==undefined) reject(request);}
  }
  const input=createInterface({input:process.stdin,crlfDelay:Infinity});
  for await (const line of input) {
    if (Buffer.byteLength(line)>8*1024*1024) throw new Error();
    const request=JSON.parse(line);void dispatch(request);
  }
  if (session) {session.clearQueue();await session.abort();session.dispose();}
} catch {
  process.stderr.write('HiRoute Pi SDK bridge unavailable\n');process.exitCode=1;
}
