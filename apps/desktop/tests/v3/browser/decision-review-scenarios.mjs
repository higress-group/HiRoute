import { scenario } from './scenario-selection.mjs';

export function decisionReviewScenarios({ fresh, routing, click, until, all, text, calls, setInput, nativeField, choose, decisionStore, control: c }) {
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const chooseIndependentOpenAI = async () => {
    const button = all('.decision-provider').find(item => item.lastElementChild?.textContent === 'OpenAI');
    assert(button, 'The OpenAI preset is missing'); button.click(); await pause(40);
  };
  return [
    scenario('desktop.decisions.details-and-removal', ['decision-services', 'model-connections'], 'Details explain the current test without opaque hashes; removal is visible, confirmed, cancellable and preserves shared credentials', async () => {
      for (const language of ['zh', 'en']) {
        const t = (zh, en) => language === 'zh' ? zh : en;
        const sourceId = 'source/review/removal';
        const record = { schema: 'hiroute.decision-service/v2', id: 'decision/review/removal', revision: 3, name: 'Review decisions', connection: { kind: 'native', provider: 'openai', wire_protocol: 'openai_decisions', model: 'gpt-6-luna', endpoint: 'https://api.openai.test/v1/decisions', timeout_ms: 10000, authentication: { kind: 'shared_source', source_id: sourceId, source_lineage_digest: c().decisionSource(sourceId).lineage_digest } } };
        let store;
        await fresh(() => { store = decisionStore([record], [c().decisionSource(sourceId)]); });
        localStorage.setItem('hiroute.language', language); c().remount();
        await click(t('模型', 'Models')); await click(t('决策模型', 'Decision models'));
        await click(t('测试连接', 'Test connection'));
        await until(() => document.querySelector('.decision-test-result.passed'), 'passed connection');
        const diagnostic = () => all('.decision-test-result summary').find(item => item.textContent.includes(t('诊断详情', 'Diagnostic details')));
        // Inspect an expanded diagnostic too: hiding hashes behind a disclosure is not the repair.
        diagnostic()?.click(); await pause(40);
        assert(!text().includes('sha256:'), 'Opaque configuration hashes are still shown in connection details');
        assert(!diagnostic(), 'A successful test retains an empty or unnecessary diagnostic disclosure');
        assert(calls('test_decision_service_v3')[0].payload.input.expected_configuration_digest === store.view(record.id).configuration_digest, 'Presentation changes dropped the exact tested configuration identity');
        store.test = () => ({ outcome: 'failed', failure_code: 'DECISION_AUTHENTICATION_REJECTED' });
        await click(t('重新测试', 'Test again'));
        await until(() => document.querySelector('.decision-test-result.failed'), 'failed connection');
        assert(diagnostic(), 'A failed test lost its support diagnostic'); diagnostic().click(); await pause(40);
        assert(text().includes('DECISION_AUTHENTICATION_REJECTED') && !text().includes('sha256:'), 'Failure diagnostics expose hashes or hide the typed failure code');
        const removal = () => all('.decision-removal button').find(item => item.textContent.trim() === t('移除模型', 'Remove model'));
        assert(removal() && !removal().disabled && !removal().closest('details'), 'The model detail has no usable standalone removal section');
        removal().scrollIntoView({ block: 'center' });
        const bounds = removal().getBoundingClientRect(); assert(bounds.top >= 0 && bounds.bottom <= innerHeight, 'The standalone removal action cannot be brought into view');
        const confirm = () => {
          const button = all('[role="dialog"] button').find(item => item.textContent.trim() === t('移除模型', 'Remove model'));
          assert(button && !button.disabled, 'The explicit removal confirmation is missing'); button.click();
        };
        removal().click(); await until(() => all('[role="dialog"]').length, 'removal confirmation');
        assert(calls('save_decision_service_v2').length === 0 && text().includes(t('仅移除这个决策模型，保留 API 接入及其 Key。', 'Remove only this decision model; retain its API connection and keys.')), 'Opening removal mutates state or fails to explain shared credential retention');
        await click(t('取消', 'Cancel'));
        assert(store.services.length === 1 && calls('save_decision_service_v2').length === 0, 'Cancel removed the decision configuration');
        const save = c().handlers.save_decision_service_v2;
        c().handlers.save_decision_service_v2 = () => { throw { code: 'DECISION_IN_USE' }; };
        removal().click(); await until(() => all('[role="dialog"]').length, 'blocked removal confirmation'); confirm();
        await until(() => text().includes(t('仍被引用，请先调整引用再移除', 'Still referenced. Update references before removing')), 'reference rejection');
        assert(store.services.length === 1 && text().includes(record.name), 'A rejected removal hid the existing configuration');
        assert(all('[role="dialog"] .btn-danger')[0]?.disabled, 'A rejected removal still allows resubmitting the same unconfirmed intent');
        await click(t('取消', 'Cancel'));
        c().handlers.save_decision_service_v2 = save;
        const sourceBefore = JSON.stringify(store.sources), managedBefore = JSON.stringify(c().management.sources);
        removal().click(); await until(() => all('[role="dialog"]').length, 'successful removal confirmation'); confirm();
        await until(() => store.services.length === 0 && text().includes(t('已移除。', 'Removed.')), 'removed configuration readback');
        const input = calls('save_decision_service_v2').at(-1).payload.input;
        assert(input.change.id === record.id && input.change.expected_revision === 3 && input.change.service === null && input.secret === null, 'Removal changed its exact revision transaction or submitted a secret');
        assert(JSON.stringify(store.sources) === sourceBefore && JSON.stringify(c().management.sources) === managedBefore, 'Removing a decision configuration modified its API source or keys');
        assert(calls('test_decision_service_v3').length === 2 && calls('preview_plan_editor').length === 0, 'Removal implicitly tested or published');
      }
    }),
    scenario('desktop.decisions.extension-removal', ['decision-services'], 'Custom extensions share the visible confirmed removal action', async () => {
      let store;
      const record = { schema: 'hiroute.decision-service/v2', id: 'decision/review/extension', revision: 2, name: 'Review extension', connection: { kind: 'custom', endpoint: 'https://extension.example.test/decide', timeout_ms: 10000, auth_header: null } };
      await fresh(() => { store = decisionStore([record]); });
      await click('模型'); await click('决策模型');
      const removal = all('.decision-removal button').find(item => item.textContent.trim() === '移除扩展');
      assert(removal && !removal.disabled && !removal.closest('details'), 'The extension detail has no standalone removal action'); removal.scrollIntoView({ block: 'center' }); removal.click();
      await until(() => all('[role="dialog"]').length, 'extension removal confirmation');
      assert(text().includes('移除「Review extension」？') && calls('save_decision_service_v2').length === 0, 'Extension removal is not separately identified or confirmed');
      await click('取消'); assert(store.services.length === 1, 'Cancel removed an extension');
    }),
    scenario('desktop.decisions.removal-references', ['decision-services', 'routing-editor'], 'Removal blocks visible references and never adopts a newer revision behind an open confirmation', async () => {
      const sourceId = 'source/review/references';
      const record = { schema: 'hiroute.decision-service/v2', id: 'decision/review/references', revision: 3, name: 'Referenced decisions', connection: { kind: 'native', provider: 'openai', wire_protocol: 'openai_decisions', model: 'gpt-6-luna', endpoint: 'https://api.openai.test/v1/decisions', timeout_ms: 10000, authentication: { kind: 'shared_source', source_id: sourceId, source_lineage_digest: c().decisionSource(sourceId).lineage_digest } } };
      await fresh(() => {
        decisionStore([record], [c().decisionSource(sourceId)]);
        c().desktop.catalog.plans[0].desired.strategy.classifier = { kind: 'decision_service_v2', service: structuredClone(record) };
      });
      await click('模型'); await click('决策模型'); await click('移除模型');
      await until(() => all('[role="dialog"]').length, 'referenced removal');
      assert(all('[role="dialog"] .btn-danger')[0]?.disabled && all('[role="dialog"] .callout.warn button').some(item => item.textContent.includes('日常编码')), 'Current route references are not explained and blocking');
      assert(calls('save_decision_service_v2').length === 0, 'A referenced confirmation submitted a removal');
      await click('取消');
      let store, releaseTest;
      await fresh(() => {
        store = decisionStore([record], [c().decisionSource(sourceId)]);
        const original = c().handlers.test_decision_service_v3;
        c().handlers.test_decision_service_v3 = payload => new Promise(resolve => {
          const admitted = original(payload); releaseTest = () => resolve(admitted);
        });
      });
      await click('模型'); await click('决策模型'); await click('测试连接');
      await until(() => releaseTest, 'pending test'); await click('移除模型');
      await until(() => all('[role="dialog"]').length, 'frozen removal identity');
      store.services[0] = { ...store.services[0], revision: 4 };
      releaseTest();
      await until(() => text().includes('这份配置已发生变化'), 'new revision invalidates removal confirmation');
      assert(all('[role="dialog"] .btn-danger')[0]?.disabled && calls('save_decision_service_v2').length === 0, 'An open confirmation adopted the newer revision');
      await click('取消'); assert(store.services[0].revision === 4, 'Cancelling the stale confirmation modified the newer configuration');
    }),
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
      assert(calls('test_decision_service_v3').length === 0, 'Save-only tested implicitly');
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
      const originalTest = c().handlers.test_decision_service_v3;
      c().handlers.test_decision_service_v3 = payload => new Promise(resolve => {
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
      c().handlers.test_decision_service_v3 = originalTest;
      await click('仅保存');
      await until(() => parentName()?.value === `Retained ${context} name`, 'explicit use returns to the retained caller');
      const second = store.services.find(service => service.name === 'Second unsaved decision');
      assert(second && all('.decision-selector select').some(select => select.value === second.id), 'Explicit use did not choose the newer service');
      assert(calls('test_decision_service_v3').length === 1 && calls('preview_plan_editor').length === 0, 'The return implicitly tested or published');
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
