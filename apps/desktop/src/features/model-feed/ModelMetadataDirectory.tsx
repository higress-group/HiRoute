import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { Dialog } from '../../ui';
import { safeDiagnosticCode } from '../../error-code';
import { readCurrentCatalog } from './catalog-read';

type Entry = {kind:string;key:string;origin:string;qualification:string;value:Record<string,unknown>};
type Page = {records:Entry[];total:number;next_cursor:string|null;complete:boolean};
function text(value:unknown):string {return typeof value === 'string' ? value : '';}
function limit(value:unknown):string {if (typeof value === 'number') return value.toLocaleString(); if (value && typeof value === 'object' && 'value' in value) return limit(value.value); return '—';}
export function ModelMetadataDirectory({ language }: {language:'zh'|'en'}) {
  const zh = language === 'zh';
  const [open,setOpen] = useState(false); const [kind,setKind] = useState('model_metadata'); const [search,setSearch] = useState('');
  const [page,setPage] = useState<Page|null>(null); const [rows,setRows] = useState<Entry[]>([]);
  const [busy,setBusy] = useState(false); const [error,setError] = useState(false); const [retry,setRetry] = useState(0);
  const epoch = useRef(0);
  async function load(cursor?:string) {
    const serial = ++epoch.current; setBusy(true);setError(false);let restarted = false;
    try {
      const result = await readCurrentCatalog(async () => {
        const input = {kind,search:search.trim() || undefined,page_size:50,cursor};
        try {return await invoke<Page>('list_model_metadata',{input});}
        catch (failure) {
          if (!cursor || !safeDiagnosticCode(failure,'').toLowerCase().includes('stale')) throw failure;
          restarted = true; return invoke<Page>('list_model_metadata',{input:{...input,cursor:undefined}});
        }
      });
      if (serial === epoch.current) {setPage(result);setRows(old => cursor && !restarted ? [...old,...result.records] : result.records);}
    } catch {if (serial === epoch.current) setError(true);} finally {if (serial === epoch.current) setBusy(false);}
  }
  useEffect(() => {
    if (!open) {++epoch.current;return;}
    const timer = window.setTimeout(() => {void load();},150);
    return () => {window.clearTimeout(timer);++epoch.current;};
  },[open,kind,search,retry]);
  return <><button className="btn btn-quiet" onClick={() => setOpen(true)}>{zh ? '浏览全部模型资料' : 'Browse all model information'}</button>
    <Dialog open={open} title={zh ? '官方模型资料' : 'Official model information'} closeLabel={zh ? '关闭' : 'Close'} onClose={() => setOpen(false)} footer={<button className="btn" onClick={() => setOpen(false)}>{zh ? '关闭' : 'Close'}</button>}>
      <p>{zh ? '资料覆盖能力、接入产品、价格与证据来源。资料存在不代表当前客户端或你的账户已能调用。' : 'Information includes capabilities, products, prices and sources. A documented model is not proof of support by this client or access by your account.'}</p>
      <div className="model-metadata-filters"><label>{zh ? '类别' : 'Category'}<select className="input" value={kind} onChange={e => setKind(e.target.value)}>{[['model_metadata',zh?'模型':'Models'],['provider',zh?'供应商':'Providers'],['access_product',zh?'接入产品':'Products'],['price',zh?'价格':'Prices'],['evidence_source',zh?'证据来源':'Sources']].map(([value,label]) => <option key={value} value={value}>{label}</option>)}</select></label><label>{zh?'搜索':'Search'}<input className="input" value={search} maxLength={256} onChange={e => setSearch(e.target.value)}/></label></div>
      <p role="status">{page ? `${rows.length} / ${page.total}` : busy ? (zh?'正在读取…':'Loading…') : ''}</p>
      <ul className="model-metadata-records">{rows.map(row => {
        const v = row.value; const capabilities = (v.capability_hints ?? v.capabilities ?? {}) as Record<string,unknown>;
        const schedules = Array.isArray(v.schedules) ? v.schedules as Record<string,unknown>[] : [];
        return <li key={`${row.kind}/${row.key}`}><strong>{text(v.display_name) || text(v.product) || text(v.provider) || text(v.title) || text(v.upstream_model_id) || row.key}</strong><span className="field-help">{text(v.provider_id) || text(v.provider)} · {row.origin === 'website_cache' ? (zh?'官网 / 本机缓存':'Website / local cache') : (zh?'内置资料':'Embedded information')}</span>
          {kind === 'model_metadata' && <dl><dt>{zh?'上下文 / 输出上限':'Context / output limit'}</dt><dd>{limit(v.context_tokens)} / {limit(v.max_output_tokens)}</dd><dt>{zh?'工具 / 图片输入':'Tools / vision'}</dt><dd>{JSON.stringify(capabilities.tool ?? capabilities.tool_call ?? '—')} / {JSON.stringify(capabilities.vision ?? '—')}</dd></dl>}
          {kind === 'access_product' && <p>{text(v.region_scope)} · {text(v.billing)} · {text(v.restriction)}</p>}
          {kind === 'price' && <><p>{text(v.product_key)} · {text(v.currency)} / {zh?'百万 Token':'million tokens'}</p>{schedules.map(s => <div key={text(s.schedule_key)}><strong>{text(s.schedule_key)}</strong><p>{Object.entries((s.rates ?? {}) as Record<string,string>).map(([name,value]) => `${name}: ${value}`).join(' · ')}</p><p className="field-help">{JSON.stringify(s.condition)} · {text(s.application)}</p></div>)}</>}
          {kind === 'evidence_source' && <p>{text(v.url)} · {text(v.collected_on)}</p>}
        </li>;
      })}</ul>
      {error && <p role="alert">{zh?'暂时无法读取资料。':'Information is temporarily unavailable.'}<button className="btn btn-quiet" onClick={() => setRetry(n => n+1)} disabled={busy}>{zh?'重试':'Retry'}</button></p>}
      {page?.next_cursor && <button className="btn" disabled={busy} onClick={() => void load(page.next_cursor ?? undefined)}>{zh?'加载更多':'Load more'}</button>}
    </Dialog></>;
}
