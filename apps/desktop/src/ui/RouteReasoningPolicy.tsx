import { useId } from 'react';

/** A route-wide policy; candidate controls edit its fallback or fixed settings. */
export function RouteReasoningPolicy({ followRequest, difficultyRouting, language, onChange }: {
  followRequest: boolean;
  difficultyRouting: boolean;
  language: 'zh' | 'en';
  onChange(followRequest: boolean): void;
}) {
  const id = useId();
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  return <div className="route-reasoning-policy">
    <div className="route-reasoning-policy-row">
      <span className="field-label" id={`${id}-label`}>{text('思考档位', 'Reasoning levels')}</span>
      <div className="segmented route-reasoning-modes" role="radiogroup" aria-labelledby={`${id}-label`} aria-describedby={`${id}-help`}>
        {[true, false].map(follow => <label key={String(follow)} className={`segment${followRequest === follow ? ' active' : ''}`}>
          <input className="sr-only" type="radio" name={id} value={follow ? 'follow' : 'fixed'} checked={followRequest === follow} onChange={() => onChange(follow)} />
          {follow ? text('跟随请求', 'Follow request') : text('固定档位', 'Fixed levels')}
        </label>)}
      </div>
    </div>
    <p className="field-help" id={`${id}-help`}>
      {followRequest ? text('未指定或不支持时，使用下方各模型的档位。', 'Use the levels below when a request omits one or it is unsupported.')
        : text('始终使用下方各模型的档位。', 'Always use the levels set below.')}
      {difficultyRouting && text(' 跟随请求可能改变各组的能力与成本。', ' Following requests may change each group’s capability and cost.')}
    </p>
  </div>;
}
