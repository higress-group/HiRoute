import { useState } from 'react';
import { invoke } from '../../host/index.ts';
import { Disclosure } from '../../ui';
import type { AgentModelSelection } from './types';

type PreviousPreview = { status: 'available'; spec: { context_id: string; model: { intent: 'configure'; settings: AgentModelSelection } }; restored_from: Record<string, string> }
  | { status: 'unavailable'; reason: string; restore_is_separate: boolean };

export function PreviousRouting({ agent, language, disabled, onDraft, onRestore }: {
  agent: string; language: 'zh' | 'en'; disabled: boolean;
  onDraft(selection: AgentModelSelection, context: string, source: HTMLElement): void;
  onRestore?(source: HTMLElement): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [presets, setPresets] = useState(['opus']);
  const [result, setResult] = useState<PreviousPreview>();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState(false);
  const claude = agent === 'agent_claude_default';
  async function read() {
    setPending(true); setError(false); setResult(undefined);
    try { setResult(await invoke<PreviousPreview>('preview_agent_previous_routing', { input: { agent, presets: claude ? presets : [] } })); }
    catch { setError(true); }
    finally { setPending(false); }
  }
  return <Disclosure label={t('回到之前的路由', 'Return to previous routing')} language={language}>
    <p className="field-help">{t('查找上一次映射变更前的配置。只生成可审阅的修改，其他预设和已有证据保留。', 'Find the mapping before its most recent change. Review the draft before saving; other presets and recorded evidence remain available.')}</p>
    {claude && <fieldset className="actions" disabled={pending || disabled}><legend>{t('选择要恢复的预设', 'Choose presets')}</legend>{['opus', 'sonnet', 'haiku'].map(preset => <label key={preset}><input type="checkbox" checked={presets.includes(preset)} onChange={event => { setResult(undefined); setPresets(value => event.target.checked ? [...value, preset] : value.filter(p => p !== preset)); }} /> {preset[0].toUpperCase() + preset.slice(1)}</label>)}</fieldset>}
    <button type="button" className="btn" disabled={disabled || pending || (claude && !presets.length)} onClick={() => void read()}>{pending ? t('正在读取…', 'Reading…') : t('查看之前的配置', 'Find previous mappings')}</button>
    {error && <p role="alert">{t('读取失败，请重试。', 'Could not read history. Please retry.')}</p>}
    {result?.status === 'unavailable' && <div role="status"><p>{result.reason === 'no_previous_configuration'
      ? t('没有可恢复的上一次 HiRoute 映射。', 'There is no earlier HiRoute mapping to recover.')
      : t('配置历史缺失、不完整或无法确认，无法安全生成回退草稿。', 'The configuration history is missing, incomplete or cannot be verified, so a recovery draft is unavailable.')}</p><p className="field-help">{t('恢复原生配置是独立操作，不代表历史中的上一次路由。', 'Restore native configuration is a separate action, not a recovered historical mapping.')}</p>{onRestore && <button type="button" className="btn" disabled={disabled} onClick={event => onRestore(event.currentTarget)}>{t('审阅恢复原生配置', 'Review restore to native')}</button>}</div>}
    {result?.status === 'available' && <div><p>{t('已找到之前的映射。接下来可查看和修改草稿，再确认保存。', 'Earlier mappings are available. Review and edit the draft before confirming the save.')}</p><button type="button" className="btn" disabled={disabled} onClick={event => onDraft(result.spec.model.settings, result.spec.context_id, event.currentTarget)}>{t('审阅路由修改', 'Review routing changes')}</button></div>}
  </Disclosure>;
}
