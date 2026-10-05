import React from 'react';
import './agent-protocol-choice.css';
export type AgentProtocol = 'responses' | 'messages';
export type ProtocolAdvice = { supported: AgentProtocol[]; native: AgentProtocol[] };
const label = (protocol: AgentProtocol) => protocol === 'responses' ? 'Responses' : 'Messages';

/** Advice never mutates the saved choice or issues a model request. */
export function AgentProtocolChoice({ value, advice, language, name, onChange }: {
  value: AgentProtocol; advice?: ProtocolAdvice; language: 'zh' | 'en'; name: string;
  onChange: (protocol: AgentProtocol) => void;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const recommended = advice?.native[0];
  return <div className="agent-protocol-choice" data-plan-protocol={name}>
    <label className="agent-protocol-choice__field"><span>{text('接入协议', 'Connection protocol')}</span>
      <select className="select" aria-label={`${name} · ${text('接入协议', 'Connection protocol')}`} value={value} onChange={event => onChange(event.target.value as AgentProtocol)}>
        {(['responses', 'messages'] as const).map(protocol => <option key={protocol} value={protocol} disabled={advice !== undefined && !advice.supported.includes(protocol)}>{label(protocol)}</option>)}
      </select>
    </label>
    <p className="field-help">{recommended
      ? text(`推荐 ${label(recommended)}：全部候选及回退模型均原生支持，可避免协议转换的额外开销与兼容风险。`, `Recommended: ${label(recommended)}. All candidates, including fallbacks, support it natively, avoiding protocol conversion overhead and compatibility risks.`)
      : advice ? text('当前模型组合没有共同的原生协议，可能需要协议转换。', 'The current models share no native protocol; conversion may be required.')
      : text('协议推荐需要候选模型的能力信息。', 'Protocol recommendations require candidate capability information.')}</p>
    {advice && !advice.native.includes(value) && advice.supported.includes(value) && <p className="field-help">{text('所选协议可用，但部分候选模型需要转换。', 'The selected protocol is supported, but some candidates require conversion.')}</p>}
  </div>;
}
