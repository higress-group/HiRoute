import { branchRouting } from '../../../src/features/decision-services/types.ts';
import { scenario } from './scenario-selection.mjs';

export function extensionDecisionScenarios({ fresh, routing, click, until, all, calls, control }) {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const custom = { id: 'decision/extension-fixture', revision: 3, name: 'Published custom decision extension with a long name', connection: { kind: 'custom', endpoint: 'https://decision.example/v1/decisions', timeout_ms: 3000 } };
  const native = { ...custom, id: 'decision/native-fixture', name: 'Built-in fixture', connection: { kind: 'system_one', provider: 'compatible', model: 'synthetic', endpoint: 'https://decision.example/systemone', timeout_ms: 3000, auth_header: { name: 'Authorization', value_secret_ref: 'fixture' } } };
  const editorScenarios = ['zh', 'en'].flatMap(language => [false, true].map(branches => scenario(
    `desktop.extension.${branches ? 'branches' : 'smart'}.${language}`, ['extension-decision'],
    `Extension ${branches ? 'categories' : 'difficulty routing'} preserves parked settings (${language})`, async () => {
      let plan;
      await fresh(() => {
        control().handlers.decision_services_v2 = () => ({ services: [custom, native].map(service => control().decisionView(service)) });
        control().handlers.desktop_snapshot = payload => {
          const snapshot = structuredClone(control().fixtureResponse('desktop_snapshot', payload));
          plan = snapshot.catalog.plans[0];
          const strategy = plan.desired.strategy;
          strategy.classifier = { kind: 'decision_service', service: custom };
          strategy.judgment.degree.instructions = '   ';
          strategy.judgment.competence.criteria[1] = '\n\t';
          plan.desired.delegation_enabled = false;
          if (branches) {
            const route = branchRouting(custom);
            route.judgment = structuredClone(strategy.judgment);
            route.branches.forEach((branch, index) => {
              branch.name = index ? 'Review / 审稿' : 'Draft / 起草';
              branch.condition = 'Review the supplied article and report concrete changes without assuming its author or source. 根据当前任务选择类别。';
              branch.candidates = structuredClone(strategy.economy);
              branch.primary_candidates = structuredClone(strategy.primary);
              branch.judgment = index ? null : structuredClone(strategy.judgment);
            });
            plan.desired.mode = 'custom_branches';
            plan.desired.strategy = { mode: 'branches', routing: route };
          }
          return snapshot;
        };
        control().handlers.plan_editor_options = payload => {
          const result = structuredClone(control().fixtureResponse('plan_editor_options', payload));
          if (payload?.input?.authoring) {
            control().handlers.desktop_snapshot({});
            const editor = result.authoring.editor;
            editor.mode = plan.desired.mode;
            editor.delegation_enabled = false;
            if (branches) editor.branch_routing = structuredClone(plan.desired.strategy.routing);
            else editor.smart = { ...editor.smart, ...structuredClone(plan.desired.strategy) };
          }
          return result;
        };
      });
      const t = (zh, en) => language === 'zh' ? zh : en;
      localStorage.setItem('hiroute.language', language);
      control().remount();
      if (language === 'zh') await routing();
      else { await click('Smart routing'); await until(() => all('.master-list .list-row').length, 'route list'); all('.master-list .list-row')[0].click(); }
      await until(() => document.querySelector('.decision-selector'), 'extension editor');
      assert(!document.querySelector('.decision-judgment-fields'), 'Extension exposed judgment fields');
      assert(!document.querySelector('.trust-level-control'), 'Extension exposed threshold control');
      await click(t('决策模型', 'Decision model'));
      await until(() => document.querySelector('.decision-judgment-fields'), 'restored native fields');
      const field = document.querySelector(`[data-decision-field="${branches ? 'branch-0' : 'smart'}-degree-instructions"]`);
      assert(field?.value === '   ', 'Switching sources discarded the parked prompt');
      const floor = document.querySelector(`[data-decision-field="${branches ? 'branch-0' : 'smart'}-floor"]`);
      const before = floor.value;
      for (let parent = floor.parentElement; parent; parent = parent.parentElement) if (parent.tagName === 'DETAILS') parent.open = true;
      floor.focus();
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(floor, '');
      floor.dispatchEvent(new Event('input', { bubbles: true }));
      await until(() => floor.getAttribute('aria-invalid') === 'true', 'visible temporary numeric error');
      await click(t('自定义扩展', 'Custom extension'));
      await until(() => !document.querySelector('.decision-judgment-fields'), 'parked empty numeric field');
      await click(t('决策模型', 'Decision model'));
      await until(() => document.querySelector('.decision-judgment-fields'), 'restored native numeric field');
      assert(document.querySelector(`[data-decision-field="${branches ? 'branch-0' : 'smart'}-floor"]`).value === before, 'Temporary numeric input corrupted the parked integer');
      await click(t('自定义扩展', 'Custom extension'));
      await until(() => !document.querySelector('.decision-judgment-fields'), 'custom fields hidden again');
      assert(!calls('test_decision_service_v2').length, 'Source switching implicitly probed the connection');
      for (const element of all('.decision-selector, .branch-routing-card, .field-label')) {
        const rect = element.getBoundingClientRect();
        assert(rect.width > 30 && rect.right <= innerWidth + 1, 'Editor label or control overflowed');
      }
      window.extensionCapture = { language, branches, state: 'custom-after-native-switch' };
    }
  )));
  return [...editorScenarios, ...['zh', 'en'].map(language => scenario(`desktop.extension.help.${language}`, ['extension-decision'], `Current custom protocol help (${language})`, async () => {
    await fresh(() => { control().handlers.decision_services_v2 = () => ({ services: [control().decisionView(custom)] }); });
    const t = (zh, en) => language === 'zh' ? zh : en;
    localStorage.setItem('hiroute.language', language); control().remount();
    await click(t('模型', 'Models')); await click(t('决策模型', 'Decision models'));
    await click(t('查看接入协议', 'View protocol'));
    await until(() => document.querySelector('.classifier-protocol'), 'protocol dialog');
    await click(t('复制 curl', 'Copy curl'));
    await until(() => control().clipboard.length > 0, 'current curl');
    const curl = control().clipboard.at(-1);
    assert(curl.includes('"route"') && curl.includes('"preceding_stage"') && !curl.includes('"assessment_target"'), 'Protocol help copied a replaced request');
    assert(all('.classifier-protocol pre')[1].textContent.includes('"group"'), 'Protocol help returned a replaced response');
  }))];
}
