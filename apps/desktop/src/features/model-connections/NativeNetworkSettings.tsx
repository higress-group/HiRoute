import { useEffect, useId, useRef, useState } from 'react';
import { Disclosure } from '../../ui/Disclosure';
import { invoke } from '../../host/index.ts';
import { manualProxyError, networkEdit, networkFailureText, networkSummary, type NativeNetworkDraft, type NativeNetworkFailure, type NativeNetworkPolicy, type NativeNetworkRoute, type NativeNetworkTarget } from './native-network';
import './native-network.css';

export function NativeNetworkSettings({ value, onChange, language, target, disabled, reveal = 0, failure }: {
  value: NativeNetworkDraft; onChange(value: NativeNetworkDraft): void; language: 'zh' | 'en';
  target: NativeNetworkTarget; disabled?: boolean; reveal?: number; failure?: NativeNetworkFailure | null;
}) {
  const zh = language === 'zh', id = useId();
  const [open, setOpen] = useState(false), [route, setRoute] = useState<NativeNetworkRoute | null>(null), [previewFailed, setPreviewFailed] = useState(false);
  const heading = useRef<HTMLElement>(null);
  const policy = networkEdit(value).policy;
  const invalid = manualProxyError(value);
  const context = JSON.stringify({ target, policy });
  useEffect(() => {
    let current = true; setRoute(null); setPreviewFailed(false);
    if (invalid || (target.kind === 'draft' && !target.endpoint_url)) return;
    const timer = window.setTimeout(() => {
      void invoke<NativeNetworkRoute>('preview_native_network_route', { input: { target, policy } })
        .then(result => { if (current) setRoute(result); }).catch(() => { if (current) setPreviewFailed(true); });
    }, 250);
    return () => { current = false; window.clearTimeout(timer); };
  }, [context, invalid]);
  useEffect(() => { if (reveal > 0) { setOpen(true); requestAnimationFrame(() => heading.current?.focus()); } }, [reveal]);
  const detail = invalid ? (zh ? '代理地址需要修正' : 'Proxy URL needs correction') : route?.failure ? networkFailureText(route.failure, zh) : route?.route === 'proxy' ? (zh ? '此地址将经过代理' : 'This destination will use the proxy') : route?.reason === 'no_proxy_configured' ? (zh ? '服务环境未配置此协议的代理，此地址将直连' : 'No service proxy is configured for this protocol; this destination will connect directly') : route?.reason === 'bypass_matched' || route?.reason === 'loopback' ? (zh ? '此地址匹配绕过规则，将直连' : 'This destination bypasses the proxy and connects directly') : route ? (zh ? '此地址将直连' : 'This destination will connect directly') : previewFailed ? (zh ? '暂时无法预览路由' : 'Route preview unavailable') : '';
  return <Disclosure className="native-network" language={language} open={open} onOpenChange={setOpen} summaryRef={heading} label={<span><strong>{zh ? '网络与代理' : 'Network and proxy'}</strong><span className="field-help">{networkSummary(policy, zh)}{detail ? ` · ${detail}` : ''}</span></span>}>
    <fieldset className="native-network-fields" disabled={disabled}>
      <legend className="sr-only">{zh ? '代理策略' : 'Proxy policy'}</legend>
      <label className="field" htmlFor={`${id}-mode`}><span className="field-label">{zh ? '连接方式' : 'Connection mode'}</span>
        <select className="input" id={`${id}-mode`} value={value.mode} onChange={event => onChange({ ...value, mode: event.target.value as NativeNetworkDraft['mode'] })}>
          <option value="direct">{zh ? '直连（默认）' : 'Direct (default)'}</option><option value="inherit">{zh ? '使用服务环境代理' : 'Use service environment proxy'}</option><option value="manual">{zh ? '手动设置代理' : 'Set proxy manually'}</option>
        </select>
      </label>
      {value.mode === 'inherit' && <p className="field-help">{zh ? '使用 HiRoute 服务所在机器启动时的 HTTP_PROXY、HTTPS_PROXY 和 NO_PROXY。修改启动环境后需重启服务。' : 'Use HTTP_PROXY, HTTPS_PROXY and NO_PROXY captured when the HiRoute service started on its machine. Restart the service after changing that environment.'}</p>}
      {value.mode === 'manual' && <>
        <label className="field" htmlFor={`${id}-url`}><span className="field-label">{zh ? '代理地址' : 'Proxy URL'}</span><input className="input" id={`${id}-url`} type="url" autoComplete="off" spellCheck={false} value={value.proxy_url} placeholder="http://127.0.0.1:7890" aria-invalid={invalid === 'proxy_url' || undefined} aria-describedby={`${id}-url-help`} onChange={event => onChange({ ...value, proxy_url: event.target.value })} />
          <span className="field-help" id={`${id}-url-help`}>{invalid ? zh ? '请填写 http:// 或 https:// 代理地址，不含账号、密码、路径或查询参数。' : 'Use an http:// or https:// proxy URL without credentials, a path or query.' : zh ? '需要认证时，请在服务启动环境中配置代理，并选择“使用服务环境代理”。' : 'For authenticated proxies, configure the service startup environment and select the environment mode.'}</span></label>
        <label className="field" htmlFor={`${id}-bypass`}><span className="field-label">{zh ? '不经过代理的地址（可选）' : 'Bypass proxy for (optional)'}</span><textarea className="input" id={`${id}-bypass`} rows={2} maxLength={4096} aria-invalid={route?.failure?.code === 'native.network.bypass_invalid' || undefined} value={value.no_proxy} placeholder="example.com, 192.0.2.0/24" onChange={event => onChange({ ...value, no_proxy: event.target.value })} /><span className="field-help">{route?.failure?.code === 'native.network.bypass_invalid' ? (zh ? '绕过规则无效，请使用逗号分隔有效域名、IP、网段及可选端口。' : 'Invalid bypass rules. Use comma-separated domains, IPs or networks with optional ports.') : zh ? '用逗号分隔域名、IP 或网段，可指定端口。localhost 和回环地址始终直连。' : 'Comma-separated domains, IPs or networks, with optional ports. Localhost and loopback addresses always connect directly.'}</span></label>
      </>}
      {value.mode === 'manual' && <p className="field-help">{zh ? '代理地址指向 HiRoute 服务所在机器；远程控制台中的 127.0.0.1 不是浏览器所在机器。' : 'The proxy address is resolved on the HiRoute service machine. In a remote console, 127.0.0.1 refers to that machine, not the browser.'}</p>}
      <p className="field-help">{zh ? '仅影响这份接入的请求；保存后对后续请求生效，在途请求继续使用原路径。' : 'Applies to this connection. Saving affects subsequent requests; requests in progress retain their route.'}</p>
      {failure && <p className="callout warn" role="alert">{networkFailureText(failure, zh)}</p>}
      {route && <p className="field-help" role="status">{detail} · {zh ? '路由预览，尚未检测连通性' : 'Route preview; connectivity has not been checked'}</p>}
    </fieldset>
  </Disclosure>;
}

export function NativeNetworkImpact({ before, after, models, decisions = [], language }: { before: NativeNetworkPolicy; after: NativeNetworkPolicy; models: { display_name: string }[]; decisions?: { service_id: string; revision: number; display_name: string; model: string }[]; language: 'zh' | 'en' }) {
  const zh = language === 'zh';
  const describe = (policy: NativeNetworkPolicy) => <>{networkSummary(policy, zh)}{policy.mode === 'manual' && <><code>{policy.proxy_url}</code><p className="field-help">{zh ? '绕过代理：' : 'Bypass proxy: '}<code>{policy.no_proxy || (zh ? '仅回环地址' : 'Loopback only')}</code></p></>}</>;
  return <section className="native-network-impact" aria-label={zh ? '代理变更影响' : 'Network change impact'}>
    <h3>{zh ? '确认修改这份接入的网络设置' : 'Review this connection’s network change'}</h3>
    <dl><div><dt>{zh ? '保存前' : 'Before'}</dt><dd>{describe(before)}</dd></div><div><dt>{zh ? '本次保存' : 'Proposed'}</dt><dd>{describe(after)}</dd></div></dl>
    <p>{zh ? `将影响此接入的 ${models.length} 个模型：` : `Affects ${models.length} models in this connection:`}</p><ul>{models.map((model, i) => <li key={i}>{model.display_name}</li>)}</ul>
    {decisions.length > 0 && <><p>{zh ? `还会影响 ${decisions.length} 份共享接入的决策模型配置：` : `Also affects ${decisions.length} shared-source decision configurations:`}</p><ul>{decisions.map(decision => <li key={`${decision.service_id}/${decision.revision}`}>{decision.display_name} · {decision.model} · {zh ? '版本' : 'revision'} {decision.revision}</li>)}</ul></>}
    <p className="field-help">{zh ? '后续请求立即使用新设置，在途请求保留原路径。保存配置并不代表连通性已验证。' : 'Subsequent requests use the new settings immediately; requests in progress retain their route. Saving does not verify connectivity.'}</p>
  </section>;
}
