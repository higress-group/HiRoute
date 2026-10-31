import { useState } from 'react';
import { Disclosure } from '../../ui';
import { invoke, isTauriHost } from '../../host/index.ts';
import type { LinkedSubscriptionProvider } from './api-sync';

export function SubscriptionApiCredits({ provider, language }: { provider: LinkedSubscriptionProvider; language: 'zh' | 'en' }) {
  const zh = language === 'zh', openai = provider === 'codex';
  const [failed, setFailed] = useState(false);
  const guide = openai ? 'https://developers.openai.com/api/docs/guides/chatgpt-api-credits' : 'https://support.claude.com/en/articles/17154008-monthly-api-credits-for-max-ultra-and-team-plans';
  const quickstart = openai ? 'https://developers.openai.com/api/docs/quickstart' : 'https://platform.claude.com/docs/en/get-started';
  async function open(event: React.MouseEvent<HTMLAnchorElement>, url: string) {
    if (!isTauriHost()) return;
    event.preventDefault(); setFailed(false);
    try { await invoke('open_external_url', { url }); } catch { setFailed(true); }
  }
  return <div className="subscription-api-credits">
    <p className="field-help">{openai
      ? (zh ? '符合条件的 ChatGPT Pro / Business 可领取 API 赠金。' : 'Eligible ChatGPT Pro / Business plans can claim API credits.')
      : (zh ? '符合条件的 Claude Max / Ultra / Team 可领取 API 赠金。' : 'Eligible Claude Max / Ultra / Team plans can claim API credits.')}
      {' '}<a href={guide} target="_blank" rel="noreferrer noopener" onClick={event => void open(event, guide)}>{zh ? '领取方式与适用范围' : 'How to claim and eligible uses'}</a>
    </p>
    <Disclosure language={language} label={zh ? 'API 赠金如何使用？' : 'How do API credits work?'}>
      <p>{openai
        ? (zh ? '前往 ChatGPT 的账单设置（Business 使用工作空间账单），在 API 赠金中关联你拥有的 API 组织并确认领取；在这里填写该组织下的 API Key。' : 'Open ChatGPT billing settings (workspace billing for Business). Link an API organization you own in the API credits benefit and confirm the claim. Enter a key from that organization here.')
        : (zh ? '前往 Claude 的设置 → 账单（Team 使用组织设置 → 账单），在 API 赠金中关联 Claude Console 组织并确认领取；在这里填写该组织下的 API Key。' : 'Open Claude Settings → Billing (Organization settings → Billing for Team). Link a Claude Console organization in API credits and confirm the claim. Enter a key from that organization here.')}</p>
      <p>{zh ? '赠金的领取资格、适用范围、余额和有效期可在官方说明及账单中查看。' : 'Check the official guide and billing page for credit eligibility, eligible uses, balance and expiry.'}</p>
      <a href={quickstart} target="_blank" rel="noreferrer noopener" onClick={event => void open(event, quickstart)}>{zh ? '查看官方 API Key 设置指引' : 'Official API key setup guide'}</a>
    </Disclosure>
    {failed && <p className="field-help" role="status">{zh ? '链接未能打开，可复制链接后在浏览器访问。' : 'Could not open the link. Copy it into your browser.'}</p>}
  </div>;
}
