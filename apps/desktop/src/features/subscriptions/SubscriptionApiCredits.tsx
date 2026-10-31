import { useState } from 'react';
import { invoke, isTauriHost } from '../../host/index.ts';
import type { LinkedSubscriptionProvider } from './api-sync';

export function SubscriptionApiCredits({ provider, language }: { provider: LinkedSubscriptionProvider; language: 'zh' | 'en' }) {
  const zh = language === 'zh', openai = provider === 'codex';
  const [failed, setFailed] = useState(false);
  const guide = openai ? 'https://developers.openai.com/api/docs/guides/chatgpt-api-credits' : 'https://support.claude.com/en/articles/17154008-monthly-api-credits-for-max-ultra-and-team-plans';
  async function open(event: React.MouseEvent<HTMLAnchorElement>, url: string) {
    if (!isTauriHost()) return;
    event.preventDefault(); setFailed(false);
    try { await invoke('open_external_url', { url }); } catch { setFailed(true); }
  }
  return <div className="subscription-api-credits">
    <p className="field-help">{openai
      ? (zh ? '符合条件的 ChatGPT Pro / Business 可领取 API 赠金。' : 'Eligible ChatGPT Pro / Business plans can claim API credits.')
      : (zh ? '符合条件的 Claude Max / Ultra / Team 可领取 API 赠金。' : 'Eligible Claude Max / Ultra / Team plans can claim API credits.')}
      {' '}<a href={guide} target="_blank" rel="noreferrer noopener" onClick={event => void open(event, guide)}>{zh ? '官方领取说明' : 'Official claim guide'}</a>
    </p>
    {failed && <p className="field-help" role="status">{zh ? '链接未能打开，可复制链接后在浏览器访问。' : 'Could not open the link. Copy it into your browser.'}</p>}
  </div>;
}
