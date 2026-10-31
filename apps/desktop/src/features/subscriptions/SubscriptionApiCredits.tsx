import { useState } from 'react';
import { Disclosure } from '../../ui';
import { invoke, isTauriHost } from '../../host/index.ts';
import type { LinkedSubscriptionProvider } from './api-sync';

export function SubscriptionApiCredits({ provider, language, part = 'details' }: { provider: LinkedSubscriptionProvider; language: 'zh' | 'en'; part?: 'intro' | 'details' }) {
  const zh = language === 'zh', openai = provider === 'codex';
  const [failed, setFailed] = useState(false);
  const guide = openai ? 'https://developers.openai.com/api/docs/guides/chatgpt-api-credits' : 'https://support.claude.com/en/articles/17154008-monthly-api-credits-for-max-ultra-and-team-plans';
  const quickstart = openai ? 'https://developers.openai.com/api/docs/quickstart' : 'https://platform.claude.com/docs/en/get-started';
  async function open(event: React.MouseEvent<HTMLAnchorElement>, url: string) {
    if (!isTauriHost()) return;
    event.preventDefault(); setFailed(false);
    try { await invoke('open_external_url', { url }); } catch { setFailed(true); }
  }
  return <div className={`subscription-api-credits subscription-api-credits-${part}`}>
    {part === 'intro' ? <p className="field-help">{openai
      ? (zh ? '符合条件的 ChatGPT Pro / Business 可领取 API 赠金。' : 'Eligible ChatGPT Pro / Business plans can claim API credits.')
      : (zh ? '符合条件的 Claude Max / Ultra / Team 可领取 API 赠金。' : 'Eligible Claude Max / Ultra / Team plans can claim API credits.')}
      {' '}<a href={guide} target="_blank" rel="noreferrer noopener" onClick={event => void open(event, guide)}>{zh ? '领取与说明' : 'Claim and details'}</a>
    </p> : <Disclosure language={language} label={zh ? '赠金使用说明' : 'Using API credits'}>
      <p>{openai
        ? (zh ? '在 ChatGPT 账单中关联 API 组织领取，再填写该组织的 Key（Business 使用工作空间账单）。' : 'Claim credits by linking your API organization in ChatGPT billing (workspace billing for Business), then use its key.')
        : (zh ? '在 Claude 账单中关联 Console 组织领取，再填写该组织的 Key（Team 使用组织账单）。' : 'Claim credits by linking your Console organization in Claude billing (organization billing for Team), then use its key.')}</p>
      <p>{zh ? '资格、范围与余额见官方说明及账单。' : 'See the official guide and billing page for eligibility, uses and balance.'}</p>
      <a href={quickstart} target="_blank" rel="noreferrer noopener" onClick={event => void open(event, quickstart)}>{zh ? 'API Key 获取指引' : 'Get an API key'}</a>
    </Disclosure>}
    {failed && <p className="field-help" role="status">{zh ? '链接未能打开，可复制链接后在浏览器访问。' : 'Could not open the link. Copy it into your browser.'}</p>}
  </div>;
}
