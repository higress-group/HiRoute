import { readCurrentCatalog } from '../model-feed/catalog-read';
import { useEffect, useState } from 'react';
import { invoke } from '../../host/index.ts';
import type { Plan } from '../../plan-editor';
import { Disclosure, UiIcon } from '../../ui';
import type { AvailableModel, ModelAvailabilityResult } from '../model-feed/types';
import { FeedDisclosure } from '../model-feed/FeedDisclosure';
import { factTags, modelSubtitle, rollingIdentity, shortDate } from '../model-feed/presentation';
import { modelAnnouncements } from '../model-feed/announcements';
import './trial.css';

type Entry = { key: string; model: AvailableModel; kind: 'new_model' | 'model_update'; headline: string; includedOn: string };

export function NewModels({ language, active, plans, refreshVersion, onTry, onOpenPlan, mutable = true }: {
  language: 'zh' | 'en'; active: boolean; mutable?: boolean; plans: Plan[]; refreshVersion: number; onTry(modelId?: string): void; onOpenPlan?(planId: string): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [data, setData] = useState<ModelAvailabilityResult | null>(null), [error, setError] = useState(false), [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!active) return;
    let current = true;
    void readCurrentCatalog(() => invoke<ModelAvailabilityResult>('model_availability')).then(result => { if (current) { setData(result); setError(false); } }).catch(() => { if (current) setError(true); });
    return () => { current = false; };
  }, [active, refreshVersion, retry]);
  const users = new Map<string, Plan[]>();
  for (const plan of plans) {
    const s = plan.desired.strategy;
    for (const c of [...s.candidates ?? [], ...s.economy ?? [], ...s.primary ?? [], ...s.routing?.branches.flatMap(b => [...b.candidates, ...b.primary_candidates]) ?? []]) {
      const list = users.get(c.binding_id) ?? [];
      if (!list.includes(plan)) users.set(c.binding_id, [...list, plan]);
    }
  }
  const usedBy = (model: AvailableModel) => [...new Set(model.binding_ids.flatMap(id => users.get(id) ?? []))];
  const announcements = data ? modelAnnouncements(data) : null;
  const entries: Entry[] = announcements?.entries.slice(0, 6).flatMap(entry => {
    const model = data!.models.find(m => m.model_configuration_id === entry.model_configuration_id);
    return model ? [{ key: entry.id, model, kind: entry.kind, headline: entry.headline[language], includedOn: entry.included_on }] : [];
  }) ?? [];
  const fromFeed = announcements?.fromFeed ?? false;
  function state(model: AvailableModel) {
    const plansUsing = usedBy(model);
    if (model.availability.kind === 'descriptive_only') return { tone: '', label: t('资料可查看，当前不可执行', 'Information only; unavailable for execution'), action: t('暂不可试用', 'Unavailable for trial'), primary: false, run: undefined };
    if (model.availability.kind === 'needs_newer_client') return { tone: 'warn', label: t(`需要 HiRoute ${model.availability.version}`, `Requires HiRoute ${model.availability.version}`), action: t('需要更新 HiRoute', 'Update HiRoute first'), primary: false, run: undefined };
    if (model.origin.kind === 'published') return { tone: '', label: t('仅在已发布路由中', 'Only in published routes'), action: t('不能用于新试用', 'Not available for new trials'), primary: false, run: undefined };
    if (plansUsing.length && onOpenPlan) return { tone: 'good', label: plansUsing.length > 1 ? t(`已在 ${plansUsing.length} 个路由中使用`, `Used in ${plansUsing.length} routes`) : t(`已在「${plansUsing[0].desired.display_name}」中使用`, `Used in “${plansUsing[0].desired.display_name}”`), action: t('查看模型表现', 'View model performance'), primary: false, run: () => onOpenPlan(plansUsing[0].agent_plan_id) };
    if (model.binding_ids.length) return { tone: 'info', label: t('已连接，可以直接试', 'Connected and ready to try'), action: t('试用', 'Try it'), primary: true, run: () => onTry(model.model_configuration_id) };
    return { tone: '', label: t('需要先连接', 'Needs a connection'), action: t('接入并试用', 'Connect and try'), primary: true, run: () => onTry(model.model_configuration_id) };
  }
  return <section className="new-models" aria-labelledby="new-models-title">
    <header className="home-section-head">
      <div><h2 id="new-models-title">{t('新模型', 'New models')}</h2>{fromFeed && <span>{t('由 hiroute.ai 维护 · 内容已校验', 'Maintained by hiroute.ai · Content validated')}</span>}</div>
      <button className="btn btn-quiet" disabled={error || !data} onClick={() => onTry()}>{t('试用其他模型', 'Try another model')}<UiIcon name="chevronRight" /></button>
    </header>
    {error && <div className="callout warn" role="alert"><span>{t('模型目录暂时无法刷新，请重试后开始试用。', 'The model catalog could not refresh. Retry before starting a trial.')}</span><button className="btn" onClick={() => setRetry(n => n + 1)}>{t('重试', 'Retry')}</button></div>}
    {!!data && fromFeed && <FeedDisclosure language={language} mutable={mutable} />}
    {entries.length > 0 && <div className="new-model-grid">{entries.map(({ key, model, kind, headline, includedOn }) => {
      const s = state(model);
      const tags = factTags(model.facts, language);
      return <article key={key} className="new-model-card">
        <div className="new-model-meta"><span>{modelSubtitle(model, language)} · {t(`公告收录 ${shortDate(includedOn, language)}`, `Announcement added ${shortDate(includedOn, language)}`)}</span><span className="badge info no-dot">{kind === 'model_update' ? t('模型更新', 'Update') : t('新模型', 'New')}</span></div>
        <h3>{headline || model.display_name}</h3>
        {tags.length > 0 && <div className="model-fact-tags">{tags.map(tag => <span key={tag} className="badge no-dot">{tag}</span>)}</div>}
        <p className="new-model-note">{t('从范围明确的小任务开始试用，并观察实际表现。', 'Start with a well-scoped task and observe the results.')}</p>
        <div className="new-model-foot">
          <span className={`badge no-dot ${s.tone}`}>{s.label}</span>
          <button className={`btn${s.primary ? ' btn-primary' : ''}`} disabled={error || !s.run} onClick={s.run}>{s.action}</button>
        </div>
      </article>;
    })}</div>}
    {entries.length > 0 && <Disclosure className="native-details" label={t('版本与来源', 'Version and source')} language={language}><p className="field-help">{entries.some(e => rollingIdentity(e.model.identity_stability)) ? t('同名模型可能被供应商更新；收录日期不代表实际执行的版本。', 'Providers may update a model behind the same name; the date added does not identify the version that runs.') : t('公告收录日期不代表实际部署的版本。', 'The announcement date does not identify the version actually deployed.')}</p></Disclosure>}
    {!error && data && !entries.length && <p className="field-help">{t('暂无推荐；仍可从目录选择模型试用。', 'No featured models. You can still choose a model from the catalog.')}</p>}
  </section>;
}
