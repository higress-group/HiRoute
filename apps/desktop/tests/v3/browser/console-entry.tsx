import React from 'react';
import { createRoot } from 'react-dom/client';
import { ConsoleRoot } from '../../../src/host/ConsoleRoot';
import { consoleTransport } from '../../../src/host/console';
import { hostCapabilities, setHostTransport } from '../../../src/host/index.ts';
import { browserCapabilities } from '../../../src/host/types';
import { DesktopApp } from '../../../src/product/DesktopApp';
import { mockProductInvoke } from './product-fixtures';
import '../../../src/occami/styles.css';

const query = new URLSearchParams(location.search);
const state = query.get('case') ?? 'login';
localStorage.setItem('hiroute.language', query.get('language') ?? 'zh');
localStorage.setItem('hiroute.theme', query.get('theme') ?? 'light');
sessionStorage.clear();
if (!['login', 'launch', 'rate-limited', 'session-limit'].includes(state)) sessionStorage.setItem('hiroute.console.session', 'b'.repeat(64));
if (state === 'launch') history.replaceState(null, '', '/launch?code=' + 'a'.repeat(64));
const control = {
  requests: [] as { path: string; headers: Record<string, string>; credentials?: RequestCredentials; url: string; body: unknown }[],
  expired: state === 'expired', unavailable: state === 'unavailable',
  errors: [] as string[],
  invoke: consoleTransport.call,
  hostRequests: [] as {command:string;args:Record<string,unknown>}[],
  loginEnabled: true,
  capabilities: hostCapabilities,
};
if (query.get('host') === 'shell-console') {
  const view = { current_version:'0.3.1',phase:query.get('update')??'ready',available:{version:'0.4.0',notes:{zh:'更新说明',en:'Release notes'},size:1000},downloaded_bytes:1000,active_calls:1,active_tasks:2,error:query.get('update')==='failed'?'UPGRADE_INSTALL_FAILED':null,can_install:true,package_ready:true,recovery:query.get('update')==='failed'?'service_available':null };
  setHostTransport({ capabilities:()=>({...browserCapabilities,host:'shell-console',platform:'windows',updates:true,login_start:true}),
    call:async <T,>(command:string,args:Record<string,unknown>={})=>{
      if (command.startsWith('update_') || command.startsWith('login_start_') || ['open_in_browser','restart_service','reconnect'].includes(command)) {
        control.hostRequests.push({command,args});
        if (command==='login_start_set') control.loginEnabled=Boolean(args.enabled);
        if (command==='update_cancel') { view.phase='ready';view.active_calls=0;view.active_tasks=0; }
        return (command.startsWith('update_')?view:command.startsWith('login_start_')?{enabled:control.loginEnabled}:null) as T;
      }
      return consoleTransport.call<T>(command,args);
    },
  });
}
const networkFetch = window.fetch.bind(window);
window.fetch = async (input, options) => {
  const url = new URL(String(input), location.href);
  if (!url.pathname.startsWith('/api/console/v1/')) return networkFetch(input, options);
  const headers = new Headers(options?.headers);
  control.requests.push({ path: url.pathname, headers: Object.fromEntries(headers), credentials: options?.credentials, url: location.href, body: options?.body ? JSON.parse(String(options.body)) : null });
  if (control.unavailable) throw new TypeError('Synthetic disconnected service');
  const reply = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
  if (/\/session\/(launch|token)$/.test(url.pathname)) {
    if (state === 'rate-limited') return reply({ kind: 'native', code: 'CONSOLE_RATE_LIMITED' }, 429);
    if (state === 'session-limit') return reply({ kind: 'native', code: 'CONSOLE_SESSION_LIMIT' }, 503);
    return reply({ bearer: 'b'.repeat(64) });
  }
  if (control.expired) return reply({ kind: 'native', code: 'CONSOLE_AUTH_REQUIRED' }, 401);
  if (url.pathname.endsWith('/session')) return reply({
    session: { id: 'session/component', credential_kind: 'access_token', token_id: 'token/component', expires: 9999999999 },
    transport: query.get('host') === 'shell-console' ? 'loopback' : 'network_plaintext', assets_available: state !== 'assets', capabilities: { platform: 'linux', diagnostic_directory: '/example/diagnostics', recovery_directory: '/example/service' },
  });
  const command = decodeURIComponent(url.pathname.split('/').at(-1)!);
  const args = options?.body ? JSON.parse(String(options.body)) : {};
  if (command === 'observe_operation' || command === 'web_confirmation_snapshot') return reply(null);
  if (command === 'gateway_listener_status') return reply({config:{schema_version:'hiroute.gateway-listener/v1',desired:{address:'127.0.0.1',port_mode:'automatic'},applied:{address:'127.0.0.1',port:5829,applied_at_unix:1}},connect_address:'127.0.0.1:5829',suggested_agent_base_url:'http://127.0.0.1:5829',existing_agent_count:0,ready:true});
  if (command === 'worker_task_list') return reply({ status: 'succeeded', data: { schema: 'hiroute.worker-task-list/v1', tasks: [], next_cursor: null }, next_actions: [], error: null });
  try { return reply(await mockProductInvoke(command, args, 'ready') ?? null); }
  catch (error) { control.errors.push(String(error)); return reply({ kind: 'native', code: 'FIXTURE_COMMAND_UNHANDLED' }, 500); }
};
Object.assign(window, { consoleComponent: control });
createRoot(document.getElementById('root')!).render(<ConsoleRoot><DesktopApp /></ConsoleRoot>);
