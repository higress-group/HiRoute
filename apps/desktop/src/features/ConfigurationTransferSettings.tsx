import { useRef, useState } from 'react';
import { invoke } from '../host/index.ts';
import { safeDiagnosticCode } from '../error-code';
import { parseConfigurationBundle, type ConfigurationBundle, type ConfigurationImportItem, type ConfigurationImportResult } from './configuration-transfer.ts';
import type { Language } from '../ui/preferences';

export function ConfigurationTransferSettings({ active, language }: { active: boolean; language: Language }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');
  const [result, setResult] = useState<ConfigurationImportResult | null>(null);
  const [importError, setImportError] = useState('');
  const fileInput = useRef<HTMLInputElement>(null);
  const disabled = busy || !active;

  async function exportBundle() {
    if (disabled) return;
    setBusy(true); setStatus(''); setError('');
    try {
      const bundle = await invoke<ConfigurationBundle>('configuration_export');
      const outcome = await invoke<'saved' | 'cancelled'>('save_configuration_bundle', { bundle });
      setStatus(outcome === 'cancelled' ? text('已取消保存。', 'Save cancelled.') : text('已导出配置。', 'Configuration exported.'));
    } catch (caught) {
      setError(safeDiagnosticCode(caught, 'CONFIGURATION_EXPORT_UNAVAILABLE'));
    } finally {
      setBusy(false);
    }
  }

  async function importBundle(file: File) {
    if (disabled) return;
    setBusy(true); setResult(null); setImportError(''); setError('');
    try {
      const parsed = parseConfigurationBundle(await file.text());
      if (!parsed) {
        setImportError('CONFIGURATION_BUNDLE_INVALID');
        return;
      }
      setResult(await invoke<ConfigurationImportResult>('configuration_import', { input: parsed }));
    } catch (caught) {
      setImportError(safeDiagnosticCode(caught, 'CONFIGURATION_IMPORT_UNAVAILABLE'));
    } finally {
      setBusy(false);
    }
  }

  const itemText = (item: ConfigurationImportItem) => item.state === 'applied'
    ? text('已应用', 'Applied')
    : item.state === 'unchanged'
      ? text('未变化', 'Unchanged')
      : text(`失败（${item.error_code ?? 'UNKNOWN'}）`, `Failed (${item.error_code ?? 'UNKNOWN'})`);

  return <section className="settings-group"><h2>{text('配置转移', 'Configuration transfer')}</h2><div className="settings-list">
    <div className="settings-row">
      <div className="settings-copy">
        <strong>{text('导出配置', 'Export configuration')}</strong>
        <span>{text('将并发上限、记录容量和模型订阅偏好保存为 JSON 文件，可导入到其他实例。', 'Save the concurrent limit, record capacity and model feed preferences as a JSON file for import on another instance.')}</span>
        <span>{text('不包含路由计划、决策服务、模型连接、网关监听和诊断等实例特定设置。', 'Routing plans, decision services, model connections, gateway listener and diagnostics are not included.')}</span>
      </div>
      <button className="btn" type="button" disabled={disabled} onClick={() => void exportBundle()}>{busy ? text('处理中…', 'Working…') : text('导出', 'Export')}</button>
    </div>
    {status && <div className="settings-inline-error" role="status">{status}</div>}
    {error && <div className="settings-inline-error" role="status">{text(`导出不可用（${error}）`, `Export unavailable (${error})`)}</div>}
    <div className="settings-row">
      <div className="settings-copy">
        <strong>{text('导入配置', 'Import configuration')}</strong>
        <span>{text('选择此前导出的 JSON 文件。相同项目保持不变，不同项目会应用，失败项目保留当前值。', 'Choose a previously exported JSON file. Matching items stay unchanged, differing items are applied, and failures keep the current value.')}</span>
      </div>
      <button className="btn" type="button" disabled={disabled} onClick={() => fileInput.current?.click()}>{text('导入', 'Import')}</button>
      <input ref={fileInput} type="file" accept="application/json,.json" hidden onChange={event => {
        const file = event.target.files?.[0];
        if (file) void importBundle(file);
        event.target.value = '';
      }} />
    </div>
    {importError === 'CONFIGURATION_BUNDLE_INVALID' && <div className="settings-inline-error" role="status">{text('文件不是有效的 HiRoute 配置包，未做任何更改。', 'The file is not a valid HiRoute configuration bundle; nothing was changed.')}</div>}
    {importError && importError !== 'CONFIGURATION_BUNDLE_INVALID' && <div className="settings-inline-error" role="status">{text(`导入不可用（${importError}）`, `Import unavailable (${importError})`)}</div>}
    {result && <div className="settings-row"><div className="settings-copy">
      <strong>{text('导入结果', 'Import result')}</strong>
      <span>{text('并发上限', 'Concurrent limit')}: {itemText(result.worker_settings)}</span>
      <span>{text('记录容量', 'Record capacity')}: {itemText(result.observation_capacity)}</span>
      <span>{text('模型订阅', 'Model feed')}: {itemText(result.model_feed)}</span>
    </div></div>}
  </div></section>;
}
