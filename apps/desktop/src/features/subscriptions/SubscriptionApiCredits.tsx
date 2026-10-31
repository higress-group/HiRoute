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
      : (zh ? '符合条件的 Claude Max / Ultra / Team 可领取 API 赠金；Pro 不包含此权益。' : 'Eligible Claude Max / Ultra / Team plans can claim API credits. Pro does not include this benefit.')}
      {' '}<a href={guide} target="_blank" rel="noreferrer noopener" onClick={event => void open(event, guide)}>{zh ? '领取方式与适用范围' : 'How to claim and eligible uses'}</a>
    </p>
    <p className="field-help">{openai
      ? (zh ? '赠金不覆盖 Codex 的 API Key 用量，也不增加订阅额度。' : 'Credits exclude Codex usage through API keys and do not increase subscription limits.')
      : (zh ? '赠金不覆盖交互式 Claude Code，也不增加订阅额度。' : 'Credits exclude interactive Claude Code and do not increase subscription limits.')}</p>
    <Disclosure language={language} label={zh ? 'API 赠金如何使用？' : 'How do API credits work?'}>
      <p>{openai
        ? (zh ? '前往 ChatGPT 的账单设置（Business 使用工作空间账单），在 API 赠金中关联你拥有的 API 组织并确认领取；在这里填写该组织下的 API Key。' : 'Open ChatGPT billing settings (workspace billing for Business). Link an API organization you own in the API credits benefit and confirm the claim. Enter a key from that organization here.')
        : (zh ? '前往 Claude 的设置 → 账单（Team 使用组织设置 → 账单），在 API 赠金中关联 Claude Console 组织并确认领取；在这里填写该组织下的 API Key。' : 'Open Claude Settings → Billing (Organization settings → Billing for Team). Link a Claude Console organization in API credits and confirm the claim. Enter a key from that organization here.')}</p>
      <p>{openai
        ? (zh ? 'OpenAI 明确排除 Codex 的 API Key 用量。通过 HiRoute 中转也不能保证符合赠金条件。' : 'OpenAI excludes Codex usage through API keys. Routing through HiRoute does not establish credit eligibility.')
        : (zh ? '赠金覆盖符合条件的平台 API 等用途，不覆盖普通交互式 Claude Code 或 Claude 应用的额外用量。' : 'Credits cover eligible platform API uses, excluding ordinary interactive Claude Code and extra usage in Claude apps.')}</p>
      <p>{zh ? 'API 检查成功表示接入可用。赠金资格、余额、到期时间及实际抵扣以官方账单为准；HiRoute 不会自动领取或激活赠金。' : 'A successful API check verifies the connection. Eligibility, balance, expiry and deductions are determined by official billing. HiRoute does not claim or activate credits.'}</p>
      <a href={quickstart} target="_blank" rel="noreferrer noopener" onClick={event => void open(event, quickstart)}>{zh ? '查看官方 API Key 设置指引' : 'Official API key setup guide'}</a>
    </Disclosure>
    {failed && <p className="field-help" role="status">{zh ? '链接未能打开，可复制链接后在浏览器访问。' : 'Could not open the link. Copy it into your browser.'}</p>}
  </div>;
}
