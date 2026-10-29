import { useId } from 'react';
import { UiIcon } from '../../ui';

export type ConnectedTrialModel = {
  bindingId: string; sourceId?: string; name: string; source: string;
  available: boolean; reason?: string;
};

/** A model combobox opens the saved-model dialog; catalog cards grant no binding. */
export function TrialConnectedModelSelect({ choices, selected, language, busy, expanded, onOpen }: {
  choices: ConnectedTrialModel[]; selected?: string; language: 'zh' | 'en'; busy: boolean;
  expanded: boolean; onOpen(): void;
}) {
  const id = useId(), zh = language === 'zh';
  const value = choices.find(choice => choice.bindingId === selected);
  return <div className="trial-connected-model"><span id={`${id}-label`}>{zh ? '选择已接入模型' : 'Choose a connected model'}</span>
    <button type="button" className="input trial-connected-trigger" data-trial-connected-model data-trial-binding={selected ?? ''} data-trial-focus role="combobox" aria-haspopup="dialog" aria-expanded={expanded} aria-labelledby={`${id}-label ${id}-value`} disabled={busy} onClick={onOpen}>
      <span id={`${id}-value`}>{value ? `${value.name} · ${value.source}` : zh ? choices.length ? '请选择已接入模型' : '还没有已接入模型' : choices.length ? 'Select a connected model' : 'No connected models yet'}</span><UiIcon name="chevron" />
    </button>
  </div>;
}
