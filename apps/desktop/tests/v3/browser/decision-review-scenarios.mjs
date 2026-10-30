import { scenario } from './scenario-selection.mjs';

export function decisionReviewScenarios({ fresh, routing, click, until, all, text, calls, setInput, nativeField, choose, decisionStore, control: c }) {
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const chooseIndependentOpenAI = async () => {
    const button = all('.decision-provider').find(item => item.lastElementChild?.textContent === 'OpenAI');
    assert(button, 'The OpenAI preset is missing'); button.click(); await pause(40);
  };
  return [
    scenario('desktop.decisions.suggested-origin-key', ['decision-services'], 'Restoring suggested settings clears a pending key only when the independent origin changes', async () => {
      await fresh(() => decisionStore());
      await click('模型'); await click('决策模型'); await click('添加决策模型');
      await until(() => all('.decision-mode')[1]?.getAttribute('aria-pressed') === 'true', 'independent default');
      await chooseIndependentOpenAI();
      await click('更改');
      setInput(nativeField('endpoint'), 'https://proxy.example.test/v1/decisions'); await pause(40);
      setInput(nativeField('credential'), 'proxy-fixture-key'); await pause(40);
      await click('使用建议设置');
      assert(nativeField('credential').value === '', 'Suggested settings carried a pending key across origins');
      await click('仅保存');
      await until(() => text().includes('请填写 API Key'), 'credential required after an origin reset');
      assert(calls('save_decision_service_v2').length === 0, 'An origin reset persisted the old pending key');

      setInput(nativeField('credential'), 'official-fixture-key');
      setInput(nativeField('model'), 'model-edited-within-the-same-origin'); await pause(40);
      await click('使用建议设置');
      assert(nativeField('credential').value === 'official-fixture-key', 'A same-origin suggestion reset discarded its key');
      await click('仅保存');
      await until(() => calls('save_decision_service_v2').length === 1, 'explicit same-origin save');
      const input = calls('save_decision_service_v2')[0].payload.input;
      assert(input.secret === 'official-fixture-key' && input.change.service.connection.endpoint === 'https://api.openai.com/v1/decisions', 'The new key was not paired with the intended origin');
      assert(calls('test_decision_service_v2').length === 0, 'Save-only tested implicitly');
    }),
    ...['route', 'trial'].map(context => scenario(`desktop.decisions.pending-use-${context}`, ['decision-services', 'routing-editor'], `A late save-and-test result cannot replace a newer decision draft in ${context}`, async () => {
      let store, releaseTest;
      const sourceId = 'source/review/late-test';
      await fresh(() => {
        store = decisionStore([], [c().decisionSource(sourceId)]);
        if (context === 'trial') c().handlers.trial_setup_snapshot = () => ({
          schema: 'hiroute.trial-setup-snapshot/v1',
          options: c().fixtureResponse('plan_editor_options', { input: { authoring: {} } }),
          management: { revisions: c().management.revisions, retained_api_connections: [],
            sources: c().management.sources.map(source => ({
              source_id: source.source_id, revision: source.revision, display_name: source.display_name,
              provenance: source.provenance, state: source.state, actions: source.actions,
              access_kind: source.connection_identity?.access_kind ?? 'unknown',
              models: source.models.map(model => ({ model_ref: model.model_ref, binding_id: model.binding_id, revision: model.revision,
                upstream_model_id: model.upstream_model_id, display_name: model.display_name, membership: model.membership,
                catalog_configuration_id: model.catalog_configuration_id, catalog_admitted: false, availability: model.presentation?.availability ?? 'unknown' })),
            })) },
        });
      });
      const parentName = () => context === 'route' ? all('.plan-identity-fields input')[0] : all('[data-trial-target=name] input')[0];
      if (context === 'route') await routing();
      else {
        await click('试用其他模型');
        await until(() => all('[data-trial-connected-model]')[0], 'trial model control');
        all('[data-trial-connected-model]')[0].click();
        await until(() => all('.trial-picker .trial-choice:not(:disabled)')[0], 'connected models');
        all('.trial-picker .trial-choice:not(:disabled)')[0].click();
        await until(() => !all('.trial-picker').length && parentName(), 'trial setup');
      }
      setInput(parentName(), `Retained ${context} name`); await pause(40);
      await click('决策模型'); await click('添加决策模型');
      await until(() => nativeField('source'), 'first decision draft');
      choose(nativeField('source'), sourceId); await pause(40);
      setInput(nativeField('name'), 'First saved decision'); await pause(40);
      const originalTest = c().handlers.test_decision_service_v2;
      c().handlers.test_decision_service_v2 = payload => new Promise(resolve => {
        const response = originalTest(payload); releaseTest = () => resolve(response);
      });
      await click('保存并测试');
      await until(() => releaseTest && text().includes('正在测试'), 'test stays pending after the acknowledged save');
      await click('添加决策模型');
      await until(() => nativeField('source'), 'second decision draft');
      choose(nativeField('source'), sourceId); await pause(40);
      setInput(nativeField('name'), 'Second unsaved decision'); await pause(40);
      const reads = calls('decision_services_v2').length;
      releaseTest();
      await until(() => calls('decision_services_v2').length > reads, 'late test readback');
      await pause(120);
      assert(nativeField('name')?.value === 'Second unsaved decision', 'A late test selected the old service or closed the new draft');
      assert(!all('dialog.discard-dialog[open]').length, 'A stale completion tried to navigate through the new draft');
      assert(store.services.length === 1 && calls('preview_plan_editor').length === 0, 'The newer draft was silently saved or published');
      const back = context === 'route' ? '返回路由' : '返回试用';
      await click(back);
      await until(() => all('dialog.discard-dialog[open]').length, 'caller return honors the newer draft guard');
      await click('继续编辑');
      assert(nativeField('name')?.value === 'Second unsaved decision', 'Cancelling return lost the newer draft');
      c().handlers.test_decision_service_v2 = originalTest;
      await click('仅保存');
      await until(() => parentName()?.value === `Retained ${context} name`, 'explicit use returns to the retained caller');
      const second = store.services.find(service => service.name === 'Second unsaved decision');
      assert(second && all('.decision-selector select').some(select => select.value === second.id), 'Explicit use did not choose the newer service');
      assert(calls('test_decision_service_v2').length === 1 && calls('preview_plan_editor').length === 0, 'The return implicitly tested or published');
      await click('添加决策模型');
      await until(() => nativeField('source'), 'another unsaved decision');
      choose(nativeField('source'), sourceId); await pause(40);
      setInput(nativeField('name'), 'Discard this decision'); await pause(40);
      await click(back);
      await until(() => all('dialog.discard-dialog[open]').length, 'one discard approval');
      await click('放弃修改');
      await until(() => parentName()?.value === `Retained ${context} name`, 'one approval returns to the caller');
      assert(!all('dialog.discard-dialog[open]').length, 'The caller asked the same discard guard twice');
    })),
  ];
}
