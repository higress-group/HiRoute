import { Disclosure, UiIcon } from '../../ui';
import type { Classifier } from '../decision-services/types';
import { protectionExplanation } from '../trust/protection-copy';
import './trial-protection-explanation.css';

export function TrialProtectionExplanation({ classifier, language }: { classifier: Classifier; language: 'zh' | 'en' }) {
  const zh = language === 'zh';
  const custom = classifier.kind === 'decision_service' && classifier.service.connection.kind === 'custom';
  const summary = custom
    ? (zh ? '由自定义扩展选择新模型或主力；HiRoute 不额外应用信任阈值或低分保护。' : 'The custom extension chooses the trial model or Primary. HiRoute adds no trust threshold or low-score protection.')
    : classifier.kind === 'local_rules'
      ? (zh ? '按当前任务信号在新模型和主力之间选择；此模式不启用低分保护。' : 'Current task signals choose between the trial model and Primary. This mode does not apply low-score protection.')
      : (zh ? '新模型先处理适合它的任务，主力承担复杂任务。需要低分保护时，本轮使用主力。' : 'The trial model handles suitable tasks; Primary handles complex tasks. When low-score protection applies, this turn uses Primary.');
  return <div className="callout trial-rule"><UiIcon name="info" /><div className="trial-rule-content">
    <p>{summary}</p>
    <Disclosure language={language} label={zh ? '如何分配任务与保护' : 'How task routing and protection work'}>
      <p>{zh ? '新模型先放进常规组。' : 'The new model starts in Regular. '}{protectionExplanation(classifier, language)}{classifier.kind === 'decision_service' && !custom && (zh ? '主力拿到低分不会再回退。熟悉之后，可以用分工把难的任务交给新模型。' : ' Primary does not fall back after a low score. Once you know the new model, divide work to give it harder tasks.')}</p>
    </Disclosure>
  </div></div>;
}
