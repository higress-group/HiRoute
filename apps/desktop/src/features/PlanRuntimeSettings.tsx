import { useState } from 'react';
import type { Editor, Options, PlanEditorMemory } from '../plan-editor';
import { contextWindowError } from '../plan-context-window';
import { requestTimeoutError } from '../plan-request-timeout';
import { Disclosure, UiIcon } from '../ui';

type CapabilityIssue = Extract<NonNullable<Options['codex_capabilities']>, { state: 'unavailable' }>['issues'][number];

export function PlanRuntimeSettings({ limits, options, language, editingMemory, onChange }: {
  limits: Editor['limits'];
  options: Options | null;
  language: 'zh' | 'en';
  editingMemory: PlanEditorMemory;
  onChange(limits: Editor['limits']): void;
}) {
  const en = language === 'en';
  const text = (zh: string, eng: string) => en ? eng : zh;
  const [previewAgent, setPreviewAgent] = useState<'codex' | 'claude'>('codex');
  const bounds = options?.context_window;
  const custom = limits.context_window_tokens !== undefined;
  const windowError = contextWindowError(limits.context_window_tokens, bounds, language);
  const timeoutError = requestTimeoutError(limits.request_timeout_ms, limits.attempt_timeout_ms, language);
  const codex = options?.codex_capabilities;
  const claude = options?.claude_capabilities;
  const format = (tokens: number) => tokens.toLocaleString(en ? 'en-US' : 'zh-CN');
  const compact = (tokens: number) => tokens >= 1000 && tokens % 1000 === 0 ? `${tokens / 1000}K` : format(tokens);
  const bindings = (ids: string[]) => ids.map(id => options?.candidates.find(candidate => candidate.binding_id === id)?.display_name ?? id).join(text('、', ', '));
  function changeWindowMode(mode: string) {
    if (mode === 'auto') {
      editingMemory.customWindowTokens = limits.context_window_tokens;
      onChange({ ...limits, context_window_tokens: undefined });
    } else {
      onChange({ ...limits, context_window_tokens: editingMemory.customWindowTokens ?? bounds?.default_tokens ?? 272000 });
    }
  }
  function capabilityIssue(issue: CapabilityIssue) {
    const candidate = issue.binding_id ? bindings([issue.binding_id]) : text('当前路由', 'This route');
    const reason = issue.kind === 'responses_protocol' ? text('缺少 Codex Responses 协议', 'is missing the Codex Responses protocol')
      : issue.kind === 'request_capabilities' ? text('Codex Responses 请求路径的协议能力无法证明（不一定是模型元信息缺失）', 'cannot prove the required Codex Responses protocol semantics (not necessarily missing model metadata)')
      : issue.kind === 'instruction_roles' ? text('当前接口无法保留 Codex 常规请求中的独立 developer 指令或中途指令位置；若是 Messages 来源，请改用原生 Responses', 'cannot preserve Codex developer instructions or mid-conversation instruction positions; use native Responses for a Messages source')
      : issue.kind === 'context_input' ? text('缺少输入上下文上限', 'is missing its input context limit')
      : issue.kind === 'context_output' ? text('缺少输出上限', 'is missing its output limit')
      : issue.kind === 'context_total' ? text('缺少总上下文上限', 'is missing its total context limit')
      : issue.kind === 'reasoning_profile' ? text('缺少所选推理配置事实', 'is missing facts for the selected reasoning configuration')
      : issue.kind === 'context_window' ? text('无法得到有效上下文窗口', 'does not yield a valid context window')
      : issue.kind === 'plan_compilation' ? text('当前候选无法按发布规则编译', 'cannot be compiled under the publication rules')
      : text('编译后的路由事实无效', 'has invalid compiled routing facts');
    return en ? `${candidate} ${reason}.` : `${candidate}：${reason}。`;
  }

  return <section className="editor-section plan-runtime-settings">
    <div className="editor-section-heading"><h3>{text('上下文与等待时间', 'Context and wait time')}</h3></div>
    <div className="plan-runtime-grid">
      <div data-plan-context-window>
        <label className="field">
          <span className="field-label">{text('上下文窗口', 'Context window')}</span>
          <select className="select plan-context-mode" value={custom ? 'custom' : 'auto'} onChange={event => changeWindowMode(event.target.value)}>
            <option value="auto">{text('自动（默认）', 'Automatic (default)')}</option>
            <option value="custom">{text('自定义', 'Custom')}</option>
          </select>
        </label>
        {custom && <label className="field">
          <span className="field-label">{text('窗口大小（tokens）', 'Window size (tokens)')}</span>
          <input className="input plan-context-window-field" type="number" required min="1" step="1" max={bounds?.maximum_tokens}
            value={limits.context_window_tokens === 0 ? '' : limits.context_window_tokens}
            aria-invalid={Boolean(windowError)}
            onChange={event => onChange({ ...limits, context_window_tokens: event.target.value === '' ? 0 : Number(event.target.value) })} />
        </label>}
        <p className="field-help plan-current-window" data-plan-window-current>
          {bounds && !windowError
            ? text(`当前窗口 ${format(limits.context_window_tokens ?? bounds.default_tokens)} tokens`, `Current window ${format(limits.context_window_tokens ?? bounds.default_tokens)} tokens`)
            : windowError ? text('请修正自定义窗口。', 'Correct the custom window.')
              : text('完成模型组合配置后显示可用窗口。', 'Complete the model groups to see the available window.')}
        </p>
        {bounds && <p className="field-help">{text(`默认 ${format(bounds.default_tokens)} · 候选共同上界 ${format(bounds.maximum_tokens)} tokens`, `Default ${format(bounds.default_tokens)} · Shared candidate limit ${format(bounds.maximum_tokens)} tokens`)}</p>}
        {windowError && <p className="oc-inline-error" role="alert">{windowError}</p>}
      </div>
      <div data-plan-request-timeout>
        <label className="field">
          <span className="field-label">{text('单次请求最长等待（秒）', 'Maximum wait per request (seconds)')}</span>
          <input className="input plan-request-timeout-field" type="number" min={Math.ceil(limits.attempt_timeout_ms / 1000)} max={3600} step={1}
            value={limits.request_timeout_ms / 1000} aria-invalid={Boolean(timeoutError)}
            onChange={event => onChange({ ...limits, request_timeout_ms: event.target.value === '' ? 0 : Number(event.target.value) * 1000 })} />
        </label>
        <p className="field-help">{text('包含重试和流式输出；超过时限会中断回复。新计划默认 1 小时。', 'Includes retries and streaming. A response is interrupted at the limit. New plans default to 1 hour.')}</p>
        {timeoutError && <p className="oc-inline-error" role="alert">{timeoutError}</p>}
      </div>
    </div>
    <div className="plan-capability-preview">
      <label className="plan-preview-target"><span className="field-label">{text('目标 Agent 能力预览', 'Target Agent capability preview')}</span>
        <select className="select" value={previewAgent} onChange={event => setPreviewAgent(event.target.value as 'codex' | 'claude')}>
          <option value="codex">Codex</option><option value="claude">Claude Code</option>
        </select>
      </label>
      {previewAgent === 'codex' && codex && <div className="codex-capability-summary" data-codex-capability-state={codex.state}>
        {codex.state === 'available' ? <>
          <div className="plan-capability-value" data-codex-capability-summary>
            <strong>{text(`上下文 ${compact(codex.context_window)} · 输入 ${codex.input_modalities.includes('image') ? '文本/图片' : '文本'}`, `Context ${compact(codex.context_window)} · Input ${codex.input_modalities.includes('image') ? 'text/images' : 'text'}`)}</strong>
            <span>{text('推理强度由路由配置决定', 'Reasoning effort is determined by the route configuration')}</span>
          </div>
          {codex.limitations.map(limit => <div className="callout warn" role="status" data-codex-capability-limit={limit.kind} key={limit.kind}>
            <UiIcon name="warning" /><div>
              <strong>{limit.kind === 'context_window' ? text('候选收窄了共同上下文', 'A candidate narrows the shared context') : text('候选收窄了图片输入', 'A candidate narrows image input')}</strong>
              <p>{limit.kind === 'context_window'
                ? text(`${bindings(limit.binding_ids)} 将共同上下文限制为 ${compact(bounds?.maximum_tokens ?? codex.context_window)}。`, `${bindings(limit.binding_ids)} limits the shared context to ${compact(bounds?.maximum_tokens ?? codex.context_window)}.`)
                : text(`${bindings(limit.binding_ids)} 不支持完整的 Codex 图片输入；最终目录只声明文本。`, `${bindings(limit.binding_ids)} does not support complete Codex image input, so the final catalog declares text only.`)}</p>
            </div>
          </div>)}
        </> : <div className="callout bad" role="alert" data-codex-capability-unavailable><UiIcon name="warning" /><div>
          <strong>{text('无法生成可靠的 Codex 客户端能力', 'Reliable Codex client capabilities are unavailable')}</strong>
          {codex.issues.map((issue, index) => <p key={`${issue.kind}:${issue.binding_id ?? index}`}>{capabilityIssue(issue)}</p>)}
        </div></div>}
      </div>}
      {previewAgent === 'claude' && (claude?.state === 'available'
        ? <p className="plan-capability-value">{text(`自动压缩窗口最多 ${compact(claude.context_window)} tokens`, `Auto-compact window up to ${compact(claude.context_window)} tokens`)}</p>
        : <p className="field-help">{claude?.state === 'unavailable' && claude.reason === 'context_window_below_minimum'
          ? text('Claude Code 自动压缩窗口最低为 100K；请调整计划窗口或使用 Codex。', 'Claude Code requires an auto-compact window of at least 100K. Adjust the plan window or use Codex.')
          : text('当前候选或窗口尚不能用于 Claude Code；需要可用的 Messages 入口和有效窗口。', 'The current candidates or window are unavailable for Claude Code. A usable Messages ingress and valid window are required.')}</p>)}
      <Disclosure className="native-details plan-capability-details" label={text('能力依据与生效方式', 'Capability details and applying changes')} language={language}>
        <p className="field-help">{text('默认最多使用 272,000 tokens，受候选共同上界约束。较大的窗口可能增加上下文与缓存费用。', 'The default is at most 272,000 tokens, bounded by the shared candidate limit. Larger windows may increase context and cache costs.')}</p>
        <p className="field-help">{text('已接入的 Agent 需重新保存模型设置并重新启动，才能加载更新后的窗口；进行中的会话不会立即改变。', 'Save the Agent model settings again and restart the client to load the updated window. Running sessions do not change immediately.')}</p>
        {previewAgent === 'codex' ? <>
          <p className="field-help">{text('依据当前编辑内容及候选的 Codex Responses 入口能力计算。上游使用其他协议时仍可能通过 HiRoute 适配；发布和接入时会重新核对，预览不代表真实客户端验证。', 'Calculated from the current edit and candidate Codex Responses ingress capabilities. HiRoute may adapt another upstream protocol; publication and connection recheck compatibility, and this preview is not a live client verification.')}</p>
          {codex?.state === 'available' && codex.fixed_limits.includes('parallel_tool_calls_disabled') && <div className="callout" data-codex-fixed-limit="parallel_tool_calls_disabled"><UiIcon name="info" /><div>
            <strong>{text('HiRoute 当前固定使用串行工具调用', 'HiRoute currently uses serial tool calls')}</strong>
            <p>{text('这是当前实现限制，不由任何候选模型造成。', 'This is a fixed implementation limit, not a candidate-model limitation.')}</p>
          </div></div>}
        </> : <p className="field-help">{text('作用于整个 Claude Code 进程；接入多个计划时取最小窗口，原生模型可能更早压缩。接入时会核对窗口、配置冲突和请求能力。', 'Applies to the whole Claude Code process. Multiple plans share the smallest window; native models may compact earlier. Connection checks window constraints, configuration conflicts and request capabilities.')}</p>}
      </Disclosure>
    </div>
  </section>;
}
