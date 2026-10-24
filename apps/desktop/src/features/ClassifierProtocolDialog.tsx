import { invoke, hostCapabilities } from '../host/index.ts';
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
      <div className="callout"><UiIcon name="info" /><span>{text('扩展直接选择任务类别与常规或主力组，可选评分仅用于观察。HiRoute 在选定组中选择模型并执行。', 'Choose the category and regular or primary group directly. Optional prior-stage scores are observation only. HiRoute selects and executes a model within that group.')}</span></div>
      <section>
        <div className="decision-methods" role="group" aria-label={text('协议示例', 'Protocol examples')}><button type="button" className="btn" aria-pressed={example === 'first'} onClick={() => { setExample('first'); setActionStatus('idle'); }}>{text('按难度分流 · 选组', 'Difficulty routing · group')}</button><button type="button" className="btn" aria-pressed={example === 'assessment'} onClick={() => { setExample('assessment'); setActionStatus('idle'); }}>{text('自定义分支 · 分类并评分', 'Custom branches · choice and assessment')}</button></div>
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
                  : actionStatus === 'saved' ? (hostCapabilities().host === 'console' ? text(`已开始下载：${CLASSIFIER_OPENAPI_FILENAME}`, `Download started: ${CLASSIFIER_OPENAPI_FILENAME}`) : text(`已保存：${CLASSIFIER_OPENAPI_FILENAME}`, `Saved: ${CLASSIFIER_OPENAPI_FILENAME}`))
                    : actionStatus === 'save_cancelled' ? text('已取消保存', 'Save cancelled')
                      : actionStatus === 'save_failed' ? text('保存失败，请重试。', 'Save failed. Try again.')
                        : ''
          }</span>
        </div>
      </section>
      <section>
        <h3>{text('响应示例', 'Response example')}</h3>
        <pre><code>{JSON.stringify(protocolExamples[example].response, null, 2)}</code></pre>
        <p>{example === 'first' ? text('首次请求的 preceding_stage 为 null。扩展直接返回 regular 或 primary。', 'The first request has preceding_stage: null. Return regular or primary directly.') : text('category 选择已发布分支，group 选择模型组。preceding_stage.from 是历史中的零基起始下标；评分只属于同类别的前一实际阶段。', 'category selects a published branch and group selects its model group. preceding_stage.from is a zero-based history index; assessment belongs only to the actual preceding stage in the same category.')}</p>
        <p>{text('category 必须来自已发布类别；双组范围返回 regular 或 primary。assessment 可选，其中 score 为 0–1，partial 必填且证据不完整时为 true。reason 可选。返回 HTTP 200 和纯 JSON，不附加未知字段。', 'category must match a published category; dual-group scopes return regular or primary. assessment is optional; within it, score is 0–1 and partial is required and true for incomplete evidence. reason is optional. Return HTTP 200 with strict JSON and no unknown fields.')}</p>
      </section>
    </div>
  </Dialog>;
}
