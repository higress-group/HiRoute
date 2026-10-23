import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from 'react';
import { invoke, isTauriHost } from '../../host/index.ts';
import { safeDiagnosticCode } from '../../error-code.ts';
import { Disclosure } from '../../ui';
import { inferenceRule, metadataReferences, type MetadataReference } from './metadata-facts.ts';

export type MetadataSnapshot = {generation:number;data_revision:number;data_digest:string};
type Evidence = MetadataReference & {value:Record<string,unknown>};
type Detail = {record:Evidence};
function text(value:unknown): string {return typeof value === 'string' ? value : '';}
function externalUrl(value:unknown): string | null {
  try {
    const url = new URL(text(value));
    return ['https:','http:'].includes(url.protocol) && !url.username && !url.password ? url.href : null;
  } catch {return null;}
}

export function MetadataFact({value,field,zh,children}:{value:Record<string,unknown>;field:string;zh:boolean;children:ReactNode}) {
  return <span className="model-metadata-fact" data-field={field}>{children}{inferenceRule(value,field) && <span className="model-metadata-inferred">{zh?'推断':'Inferred'}</span>}</span>;
}

export function ModelMetadataEvidence({value,snapshot,zh,onReload}:{value:Record<string,unknown>;snapshot:MetadataSnapshot;zh:boolean;onReload():void}) {
  const [records,setRecords] = useState<Evidence[]|null>(null);
  const [busy,setBusy] = useState(false);
  const [error,setError] = useState<'stale'|'unavailable'|null>(null);
  const [linkError,setLinkError] = useState(false);
  const epoch = useRef(0);
  const references = metadataReferences(value);
  useEffect(() => () => {++epoch.current;},[]);
  async function openSource(event:MouseEvent<HTMLAnchorElement>,url:string) {
    if (!isTauriHost()) return;
    event.preventDefault();setLinkError(false);
    try {await invoke('open_external_url',{url});}
    catch {setLinkError(true);}
  }
  async function load() {
    const serial = ++epoch.current;
    setBusy(true);setError(null);
    const read = async (ref:MetadataReference) => (await invoke<Detail>('get_model_metadata',{input:{...ref,snapshot}})).record;
    try {
      const result = await Promise.all(references.map(read));
      // Rule evidence explains a conservative inference; the source of the model
      // record alone need not contain the facts used by that inference.
      const linked = new Set(result.flatMap(record => record.kind === 'inference_rule' && Array.isArray(record.value.evidence_refs) ? record.value.evidence_refs.filter((key):key is string => typeof key === 'string') : []));
      const extra = await Promise.all([...linked].filter(key => !result.some(record => record.kind === 'evidence_source' && record.key === key)).map(key => read({kind:'evidence_source',key})));
      if (serial === epoch.current) setRecords([...result,...extra]);
    } catch (failure) {
      if (serial === epoch.current) setError(safeDiagnosticCode(failure,'').toLowerCase().includes('stale') ? 'stale' : 'unavailable');
    } finally {if (serial === epoch.current) setBusy(false);}
  }
  if (!references.length) return null;
  return <Disclosure className="model-metadata-evidence" label={zh?'来源与推断依据':'Sources and inference'} language={zh?'zh':'en'} onOpenChange={open => {if (open && records === null && !busy && !error) void load();}}>
    {busy && <p role="status">{zh?'正在读取来源…':'Loading sources…'}</p>}
    {error && <p role="alert">{error === 'stale' ? (zh?'模型资料已更新，请重新读取目录。':'Model information changed. Reload the directory.') : (zh?'暂时无法读取来源。':'Sources are temporarily unavailable.')} <button className="btn btn-quiet" disabled={busy} onClick={() => error === 'stale' ? onReload() : void load()}>{error === 'stale' ? (zh?'重新读取目录':'Reload directory') : (zh?'重试':'Retry')}</button></p>}
    {records?.map(record => {
      const rule = record.kind === 'inference_rule';
      const repository = record.kind === 'client_run';
      const source = externalUrl(repository ? record.value.repository_url : record.value.locator);
      return <section className={rule?'model-metadata-rule':'model-metadata-source'} key={`${record.kind}/${record.key}`}>
        <strong>{rule ? (zh?'推断依据':'Inference basis') : repository ? (zh?'客户端资料':'Client source') : (zh?'资料来源':'Source')}</strong>
        {rule && <p>{text(record.value.reason)}</p>}
        {source && <p><a href={source} target="_blank" rel="noreferrer" onClick={event => void openSource(event,source)}>{source}</a></p>}
        {repository && <p className="field-help">{zh?'来源版本':'Source revision'}: {text(record.value.commit)}</p>}
        <p className="field-help">{record.key}{record.value.collected_on ? ` · ${zh?'资料日期':'Collected'}: ${text(record.value.collected_on)}` : ''}</p>
      </section>;
    })}
    {linkError && <p className="model-metadata-link-error" role="alert">{zh?'无法打开浏览器。来源资料仍可查看，请重试链接。':'Could not open the browser. Sources are still available; try the link again.'}</p>}
  </Disclosure>;
}
