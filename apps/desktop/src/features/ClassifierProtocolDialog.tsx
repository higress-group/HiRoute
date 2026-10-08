import { invoke } from '@tauri-apps/api/core';
import { copyText, Dialog, UiIcon } from '../ui';
import { useState } from 'react';
import { protocolExamples, protocolCurl } from './decision-services/protocol-examples';

export const CLASSIFIER_OPENAPI_FILENAME = 'hiroute-decision.openapi.json';

export const classifierCurlExample = protocolCurl('first');

export async function saveClassifierOpenApi(): Promise<'saved' | 'cancelled'> {
  return invoke<'saved' | 'cancelled'>('save_classifier_openapi');
}

export function ClassifierProtocolDialog({
  open,
  language,
  onClose,
}: {
  open: boolean;
  language: 'zh' | 'en';
  onClose: () => void;
}) {
  const en = language === 'en';
  const text = (zh: string, english: string) => en ? english : zh;
  const [actionStatus, setActionStatus] = useState<'idle' | 'copied' | 'copy_failed' | 'saving' | 'saved' | 'save_cancelled' | 'save_failed'>('idle');
  const [example, setExample] = useState<keyof typeof protocolExamples>('first');
  const curl = protocolCurl(example);
  const close = () => {
    setActionStatus('idle');
    onClose();
  };

  return <Dialog
    open={open}
    title={text('自定义扩展接入协议', 'Custom extension protocol')}
    description={text('HiRoute 向你配置的完整地址发送一次 HTTP POST JSON 请求。', 'HiRoute sends one HTTP POST JSON request to the complete endpoint you configure.')}
    closeLabel={text('关闭接入协议', 'Close protocol')}
    onClose={close}
    footer={<button className="btn btn-primary" type="button" onClick={close}>{text('完成', 'Done')}</button>}
  >
    <div className="classifier-protocol">
      <div className="callout"><UiIcon name="info" /><span>{text('扩展按传入的定义判断任务类别或程度，并按指定标准评价上一阶段。阈值比较和模型选择由 HiRoute 处理。', 'Follow the supplied category or degree definition and assess the prior stage using its supplied standard. HiRoute handles thresholds and model selection.')}</span></div>
      <section>
        <div className="decision-methods" role="group" aria-label={text('协议示例', 'Protocol examples')}><button type="button" className="btn" aria-pressed={example === 'first'} onClick={() => { setExample('first'); setActionStatus('idle'); }}>{text('智能省钱 · 程度判断', 'Smart saving · degree')}</button><button type="button" className="btn" aria-pressed={example === 'assessment'} onClick={() => { setExample('assessment'); setActionStatus('idle'); }}>{text('自定义分支 · 分类并评分', 'Custom branches · choice and assessment')}</button></div>
        <h3>{text('请求示例', 'Request example')}</h3>
        <p>{text('将示例地址替换为你的服务地址；如已配置认证，HiRoute 会额外发送对应请求头。', 'Replace the example URL with your service endpoint. When authentication is configured, HiRoute also sends that header.')}</p>
        <pre><code>{curl}</code></pre>
        <div className="classifier-protocol-actions">
          <button className="btn" type="button" onClick={() => {
            void copyText(curl)
              .then(() => setActionStatus('copied'))
              .catch(() => setActionStatus('copy_failed'));
          }}><UiIcon name="copy" />{text('复制 curl', 'Copy curl')}</button>
          <button className="btn" type="button" disabled={actionStatus === 'saving'} onClick={() => {
            setActionStatus('saving');
            void saveClassifierOpenApi()
              .then(outcome => setActionStatus(outcome === 'saved' ? 'saved' : 'save_cancelled'))
              .catch(() => setActionStatus('save_failed'));
          }}><UiIcon name="download" />{text('保存 OpenAPI', 'Save OpenAPI')}</button>
          <span className="field-help" role="status" aria-live="polite">{
            actionStatus === 'copied' ? text('已复制', 'Copied')
              : actionStatus === 'copy_failed' ? text('复制失败', 'Copy failed')
                : actionStatus === 'saving' ? text('请选择保存位置', 'Choose where to save the file')
                  : actionStatus === 'saved' ? text(`已保存：${CLASSIFIER_OPENAPI_FILENAME}`, `Saved: ${CLASSIFIER_OPENAPI_FILENAME}`)
                    : actionStatus === 'save_cancelled' ? text('已取消保存', 'Save cancelled')
                      : actionStatus === 'save_failed' ? text('保存失败，请重试。', 'Save failed. Try again.')
                        : ''
          }</span>
        </div>
      </section>
      <section>
        <h3>{text('响应示例', 'Response example')}</h3>
        <pre><code>{JSON.stringify(protocolExamples[example].response, null, 2)}</code></pre>
        <p>{example === 'first' ? text('首次请求的 assessment_target 为 null。ordinal 返回各程度的概率，合计为 1。', 'The first request has assessment_target: null. ordinal returns level probabilities summing to 1.') : text('categorical 选择本轮分支，refinement 返回该分支的程度概率。assessment_target.from 是历史中的零基起始下标；这里评分属于上一写稿阶段。', 'categorical chooses this turn’s branch; refinement gives its degree probabilities. assessment_target.from is a zero-based history index. Assessment belongs to the previous writing stage.')}</p>
        <p>{text('choice 和概率的 ID 必须来自本次定义。score 为 0–1；partial 必填，目标证据不完整时为 true；reason 可选。返回 HTTP 200 和纯 JSON，不附加未知字段。', 'choice and probability IDs must match the definition. score is 0–1; partial is required and true for incomplete target evidence; reason is optional. Return HTTP 200 with strict JSON and no unknown fields.')}</p>
      </section>
    </div>
  </Dialog>;
}
