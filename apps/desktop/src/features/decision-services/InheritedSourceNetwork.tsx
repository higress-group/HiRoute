import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { NativeSourceNetworkEditor } from '../model-connections/NativeSourceNetworkEditor';
import { networkFailureText, networkSummary, type NativeNetworkRoute, type NativeNetworkTarget, type NativeSourceNetwork } from '../model-connections/native-network';

/** A compact inherited-policy fact; the source editor opens only on an explicit action. */
export function InheritedSourceNetwork({ sourceId, target, route: providedRoute, language, mutable, refreshKey = 0, reveal = 0, onSaved, onDirty }: {
  sourceId: string; target: NativeNetworkTarget; route?: NativeNetworkRoute | null;
  language: 'zh' | 'en'; mutable: boolean; refreshKey?: number; reveal?: number;
  onSaved?(): void | Promise<void>; onDirty?(dirty: boolean): void;
}) {
  const zh = language === 'zh', id = useId();
  const [route, setRoute] = useState<NativeNetworkRoute | null>(null), [loading, setLoading] = useState(false);
  const [open, setOpen] = useState(false), [opened, setOpened] = useState(false), [dirty, setDirty] = useState(false), [revision, setRevision] = useState(0);
  const [editorReveal, setEditorReveal] = useState(0);
  const callback = useRef(onDirty); callback.current = onDirty;
  const context = JSON.stringify({ sourceId, target, refreshKey, revision });
  function networkChanged(dirty: boolean) { setDirty(dirty); callback.current?.(dirty); }
  useEffect(() => () => callback.current?.(false), []);
  useEffect(() => { if (reveal > 0) { setOpened(true); setOpen(true); setEditorReveal(value => value + 1); } }, [reveal]);
  useEffect(() => {
    if (providedRoute !== undefined) return;
    let current = true; setRoute(null); setLoading(true);
    const timer = window.setTimeout(() => {
      // A draft decision URL uses the selected source policy, without granting or contacting that URL.
      void invoke<NativeSourceNetwork>('native_source_network', { input: { source_id: sourceId } })
        .then(source => invoke<NativeNetworkRoute>('preview_native_network_route', { input: { target, policy: source.policy } }))
        .then(value => { if (current) setRoute(value); }).catch(() => {})
        .finally(() => { if (current) setLoading(false); });
    }, 250);
    return () => { current = false; window.clearTimeout(timer); };
  }, [context, providedRoute]);
  const current = providedRoute === undefined ? route : providedRoute;
  const mode = current ? networkSummary(current, zh) : '';
  const detail = current?.failure ? networkFailureText(current.failure, zh) : current?.route === 'proxy' ? zh ? '此决策地址将经过代理' : 'This decision URL uses the proxy'
    : current?.reason === 'loopback' || current?.reason === 'bypass_matched' ? zh ? '此决策地址绕过代理' : 'This decision URL bypasses the proxy'
      : current ? zh ? '此决策地址将直连' : 'This decision URL connects directly'
        : loading ? zh ? '正在读取网络设置…' : 'Reading network settings…' : zh ? '网络状态暂不可读' : 'Network state unavailable';
  return <div className="decision-inherited-network">
    <div className="native-network-actions"><span className="field-help" role="status">{zh ? '使用 API 接入的网络设置' : 'Network settings from API connection'}{mode ? ` · ${mode}` : ''} · {detail}{dirty ? zh ? ' · 网络修改尚未完成' : ' · Network changes unfinished' : ''}</span><button className="btn btn-quiet" type="button" aria-expanded={open} aria-controls={id} onClick={() => { setOpened(true); setOpen(value => !value); if (!open) setEditorReveal(value => value + 1); }}>{open ? zh ? '收起网络设置' : 'Close network settings' : zh ? '网络设置' : 'Network settings'}</button></div>
    {opened && <div id={id} hidden={!open}><NativeSourceNetworkEditor key={sourceId} sourceId={sourceId} language={language} mutable={mutable} reveal={editorReveal} onDirty={networkChanged} onSaved={async () => { setRevision(value => value + 1); await onSaved?.(); }} /></div>}
  </div>;
}
