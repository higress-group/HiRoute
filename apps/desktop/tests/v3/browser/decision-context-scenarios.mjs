import { scenario } from './scenario-selection.mjs';

export function decisionContextScenarios({ fresh, click, until, all, text, calls, setInput, nativeField, decisionStore, control: c }) {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const setting = () => document.querySelector('.decision-context');
  const within = label => [...setting().querySelectorAll('button,summary')].find(b => (b.querySelector('.disclosure-label')?.textContent ?? b.textContent).trim() === label);
  async function choose(label) { const target = within(label); assert(target, 'Missing context control: ' + label); target.click(); await pause(40); }
  async function independent(provider = 'OpenAI') {
    await click('模型'); await click('决策模型'); await click('添加决策模型');
    await until(() => all('.decision-mode')[1]?.getAttribute('aria-pressed') === 'true', 'independent mode');
    const preset = provider === 'compatible' ? document.querySelector('.decision-compatible') : all('.decision-provider').find(b => b.lastElementChild?.textContent === provider);
    assert(preset, 'Missing provider'); preset.click();
    await until(() => preset.getAttribute('aria-pressed') === 'true' && setting(), 'context settings');
    setInput(nativeField('credential'), 'context-browser-fixture');
  }
  return [
    scenario('desktop.decisions.context-intent', ['decision-services'], 'Known, fallback and custom context preserve exact intent and validate before saving', async () => {
      let store; await fresh(() => { store = decisionStore(); });
      await independent();
      await until(() => setting().textContent.includes('1,050,000'), 'maintained OpenAI context');
      assert(setting().textContent.includes('产品策略值'), 'Policy value was presented as measured capacity');
      await choose('自定义'); setInput(nativeField('context_window'), '100000');
      await until(() => setting().textContent.includes('100,000'), 'custom limits on the known endpoint');
      assert(!setting().textContent.includes('产品策略值'), 'User overrides were mislabeled as the automatic policy');
      await choose('自动');
      await until(() => setting().textContent.includes('1,050,000'), 'automatic limits restored');
      await click('更改'); setInput(nativeField('endpoint'), 'https://proxy.example.test/v1/decisions');
      await until(() => setting().textContent.includes('32,000') && setting().textContent.includes('保守默认'), 'unknown endpoint fallback');
      setInput(nativeField('credential'), 'context-browser-fixture');
      await choose('自定义');
      for (const invalid of ['0', '1.5', '4294967296']) {
        setInput(nativeField('context_window'), invalid); await pause(50);
        assert(setting().querySelector('[role="alert"]') && !calls('save_decision_service_v3').length, 'Invalid window was accepted');
      }
      setInput(nativeField('context_window'), '65536');
      await choose('高级：单个问题上限');
      setInput(nativeField('context_question'), '32000');
      await until(() => setting().textContent.includes('65,536') && setting().textContent.includes('32,000'), 'independent question cap');
      await choose('自动'); await until(() => setting().textContent.includes('保守默认'), 'automatic restore');
      await choose('自定义');
      assert(nativeField('context_window').value === '65536' && nativeField('context_question').value === '32000', 'Mode switch lost parked values');
      await until(() => setting().textContent.includes('65,536'), 'custom restored');
      await click('仅保存');
      await until(() => store.services.length === 1, 'explicit context save');
      const service = store.services[0];
      assert(service.schema === 'hiroute.decision-service/v3', 'Saved a legacy service');
      assert(service.context.policy.window_tokens === 65536 && service.context.policy.per_question_tokens === 32000, 'Saved policy differs from displayed intent');
      assert(service.context.resolved.request_window_tokens === 65536 && service.context.resolved.per_question_window_tokens === 32000, 'Saved different effective limits');
      assert(!calls('test_decision_service_v3').length && !calls('preview_plan_editor').length, 'Save implicitly tested or published');
      await until(() => text().includes('65,536') && text().includes('32,000'), 'saved context summary');
    }),
    scenario('desktop.decisions.context-cross-field-error', ['decision-services'], 'A parked question limit conflict stays visible, opens its field and preserves typing focus', async () => {
      let store; await fresh(() => { store = decisionStore(); });
      await independent();
      await choose('自定义'); setInput(nativeField('context_window'), '65536');
      await choose('高级：单个问题上限'); setInput(nativeField('context_question'), '32000');
      await until(() => setting().textContent.includes('65,536'), 'custom limits');
      await choose('高级：单个问题上限');
      const request = nativeField('context_window'); request.focus(); setInput(request, '16000');
      await until(() => setting().querySelector('details').open, 'question error expands advanced settings');
      assert(document.activeElement === request, 'Cross-field validation stole typing focus');
      const alert = setting().querySelector('.decision-context-validation');
      assert(alert?.getClientRects().length && alert.textContent.includes('32,000') && alert.textContent.includes('16,000'), 'Conflict has no visible, specific explanation');
      assert(nativeField('context_question').getAttribute('aria-describedby')?.includes('-question-error'), 'Question error is not associated with its input');
      await choose('高级：单个问题上限');
      assert(!setting().querySelector('details').open && alert.getClientRects().length, 'Collapsing hid the reason saving is blocked');
      await choose('检查问题上限');
      await until(() => document.activeElement === nativeField('context_question'), 'review action focuses the question');
      assert(setting().querySelector('details').open && !calls('save_decision_service_v3').length, 'Repair changed data or failed to expand');
      setInput(nativeField('context_question'), '');
      await until(() => setting().textContent.includes('16,000') && !setting().querySelector('.decision-context-validation'), 'corrected limit resolves');
      await click('仅保存'); await until(() => store.services.length === 1, 'corrected explicit save');
      assert(store.services[0].context.policy.window_tokens === 16000 && !('per_question_tokens' in store.services[0].context.policy), 'Repair did not preserve user intent');
      assert(!calls('test_decision_service_v3').length, 'Save-only tested the provider');
    }),
    scenario('desktop.decisions.context-typesafe', ['decision-services'], 'TypeSafe and OpenRouter presets retain exact aliases, channel windows and official-only headers', async () => {
      await fresh(() => decisionStore());
      await independent('TypeSafe');
      assert(nativeField('model').value === 'jev-latest', 'TypeSafe preset lost its documented rolling alias');
      assert(document.querySelector('.decision-endpoint')?.textContent === 'https://api.typesafe.ai/v1/systemone', 'TypeSafe preset changed destination');
      await until(() => setting().querySelector('input[type="checkbox"]') && setting().textContent.includes('262,144'), 'official long context');
      setting().querySelector('input[type="checkbox"]').click();
      await until(() => setting().textContent.includes('64,000') && setting().textContent.includes('32,000'), 'provider smaller windows');
      document.querySelector('.decision-address-value button').click(); await pause(40);
      setInput(nativeField('endpoint'), 'https://api.typesafe.ai.proxy.example.test/v1/systemone');
      await until(() => !setting().querySelector('input[type="checkbox"]') && setting().textContent.includes('保守默认'), 'lookalike fallback without toggle');
      assert(!calls('save_decision_service_v3').length && !calls('test_decision_service_v3').length, 'Window preview mutated or tested');
      await fresh(() => decisionStore());
      await independent('OpenRouter');
      assert(nativeField('model').value === '~typesafe/jev-latest', 'OpenRouter preset lost its exact documented alias');
      await until(() => setting().textContent.includes('262,144'), 'OpenRouter alias context');
      assert(!setting().querySelector('input[type="checkbox"]'), 'OpenRouter borrowed a direct-provider header');
      assert(setting().textContent.includes('折算值'), 'Alias budget was not identified as a source-backed inference');
      assert(!calls('save_decision_service_v3').length && !calls('test_decision_service_v3').length, 'Preset preview mutated or tested');
    }),
    scenario('desktop.decisions.context-preview-repair', ['decision-services'], 'Preview failure and stale metadata preserve drafts and require an explicit retry', async () => {
      let store, failed = true;
      await fresh(() => {
        store = decisionStore();
        c().handlers.preview_decision_context = args => {
          if (failed) throw { code: 'MODEL_METADATA_UNAVAILABLE' };
          return c().fixtureResponse('preview_decision_context', args);
        };
      });
      await independent();
      await until(() => setting().textContent.includes('窗口暂时无法解析'), 'preview error');
      await choose('自定义'); setInput(nativeField('context_window'), '100000');
      failed = false; await choose('刷新窗口');
      await until(() => setting().textContent.includes('100,000'), 'resolved retry');
      const save = c().handlers.save_decision_service_v3;
      c().handlers.save_decision_service_v3 = () => { throw { code: 'REVISION_CONFLICT' }; };
      await click('仅保存');
      await until(() => text().includes('未保存的 Key 已清除'), 'conflict repair');
      assert(nativeField('context_window').value === '100000' && store.services.length === 0, 'Failed save discarded non-secret intent or committed');
      await pause(300);
      assert(calls('save_decision_service_v3').length === 1, 'Conflict resubmitted implicitly');
      c().handlers.save_decision_service_v3 = save;
      setInput(nativeField('credential'), 'context-reentered-fixture');
      await click('仅保存'); await until(() => store.services.length === 1, 'explicit corrected save');
    }),
  ];
}
