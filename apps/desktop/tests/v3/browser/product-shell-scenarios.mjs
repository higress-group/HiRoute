import { scenario, scenarioCatalog, runScenarios } from './scenario-selection.mjs';
import { productShellRequired } from './scenario-requirements.mjs';
const c = () => window.productShell;
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
const visible = element => Boolean(element?.getClientRects().length);
const all = selector => [...document.querySelectorAll(selector)].filter(visible);
const text = () => document.body.innerText;
const button = label => all('button').find(element => element.textContent.trim() === label);
const calls = command => c().commands.filter(item => item.command === command);
async function until(predicate, label, timeout = 6000) {
  const deadline = Date.now() + timeout;
  while (!predicate()) { if (Date.now() > deadline) throw new Error(`Timed out: ${label}; ${text().slice(-500)}`); await pause(25); }
  await pause(30);
}
async function click(label) { await until(() => button(label), label); button(label).click(); await pause(40); }
async function fresh(setup = () => {}) {
  c().reset(); setup(); await until(() => text().includes('仅在本机运行'), 'ready DesktopApp');
}
function setInput(input, value) {
  assert(input, 'Expected an editable input');
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}
const operation = (state = 'running', id = 'operation/shell') => ({ operation_id: id, state, sequence: state === 'succeeded' ? 2 : 1, cancellable: false, safe_error_code: null });
const feedback = () => document.querySelector('[data-operation-phase]');
async function pending() {
  await fresh(() => {
    c().desktop.pending = { plan_id: 'plan/daily', operation_id: 'operation/shell', idempotency_key: 'key/shell' };
    c().handlers.observe_operation = () => operation();
  });
  await until(() => feedback()?.dataset.operationState === 'running', 'observed operation');
}
async function routing() {
  await click('智能路由');
  await until(() => all('.master-list .list-row').length, 'saved routes');
  all('.master-list .list-row')[0].click();
  await until(() => document.querySelector('.plan-identity-fields input'), 'route editor');
}
const scenarios = [
  scenario('desktop.models.selection-survives-save-refresh', ['model-connections'], 'Renaming a selected connection does not jump back to the previous save result', async () => {
    let a; let b; let beforeB; let latestChange;
    await fresh(() => {
      b = c().management.sources.find(source => source.source_id === 'source/bailian/coding');
      a = structuredClone(b); a.source_id = 'source/selection-a'; a.display_name = '接入 A';
      a.models = a.models.map(model => ({ ...model, model_ref: `${model.model_ref}/a`, binding_id: `${model.binding_id}/a` }));
      c().management.sources = [a, b]; beforeB = JSON.stringify(b);
      c().handlers.get_compute_save_result = payload => ({ ...c().fixtureResponse('get_compute_save_result', payload), source_id: b.source_id });
      c().handlers.preview_compute_save = payload => { latestChange = payload.change; return c().fixtureResponse('preview_compute_save', payload); };
      c().handlers.apply_compute_save = payload => {
        if (latestChange?.subject.kind === 'saved_source' && latestChange.subject.source_id === a.source_id && latestChange.edit?.action === 'rename') {
          a.display_name = latestChange.edit.display_name; a.revision += 1;
        }
        return c().fixtureResponse('apply_compute_save', payload);
      };
    });
    await click('模型'); await click('添加模型');
    await until(() => all('button').some(item => item.textContent.includes('新建 API 接入')), 'new connection entry');
    all('button').find(item => item.textContent.includes('新建 API 接入')).click();
    await until(() => document.querySelector('[role="dialog"] input[type="password"]'), 'key field');
    setInput(document.querySelector('[role="dialog"] input[type="password"]'), 'synthetic-selection-key');
    await click('检查接入');
    await until(() => all('.model-result-row input[type="checkbox"]').length, 'model choices');
    all('.model-result-row input[type="checkbox"]')[0].click(); await pause(40);
    await click('保存接入');
    await until(() => !document.querySelector('[role="dialog"]') && calls('get_compute_save_result').length, 'completed initial save');
    await click('按接入');
    await until(() => document.querySelector('.models-feature .detail-identity h2')?.textContent === b.display_name, 'saved connection focus');
    all('.models-feature .master-list .list-row').find(row => row.textContent.includes('接入 A')).click(); await pause(40);
    await click('重命名');
    await until(() => document.querySelector('[role="dialog"] input'), 'rename field');
    setInput(document.querySelector('[role="dialog"] input'), '接入 A 已改名');
    const snapshots = calls('compute_management_snapshot').length;
    await click('保存名称');
    await until(() => calls('compute_management_snapshot').length > snapshots && !document.querySelector('[role="dialog"]'), 'rename refresh');
    assert(document.querySelector('.models-feature .detail-identity h2')?.textContent === '接入 A 已改名', 'Refresh jumped to the previous saved connection');
    assert(JSON.stringify(b) === beforeB, 'Another connection changed while renaming A');
  }),
  scenario('desktop.models.connection-template-directory-layout', ['model-connections'], 'The production stylesheet renders searchable template rows and pagination', async () => {
    await fresh(); await click('模型'); await click('添加模型');
    await until(() => all('button').some(item => item.textContent.includes('新建 API 接入')), 'new connection entry');
    all('button').find(item => item.textContent.includes('新建 API 接入')).click();
    await until(() => all('.mc-template-row').length > 0, 'template directory');
    assert(getComputedStyle(document.querySelector('.mc-template-directory')).display === 'grid', 'Production directory layout missing');
    for (const row of all('.mc-template-row')) {
      assert(getComputedStyle(row).display === 'flex' && getComputedStyle(row).alignItems === 'center', 'Template row is not aligned');
      assert(getComputedStyle(row.querySelector('.field-help')).display === 'block', 'Template protocol is not on its own line');
    }
    const selected = document.querySelector('.mc-template-row input:checked');
    assert(selected?.closest('.mc-template-row').classList.contains('selected'), 'Selection styling missing');
    setInput(document.querySelector('.mc-template-directory input[type="search"]'), '百炼');
    await until(() => all('.mc-template-row').every(row => row.textContent.includes('百炼')), 'filtered templates');
    assert(all('.mc-template-row').length > 0, 'Search removed matching templates');
    assert(getComputedStyle(document.querySelector('.mc-template-pagination')).display === 'flex', 'Pagination layout missing');
    const close = all('[role="dialog"] button[aria-label="取消添加"]')[0];
    assert(close, 'Template dialog close control missing');
    close.click();
    await until(() => !document.querySelector('[role="dialog"]'), 'closed template dialog');
  }),
  scenario('desktop.models.connection-rename', ['model-connections'], 'Rename is scoped to one saved connection without replacing its models or keys', async () => {
    await fresh(() => { c().management.sources = [c().management.sources.find(source => source.source_id === 'source/bailian/coding')]; });
    await click('模型'); await click('按接入'); await click('重命名');
    await until(() => document.querySelector('[role="dialog"] input'), 'rename input');
    setInput(document.querySelector('[role="dialog"] input'), '团队百炼'); await click('保存名称');
    await until(() => calls('preview_compute_save').length === 1, 'rename preview');
    const change = calls('preview_compute_save')[0].payload.change;
    assert(change.edit.action === 'rename' && change.edit.display_name === '团队百炼', 'Rename intent missing');
    assert(change.subject.source_id === 'source/bailian/coding' && change.selected_model_refs.length === 0 && change.key_edits.length === 0, 'Rename replaced a model or credential selection');
  }),
  scenario('desktop.models.delete-reference-block', ['model-connections'], 'A disabled route reference remains visible and prevents Apply', async () => {
    await fresh(() => {
      c().management.sources = [c().management.sources.find(source => source.source_id === 'source/bailian/coding')];
      const plan = c().desktop.catalog.plans[0]; plan.head.status = 'disabled';
      c().handlers.preview_compute_save = payload => ({ ...c().fixtureResponse('preview_compute_save', payload), affected_plan_refs: [plan.agent_plan_id] });
    });
    await click('模型'); await click('移除模型');
    await until(() => text().includes('仍被引用，请先调整引用再删除。'), 'reference blocker');
    assert(text().includes('这是最后一个模型，将一并删除所属接入。'), 'Last-model consequence was hidden');
    const remove = all('[role="dialog"] button').find(item => item.textContent.trim() === '删除接入');
    assert(remove?.disabled, 'Referenced connection could be deleted');
    remove.click(); await pause(50);
    assert(calls('apply_compute_save').length === 0, 'Disabled button dispatched Apply');
    assert(calls('preview_compute_save')[0].payload.change.edit.action === 'delete', 'Last model removal did not use explicit delete');
    const deleteInDialog = () => all('[role="dialog"] button').find(item => item.textContent.trim() === '删除接入');
    c().handlers.refresh_route_references = () => { throw new Error('publication unavailable'); };
    await click('更新引用状态');
    await until(() => text().includes('引用状态暂未更新'), 'failed reference refresh');
    assert(button('更新引用状态') && !button('更新引用状态').disabled, 'Failed refresh lost its retry entry');
    assert(deleteInDialog()?.disabled, 'Failed refresh released a reference');
    c().handlers.refresh_route_references = () => undefined;
    await click('更新引用状态');
    await until(() => !document.querySelector('[role="dialog"]'), 'refresh closes stale preview');
    await click('移除模型');
    await until(() => text().includes('仍被引用，请先调整引用再删除。'), 'fresh disabled reference');
    assert(deleteInDialog()?.disabled, 'Checkpoint bypassed a disabled route');
    assert(calls('preview_compute_save').length === 2, 'Reopen reused an old deletion preview');
    assert(calls('apply_compute_save').length === 0, 'Reference refresh automatically deleted configuration');
    // A separately completed publication can release historical references. Its
    // new deletion preview is still required before the user explicitly deletes.
    c().handlers.refresh_route_references = () => {
      c().handlers.preview_compute_save = payload => ({ ...c().fixtureResponse('preview_compute_save', payload), affected_plan_refs: [] });
    };
    await click('更新引用状态');
    await until(() => !document.querySelector('[role="dialog"]'), 'completed historical reference refresh');
    await click('移除模型');
    await until(() => deleteInDialog() && !deleteInDialog().disabled, 'new unreferenced preview');
    assert(calls('apply_compute_save').length === 0, 'Refresh was mistaken for delete consent');
  }),
  scenario('desktop.models.append-preserves-existing', ['model-connections'], 'Appending selects only new models and retains a disabled connection', async () => {
    await fresh(() => {
      c().management.sources = [c().management.sources.find(source => source.source_id === 'source/bailian/coding')];
      c().management.sources[0].state = 'disabled';
      c().handlers.check_saved_model_connection = payload => {
        const checked = c().fixtureResponse('check_saved_model_connection', payload);
        checked.candidate.models.push({ ...checked.candidate.models[0], model_ref: 'model-ref/new', upstream_model_id: 'new-model', display_name: 'New model' });
        return checked;
      };
    });
    await click('模型'); await click('向此接入添加模型');
    await until(() => all('[role="dialog"] input[type="checkbox"]').length === 2, 'append inventory');
    const checks = all('[role="dialog"] input[type="checkbox"]');
    assert(checks[0].disabled && checks[0].checked, 'Existing model was editable as a removal');
    checks[1].click(); await click('添加 1 个模型');
    await until(() => calls('preview_compute_save').length === 1, 'append preview');
    const change = calls('preview_compute_save')[0].payload.change;
    assert(change.edit.action === 'append_models' && change.intent === 'save_disabled', 'Append lost explicit action or disabled state');
    assert(JSON.stringify(change.selected_model_refs) === '["model-ref/new"]' && change.key_edits.length === 0, 'Append modified existing model/key selection');
  }),
  scenario('desktop.models.remove-only-selected', ['model-connections'], 'Removing one model retains the connection and sends exactly the selected saved model', async () => {
    await fresh(() => { c().management.sources = [c().management.sources[0]]; });
    await click('模型'); await click('移除模型');
    await until(() => all('[role="dialog"] button').some(item => item.textContent.trim() === '移除模型' && !item.disabled), 'remove preview');
    const change = calls('preview_compute_save')[0].payload.change;
    assert(change.edit.action === 'remove_models' && change.selected_model_refs.length === 1 && change.key_edits.length === 0, 'Remove scope replaced connection or keys');
    all('[role="dialog"] button').find(item => item.textContent.trim() === '移除模型').click();
    await until(() => calls('apply_compute_save').length === 1, 'single remove apply');
  }),
  scenario('desktop.models.tool-check-selection', ['model-connections'], 'A tool check retains the chosen model with its fresh reference without selecting new inventory', async () => {
    await fresh(); await click('模型'); await click('添加模型');
    await until(() => all('button').some(item => item.textContent.includes('新建 API 接入')), 'add API entry');
    all('button').find(item => item.textContent.includes('新建 API 接入')).click();
    await until(() => document.querySelector('[role="dialog"] input[type="password"]'), 'API key input');
    setInput(document.querySelector('[role="dialog"] input[type="password"]'), 'synthetic-tool-check-input');
    c().handlers.check_registered_model_connection = payload => {
      const result = c().fixtureResponse('check_registered_model_connection', payload);
      if (payload.request.inference_model_id) {
        result.inference = 'verified';
        result.candidate.models = result.candidate.models.map(model => ({ ...model, model_ref: `${model.model_ref}/checked` }));
      }
      return result;
    };
    await click('检查接入');
    await until(() => all('.model-result-row input[type="checkbox"]').length === 2, 'inventory');
    assert(all('.model-result-row input:checked').length === 0, 'Inventory selected models without consent');
    all('.model-result-row input[type="checkbox"]')[0].click(); await pause(40);
    await click('验证工具调用（仅所选模型，可能计费）');
    await until(() => calls('check_registered_model_connection').length === 2 && button('保存接入'), 'tool check completed');
    assert(all('.model-result-row input:checked').length === 1, 'Tool check lost the explicit model selection');
    assert(!button('保存接入').disabled, 'Successful tool check disabled save');
    await click('保存接入');
    await until(() => calls('preview_compute_save').length === 1, 'save preview');
    const preview = JSON.stringify(calls('preview_compute_save')[0].payload);
    assert(preview.includes('model-ref/qwen3-coder-plus/checked') && !preview.includes('model-ref/qwen3-max/checked'), 'Save did not use only the selected fresh model reference');
  }),
  scenario('desktop.routing.disabled-header', ['routing-editor'], 'A stopped route stays visibly disabled while its editor has unsaved changes', async () => {
    await fresh(() => {
      c().desktop.catalog.plans[0].head.status = 'disabled';
    });
    await routing();
    const header = () => document.querySelector('.plan-editor .editor-header').textContent;
    assert(header().includes('已停用') && !header().includes('已启用'), 'Stopped route is labelled active');
    setInput(document.querySelector('.plan-identity-fields input'), 'Stopped route edit'); await pause(40);
    assert(header().includes('已停用') && header().includes('有未发布更改'), 'Editing hid the stopped call state');
  }),
  scenario('desktop.quality.evidence-return', ['routing-editor', 'sessions'], 'Quality evidence names the exact request, preserves it on refresh and returns to the same filtered performance view', async () => {
    const session = 'session/quality-link';
    const selectedId = 'request/quality-selection';
    let failExactOnce = false;
    const now = Date.now();
    const identity = { plan_revision: 8, selected_branch_id: 'smart_saving', executed_branch_id: 'smart_saving', model_configuration_id: 'model/evidence', profile_digest: 'profile/evidence', attribution: 'single', group: 'primary', candidate_index: 0 };
    const sample = { ...identity, segment_id: 'stage/evidence', session_id: session, plan_id: 'plan/daily-coding',
      native_model: 'quality-evidence-model', reasoning_profile_id: 'low', first_turn_ordinal: 1, last_observed_turn_ordinal: 1,
      first_at_ms: now - 1000, last_at_ms: now - 1000, history_partial: false,
      first_request_id: selectedId, last_request_id: selectedId, execution_evidence_available: true,
      branch_execution: { group: 'primary', candidate_index: 0, policy: { name: 'Smart saving', floor_millis: 500 } },
      selection: { execution_group: 'regular', selection_reason: 'simple_task', simple_probability: .95, simple_threshold_millis: 800 },
      assessment: { score: .5, target_from_ordinal: 1, target_through_ordinal: 1, partial: false, evidence_available: false },
    };
    const rows = ['request/old-history', selectedId].map((id, index) => ({ request_id: id, session_id: session, started_at_ms: now - 2000 + index * 1000, final_native_model: index ? 'quality-evidence-model' : 'earlier-model', content_completeness: 'complete', within_request_fallback: false }));
    const reads = view => calls('observation_read').filter(call => call.payload.request?.intent?.view === view);
    const query = call => call.payload.request.intent.query;
    await fresh(() => {
      c().handlers.observation_read = payload => {
        const { view, query: q } = payload.request.intent;
        if (view === 'plan_quality') return { samples: [sample], summary: { models: [{ execution: identity, native_model: 'quality-evidence-model', reasoning_profile_id: 'low', scored_stage_count: 1, unrated_stage_count: 0, average_score: .5 }], scored_stage_count: 1, unrated_stage_count: 0, session_count: 1, available_revisions: [8] }, next_cursor: null };
        if (view === 'sessions') return { sessions: [{ session_id: session, agent_id: '', request_count: 2, fallback_request_count: 0, last_request_at_ms: now - 1000, correlation_kind: 'agent_supplied', content_completeness: 'complete' }], next_cursor: null };
        if (view === 'timeline') {
          if (q.request_id && failExactOnce) { failExactOnce = false; throw { code: 'LOCAL_SERVICE_UNAVAILABLE' }; }
          return { requests: q.request_id ? rows.filter(row => row.request_id === q.request_id) : rows, next_cursor: null };
        }
        if (view === 'catalog') return { contents: [{ content_id: q.request_id, role: 'user', kind: 'text', state: 'complete', direction: 'request_input', media_type: 'text/plain', message_occurrence_id: 'user' }], transcript_roots: [], roots_partial: false, next_cursor: null };
        if (view === 'content') return { state: 'complete', chunks: [{ text: q.content_id === selectedId ? 'SELECTED_REQUEST_WITH_REPLAYED_HISTORY' : 'EARLIER_REQUEST', original_byte_offset: 0 }], next_cursor: null };
        return c().fixtureResponse('observation_read', payload);
      };
    });
    await routing(); await click('模型表现');
    await until(() => all('[data-model-configuration="model/evidence"] .quality-view-stages').length, 'observed model');
    all('[data-model-configuration="model/evidence"] .quality-view-stages')[0].click();
    await until(() => all('.quality-score-filter').length, 'stage filter');
    const filter = all('.quality-score-filter')[0]; filter.value = 'high'; filter.dispatchEvent(new Event('change', { bubbles: true }));
    await until(() => query(reads('plan_quality').at(-1)).competence === 'meets_floor' && all('.quality-assessment-details summary').length, 'filtered stage');
    all('.quality-assessment-details summary')[0].click(); await click('查看选择请求');
    await until(() => text().includes('SELECTED_REQUEST_WITH_REPLAYED_HISTORY'), 'exact selected request body');
    assert(reads('timeline').some(call => query(call).request_id === selectedId), 'Quality link omitted the request identity');
    assert(all('.session-request-focus')[0]?.dataset.requestId === selectedId, 'Exact evidence has no visible request focus');
    assert(all('.session-request-focus')[0].textContent.includes('quality-evidence-model'), 'Focus omits the actual model');
    const beforeRefresh = reads('sessions').length;
    window.dispatchEvent(new Event('focus'));
    await until(() => reads('sessions').length > beforeRefresh && all('.session-request-focus')[0]?.dataset.requestId === selectedId && text().includes('SELECTED_REQUEST_WITH_REPLAYED_HISTORY'), 'foreground refresh retains exact request');
    await click('返回模型表现');
    await until(() => all('.quality-score-filter')[0]?.value === 'high', 'return retains stage filter');
    assert(query(reads('plan_quality').at(-1)).plan_revision === 8 && query(reads('plan_quality').at(-1)).from_ms > 0, 'Return lost current revision or seven-day scope');
    failExactOnce = true;
    await click('查看选择请求');
    await until(() => all('.session-detail-feedback[data-error-code]').length, 'exact request read failure');
    const beforeRetry = reads('timeline').length;
    await click('重试');
    await until(() => !all('.session-detail-feedback[data-error-code]').length && all('.session-request-focus')[0]?.textContent.includes('quality-evidence-model'), 'retry preserves exact evidence');
    assert(reads('timeline').slice(beforeRetry).some(call => query(call).request_id === selectedId), 'Retry silently opened the complete session');
    await click('查看完整会话');
    await until(() => !all('.session-request-focus').length && all('.request-timeline').length, 'explicit full-session navigation');
  }),
  scenario('desktop.models.return-from-decisions', ['model-connections', 'decision-services'], 'Returning from a decision connection save uses the current configuration revision for credential changes', async () => {
    let services = [];
    await fresh(() => {
      c().handlers.decision_services = () => ({ services: structuredClone(services) });
      c().handlers.save_decision_service = ({ input }) => {
        services = [structuredClone(input.change.service)];
        c().management.revisions.target += 1;
        return { state: 'succeeded' };
      };
      c().handlers.preview_compute_save = ({ change }) => {
        if (change.expected_revisions.target !== c().management.revisions.target) throw { code: 'REVISION_CONFLICT' };
        return c().fixtureResponse('preview_compute_save', { change });
      };
    });
    await click('模型');
    await until(() => all('.oc-models .master-list button').length, 'general models');
    await click('决策模型'); await click('添加决策模型');
    setInput(all('.decision-connection-form [name="credential"]')[0], 'synthetic-decision-key');
    await click('仅保存'); await until(() => services.length && !all('.decision-connection-form').length, 'saved decision connection');
    await click('通用模型');
    await until(() => all('.oc-models .master-list button').some(item => item.textContent.includes('Qwen3-Coder-Plus')), 'Bailian model');
    all('.oc-models .master-list button').find(item => item.textContent.includes('Qwen3-Coder-Plus')).click(); await pause(40);
    await click('管理凭据'); await click('保存凭据');
    await until(() => calls('preview_compute_save').length > 0, 'credential preview');
    assert(calls('preview_compute_save').at(-1).payload.change.expected_revisions.target === c().management.revisions.target, 'Credential change reused the revision from before the decision save');
    await until(() => text().includes('接入凭据已保存'), 'credential save completed');
  }),
  scenario('desktop.routing.follow-up-preference', ['routing-editor', 'decision-services'], 'New routes allow model switching; each mode retains the choice in its saved draft', async () => {
    await fresh(); await routing(); await click('新建智能路由');
    const fields = all('.plan-identity-fields input');
    setInput(fields[0], 'Follow-up preference'); setInput(fields[1], 'Check new route defaults');
    const mode = async label => {
      all('.mode-card').find(item => item.querySelector('strong')?.textContent === label).click();
      await pause(40);
      const details = [...document.querySelectorAll('.plan-editor details')].find(item => item.querySelector('summary')?.textContent.includes('追问设置'));
      assert(details, 'Follow-up settings are missing');
      if (!details.open) { details.querySelector('summary').click(); await pause(40); }
      assert(details.textContent.includes('模型不可用时仍会自动切换'), 'Automatic switching caveat is missing');
      assert(!details.querySelector('select'), 'Default branch selection is mixed into follow-up settings');
      return details.querySelector('input[type="checkbox"]');
    };
    const smart = await mode('智能省钱');
    assert(smart.checked && smart.closest('label').textContent.includes('追问时允许换模型'), 'New smart route does not enable the positive switching preference');
    smart.click(); await pause(40);
    const branch = await mode('自定义分支');
    assert(branch.checked, 'New custom routing does not allow model switching');
    branch.click(); await pause(40);
    assert(!(await mode('智能省钱')).checked, 'Changing modes lost the smart preference');
    await mode('自定义分支'); await click('保存草稿');
    await until(() => calls('preview_plan_editor').length > 0, 'follow-up draft request');
    const editor = calls('preview_plan_editor').at(-1).payload.input.editor;
    assert(editor.smart.reselect_on_user_message === false && editor.branch_routing.reselect_on_user_message === false, 'Unchecked preferences were inverted or reset when saving');
    await until(() => !button('保存草稿').disabled, 'follow-up save finished');
    (await mode('智能省钱')).click(); await pause(40);
    (await mode('自定义分支')).click(); await pause(40); await click('保存草稿');
    await until(() => calls('preview_plan_editor').length === 2, 'enabled follow-up draft request');
    const enabled = calls('preview_plan_editor').at(-1).payload.input.editor;
    assert(enabled.smart.reselect_on_user_message === true && enabled.branch_routing.reselect_on_user_message === true, 'Checked preferences were inverted when saving');
  }),
  scenario('desktop.routing.judgment-settings', ['routing-editor', 'decision-services'], 'Advanced judgment starts collapsed; branch copy/reset stays independent and draft payload retains settings', async () => {
    await fresh(); await routing();
    const followUp = [...document.querySelectorAll('.plan-editor details')].find(item => item.querySelector('summary')?.textContent.includes('追问设置'));
    followUp.querySelector('summary').click(); await pause(40);
    assert(!followUp.querySelector('input').checked, 'Opening a saved false preference applied the new default');
    await click('决策模型');
    const field = id => document.querySelector(`[data-decision-field="${id}"]`);
    const smartDetails = field('smart-simple-threshold').closest('details');
    assert(!smartDetails.open, 'Smart judgment is not collapsed by default');
    smartDetails.querySelector('summary').click(); await pause(40);
    assert(field('smart-simple-threshold').value === '0.8' && field('smart-floor').value === '0.5', 'Default thresholds differ from the published defaults');
    assert(!field('smart-simple').closest('details').open && !field('smart-criterion-0').closest('details').open, 'Prompt details start expanded');
    setInput(field('smart-simple-threshold'), '0.7'); await pause(40);
    await click('保存草稿');
    await until(() => calls('preview_plan_editor').length > 0 && !button('保存草稿').disabled, 'smart draft request');
    const smart = calls('preview_plan_editor').at(-1).payload.input.editor.smart;
    assert(smart.reselect_on_user_message === false, 'Saving unrelated edits changed the saved follow-up preference');
    assert(smart.judgment.degree.simple_threshold_millis === 700 && !('branch_routing' in calls('preview_plan_editor').at(-1).payload.input.editor), 'Smart saving was serialized as task branches');
    all('.mode-card').find(item => item.querySelector('strong')?.textContent === '自定义分支').click();
    await until(() => all('.branch-routing-card').length === 2, 'custom branch editor');
    assert(all('.branch-routing-card').every(card => !card.querySelector('details').open), 'Branch advanced fields start expanded');
    assert(!field('branch-0-simple-threshold'), 'Single-group branch exposes a degree threshold');
    const defaults = field('global-floor').closest('details');
    defaults.querySelector('summary').click(); await pause(40);
    setInput(field('global-floor'), '0.6'); await pause(40);
    const first = all('.branch-routing-card')[0];
    first.querySelector('details > summary').click(); await pause(40);
    assert(field('branch-0-floor').disabled || field('branch-0-floor').closest('fieldset').disabled, 'Following defaults is editable before copying');
    [...first.querySelectorAll('button')].find(item => item.textContent === '单独调整').click(); await pause(40);
    assert(field('branch-0-floor').value === '0.6', 'Copy did not capture current plan defaults');
    setInput(field('branch-0-floor'), '0.4'); await pause(40);
    setInput(field('global-floor'), '0.7'); await pause(40);
    assert(field('branch-0-floor').value === '0.4' && field('branch-1-floor').value === '0.7', 'Independent branch changed with plan defaults');
    await click('保存草稿');
    await until(() => calls('preview_plan_editor').at(-1).payload.input.editor.mode === 'custom_branches', 'custom draft request');
    const saved = calls('preview_plan_editor').at(-1).payload.input.editor;
    assert(saved.branch_routing.branches[0].judgment.competence.floor_millis === 400 && saved.branch_routing.branches[1].judgment == null, 'Whole-setting override identity was lost');
    assert(saved.smart.judgment.degree.simple_threshold_millis === 700, 'Changing modes lost parked smart judgment');
    await until(() => !button('保存草稿').disabled, 'draft action finished');
    await click('恢复计划默认');
    assert(field('branch-0-floor').value === '0.7', 'Restore did not resume following plan defaults');
  }),
  scenario('desktop.models.branch-route-references', ['model-connections', 'routing-editor'], 'Model references include regular and upgrade branch candidates and open the correct published route', async () => {
    await fresh(() => {
      const base = structuredClone(c().desktop.catalog.plans[0]);
      const selection = { binding_id: 'binding/codex/gpt-6-astra' };
      const makePlan = (id, name, upgrade) => ({ ...structuredClone(base), agent_plan_id: id, model_alias: id.replace('/', '-'), desired: { ...base.desired, display_name: name, mode: 'custom_branches', strategy: { mode: 'branches', routing: {
        classifier: { kind: 'decision_service', service: { id: 'decision/fixture', revision: 1, name: 'Fixture extension', connection: { kind: 'custom', endpoint: 'https://example.test/decision', timeout_ms: 10000 } } }, default_branch_id: 'primary', judgment: structuredClone(base.desired.strategy.judgment), reselect_on_user_message: false,
        branches: [{ id: 'economy', name: 'Economy', condition: 'Simple', judgment: null, candidates: upgrade ? [{ binding_id: 'binding/bailian/qwen-coder' }] : [selection], primary_candidates: upgrade ? [selection] : [] }, { id: 'primary', name: 'Primary', condition: 'Complex', judgment: null, candidates: [{ binding_id: 'binding/bailian/qwen-coder' }], primary_candidates: [] }],
      } } } });
      const regular = makePlan('plan/branch-regular', 'Branch regular model use', false);
      const upgrade = makePlan('plan/branch-upgrade', 'Branch upgrade model use', true);
      const disabled = { ...makePlan('plan/disabled', 'Disabled branch model use', false), head: { ...base.head, status: 'disabled' } };
      c().desktop.catalog.plans = [regular, upgrade, disabled];
    });
    await click('模型');
    await until(() => all('.oc-models .master-list button').some(item => item.textContent.includes('GPT-6-Astra')), 'model row');
    all('.oc-models .master-list button').find(item => item.textContent.includes('GPT-6-Astra')).click(); await pause(40);
    const refs = () => all('.oc-models .v3-linked');
    await until(() => text().includes('用于这些路由'), 'model route references');
    assert(refs().some(item => item.textContent.includes('Branch regular model use')), 'Published regular branch model reference is missing');
    assert(refs().some(item => item.textContent.includes('Branch upgrade model use')), 'Published upgrade branch model reference is missing');
    assert(refs().some(item => item.textContent.trim() === 'Disabled branch model use（已停用）'), 'Disabled reference is missing or presented without its stopped state');
    assert(refs().filter(item => item.textContent.includes('Disabled branch model use')).length === 1, 'Disabled reference is duplicated');
    await click('Branch regular model use');
    await until(() => document.querySelector('.plan-identity-fields input')?.value === 'Branch regular model use', 'referenced route opened');
  }),
  scenario('desktop.decisions.hidden-invalid-field', ['decision-services'], 'An invalid collapsed field and its error scroll above the sticky form actions', async () => {
    const saved = { id: 'decision/invalid-field', revision: 1, name: 'Connection with a collapsed timeout', connection: { kind: 'system_one', provider: 'bailian-token-plan', model: 'decision-model-preview', endpoint: 'https://example.test/systemone', timeout_ms: 10000, auth_header: { name: 'Authorization', value_secret_ref: 'protected/r1' } } };
    await fresh(() => { c().handlers.decision_services = () => ({ services: [saved] }); });
    await click('模型'); await click('决策模型'); await click('编辑');
    const form = all('.decision-connection-form')[0];
    const timeout = form.querySelector('[name="timeout"]');
    const details = timeout.closest('details');
    details.querySelector('summary').click(); await pause(40);
    setInput(timeout, '0'); await pause(40);
    details.querySelector('summary').click(); await pause(40);
    const pane = form.closest('.detail-pane'); pane.scrollTop = pane.scrollHeight;
    await click('仅保存');
    await until(() => document.activeElement === timeout && details.open, 'invalid field expanded and focused');
    const field = timeout.closest('.field').getBoundingClientRect();
    const footer = form.querySelector('.decision-form-actions').getBoundingClientRect();
    assert(field.top >= pane.getBoundingClientRect().top && field.bottom <= footer.top, 'Invalid field or error is hidden behind the sticky actions');
    assert(calls('save_decision_service').length === 0, 'Invalid timeout was submitted');
  }),
  scenario('desktop.decisions.save-test-revision', ['decision-services'], 'Save and test uses the saved credential revision; failure repairs and origin changes retain safe state', async () => {
    let services = [];
    await fresh(() => {
      c().handlers.decision_services = () => ({ services: structuredClone(services) });
      c().handlers.save_decision_service = ({ input }) => {
        const saved = structuredClone(input.change.service);
        saved.connection.auth_header.value_secret_ref = `protected/r${saved.revision}`;
        services = [saved]; return { state: 'succeeded' };
      };
      c().handlers.test_classifier_decision = () => ({ outcome: 'failed', failure_code: 'CLASSIFIER_INPUT_REJECTED', duration_millis: 12 });
    });
    await click('模型'); await click('决策模型'); await click('添加决策模型');
    const form = () => all('.decision-connection-form')[0];
    const input = name => form()?.querySelector(`[name="${name}"]`);
    assert(all('.decision-provider img').length >= 2, 'Provider brand logos are missing');
    assert(!input('timeout').closest('details').open && !input('endpoint').closest('details').open, 'Advanced defaults overwhelm initial form');
    setInput(input('credential'), 'synthetic-test-key'); await pause(30);
    form().querySelector('summary').click(); await pause(30);
    assert(input('credential').value === 'synthetic-test-key', 'Disclosure lost the credential');
    await click('保存并测试'); await until(() => text().includes('配置已保存，测试失败'), 'saved failure feedback');
    const saved = calls('save_decision_service').at(-1).payload.input;
    assert(saved.change.expected_revision === 0 && saved.change.service.revision === 1, 'New connection did not use revision transaction');
    assert(saved.secret === 'synthetic-test-key', 'Built-in auth was incorrectly prefixed by UI');
    const tested = calls('test_classifier_decision').at(-1).payload.input.classifier.service;
    assert(tested.revision === 1 && tested.connection.auth_header.value_secret_ref === 'protected/r1', 'Test used the unsaved editor or old credential');
    assert(text().includes('协议兼容性') && !text().includes('CLASSIFIER_INPUT_REJECTED'), 'Failure is not actionable or exposes raw code by default');
    await click('修改连接'); await click('更换');
    setInput(input('credential'), 'replacement-key');
    form().querySelector('summary').click(); await pause(30);
    setInput(input('endpoint'), 'https://other.example.test/systemone'); await pause(30);
    assert(input('credential').value === '' && text().includes('请重新填写认证'), 'Endpoint origin change retained credentials');
    const before = calls('save_decision_service').length;
    await click('仅保存');
    assert(calls('save_decision_service').length === before, 'Missing credential was submitted');
    setInput(input('credential'), 'replacement-key'); await pause(30);
    await click('仅保存'); await until(() => !form(), 'saved edited revision');
    assert(calls('test_classifier_decision').length === 1 && text().includes('本次打开尚未测试'), 'Old test status leaked to new revision or save-only tested');
    c().handlers.test_classifier_decision = () => ({ outcome: 'passed', duration_millis: 20 });
    await click('测试连接'); await until(() => text().includes('连接测试通过'), 'test repaired connection');
    assert(calls('test_classifier_decision').at(-1).payload.input.classifier.service.revision === 2, 'Repair tested a stale revision');
    await click('编辑'); setInput(input('name'), 'Saved while readback fails'); await pause(40);
    c().handlers.decision_services = () => { throw new Error('DAEMON_UNAVAILABLE'); };
    await click('保存并测试'); await until(() => text().includes('配置已保存，但未能核实新版本'), 'acknowledged save readback failure');
    assert(!form() && calls('test_classifier_decision').length === 2, 'Readback failure retained a resubmittable secret or ran a stale test');
    assert(button('测试连接').disabled, 'Readback failure allowed testing an old snapshot');
    c().handlers.decision_services = () => ({ services: structuredClone(services) });
    await click('刷新'); await until(() => text().includes('Saved while readback fails') && !button('测试连接').disabled, 'readback recovery');
  }),
  scenario('desktop.decisions.route-detour', ['decision-services', 'routing-editor'], 'Adding a custom extension preserves the route and selects its exact saved version without publication', async () => {
    let services = [];
    await fresh(() => {
      c().handlers.decision_services = () => ({ services: structuredClone(services) });
      c().handlers.save_decision_service = ({ input }) => {
        const saved = structuredClone(input.change.service);
        if (saved.connection.auth_header) saved.connection.auth_header.value_secret_ref = 'protected/custom';
        services = [saved]; return { state: 'succeeded' };
      };
    });
    await routing();
    const routeName = () => all('.plan-identity-fields input')[0];
    setInput(routeName(), 'Preserved route draft'); await pause(40);
    await click('自定义扩展'); await click('接入自定义扩展');
    assert(!document.querySelector('dialog[open]') && text().includes('路由草稿已保留'), 'Detour tried to discard the route');
    const form = () => all('.decision-connection-form')[0];
    const input = name => form()?.querySelector(`[name="${name}"]`);
    setInput(input('endpoint'), 'https://extension.example.test/decision');
    const auth = form().querySelector('select'); auth.value = 'bearer'; auth.dispatchEvent(new Event('change', { bubbles: true })); await pause(40);
    setInput(input('credential'), 'extension-token');
    setInput(input('name'), 'Review extension'); await pause(40);
    await click('查看接入协议'); await click('完成');
    assert(input('credential').value === 'extension-token', 'Protocol dialog cleared authentication');
    await click('仅保存');
    await until(() => routeName()?.value === 'Preserved route draft', 'return to original draft');
    assert(calls('save_decision_service').at(-1).payload.input.secret === 'Bearer extension-token', 'Custom bearer auth was not serialized correctly');
    assert(all('.decision-selector select')[0]?.value === services[0].id, 'Saved extension was not selected');
    assert(calls('preview_plan_editor').length === 0 && calls('test_classifier_decision').length === 0, 'Connection save published the route or save-only ran a test');
    await click('启发式规则'); await click('自定义扩展');
    assert(all('.decision-selector select')[0]?.value === services[0].id, 'Changing decision method lost the selected extension');
    await click('决策模型');
    assert(all('.decision-selector select')[0].options.length === 1, 'Extension leaked into decision model choices');
    await click('添加决策模型'); await click('返回路由'); await click('放弃修改');
    assert(routeName()?.value === 'Preserved route draft', 'Cancelling detour discarded the route draft');
  }),
  scenario('desktop.decisions.discard-navigation', ['decision-services'], 'Discarding decision edits clears retained editors and credentials across navigation', async () => {
    const saved = { id: 'decision/saved', revision: 1, name: 'Saved decision', connection: {
      kind: 'system_one', provider: 'bailian-token-plan', model: 'decision-model-preview',
      endpoint: 'https://example.test/systemone', timeout_ms: 10000,
      auth_header: { name: 'Authorization', value_secret_ref: 'decision/saved/r1' },
    } };
    await fresh(() => { c().handlers.decision_services = () => ({ services: [saved] }); });
    await click('模型'); await click('决策模型');
    const editor = () => all('.decision-connection-form')[0];
    await until(() => text().includes(saved.name), 'saved decision');
    for (const creating of [false, true]) {
      if (creating) await click('添加决策模型');
      else { await click('编辑'); await click('更换'); }
      setInput(editor().querySelector('[name="name"]'), 'Unsaved decision');
      setInput(editor().querySelector('[name="credential"]'), 'synthetic-discard-value');
      await click('通用模型'); await click('继续编辑');
      assert(editor().querySelector('[name="name"]').value === 'Unsaved decision', 'Keep editing lost the name');
      assert(editor().querySelector('[name="credential"]').value === 'synthetic-discard-value', 'Keep editing lost the credential');
      await click('首页'); await click('放弃修改'); await click('模型');
      await until(() => !editor() && text().includes(saved.name), 'discard restores saved detail');
      await click('编辑'); await click('更换');
      assert(editor().querySelector('[name="credential"]').value === '', 'Discard retained the unsaved credential');
      assert(editor().querySelector('button[type="submit"]').disabled, 'Discard left a dirty editor');
      await click('取消'); await click('首页');
      assert(!document.querySelector('dialog[open]'), 'Discarded editor prompted again');
      await click('模型');
    }
    assert(calls('save_decision_service').length === 0, 'Discard persisted an edit');
  }),
  scenario('desktop.settings.persisted-scale', ['desktop-presentation'], 'Settings apply and persist all supported text scales through the shell', async () => {
    await fresh(); await click('设置');
    for (const [label, value] of [['150%', '1.5'], ['200%', '2'], ['100%', '1']]) {
      await click(label);
      assert(button(label).getAttribute('aria-pressed') === 'true', 'Scale choice did not become selected');
      assert(document.querySelector('.hr-ui').dataset.textScale === value, 'Shell did not apply the selected scale');
      assert(localStorage.getItem('hiroute.text-scale') === value, 'Selected scale was not persisted');
    }
  }),
  scenario('desktop.operation.dismiss-observes', ['operation-feedback'], 'Closing feedback leaves the operation observable and never cancels it', async () => {
    await pending();
    const before = calls('observe_operation').length;
    feedback().querySelector('button[aria-label="关闭"]').click();
    await until(() => !feedback(), 'feedback closed');
    await until(() => calls('observe_operation').length > before, 'observation continues after closing');
    assert(!c().commands.some(({ command }) => /cancel|stop_observ/.test(command)), 'Closing feedback cancelled or stopped the operation');
  }),
  scenario('desktop.operation.identity-retry-converges', ['operation-feedback'], 'A mismatched observation stays neutral and retry converges on the accepted operation', async () => {
    await pending();
    const before = calls('observe_operation').length;
    c().handlers.observe_operation = () => operation('succeeded', 'operation/unrelated');
    await until(() => calls('observe_operation').length > before, 'first mismatched observation');
    assert(!feedback().dataset.observationErrorCode, 'A transient mismatch skipped the recovery grace period');
    await until(() => feedback()?.dataset.operationPhase === 'unverified', 'unconfirmed result becomes retryable', 12000);
    assert(feedback().dataset.observationErrorCode === 'OPERATION_IDENTITY_MISMATCH', 'Mismatch was not diagnosed');
    assert(feedback().querySelector('.oc-spinner') && !feedback().querySelector('.callout.bad'), 'Unknown result was presented as terminal failure');
    c().handlers.observe_operation = () => operation('succeeded');
    c().desktop.pending = null;
    const reads = calls('desktop_snapshot').length;
    await click('重新查询');
    await until(() => feedback()?.dataset.operationPhase === 'succeeded', 'retry observes the accepted identity');
    await until(() => calls('desktop_snapshot').length > reads, 'success refreshes actual state');
    await until(() => !feedback(), 'success feedback dismisses itself', 6500);
  }),
  scenario('desktop.operation.repeated-submission', ['operation-feedback'], 'Repeated accepted and unverified Agent saves each resume operation observation', async () => {
    for (const knownIdentity of [true, false]) {
      await fresh(() => {
        const agent = c().agents.agents.find(item => item.agent_id === 'agent_codex_default');
        agent.configuration_state = 'not_configured';
        agent.settings.current_selection = null;
        agent.settings.state = 'not_configured';
        c().agents.plans.plans = c().agents.plans.plans.filter(plan => plan.head.status === 'enabled').slice(0, 1);
        c().handlers.preview_agent_settings = () => ({ preview: { applicable: true, blockers: [] }, mutation: { state: knownIdentity ? 'applied' : 'response_unknown', operation: knownIdentity ? operation() : null } });
      });
      await click('Agent');
      await until(() => all('.native-list .list-row').some(item => item.textContent.includes('Codex')), 'Codex row');
      all('.native-list .list-row').find(item => item.textContent.includes('Codex')).click();
      await until(() => document.querySelector('[data-agent-facet="model"]'), 'model configuration');
      document.querySelector('[data-agent-facet="model"]').click();
      for (let attempt = 0; attempt < 2; attempt++) {
        if (attempt && knownIdentity) document.querySelector('[data-agent-facet="model"]').click();
        await until(() => document.querySelector('[role="dialog"] button[type="submit"]')?.disabled === false, 'save enabled');
        const before = calls('observe_operation').length;
        document.querySelector('[role="dialog"] button[type="submit"]').click();
        await until(() => calls('preview_agent_settings').length === attempt + 1, 'save submitted');
        await until(() => calls('observe_operation').length > before, `${knownIdentity ? 'accepted' : 'unverified'} save ${attempt + 1} is observed`, 4000);
      }
    }
  }),
  scenario('desktop.routing.qoder-budget-checkpoint', ['routing-editor', 'qoder'], 'An accepted publication that fails its Qoder budget checkpoint explains model reconnection and retains edits', async () => {
    await fresh(() => {
      c().handlers.preview_plan_editor = () => ({ state: 'applied', operation: operation() });
      c().handlers.observe_operation = () => ({ ...operation('rolled_back'), sequence: 2, safe_error_code: 'QODER_MODEL_BUDGET_CONFLICT' });
    });
    await routing();
    const contextMode = document.querySelector('.plan-context-mode');
    assert(contextMode && visible(contextMode), 'Context window mode is not reachable');
    contextMode.value = 'custom';
    contextMode.dispatchEvent(new Event('change', { bubbles: true }));
    await until(() => visible(document.querySelector('.plan-context-window-field')), 'custom context window');
    const contextWindow = document.querySelector('.plan-context-window-field');
    setInput(contextWindow, '32000');
    await until(() => document.querySelector('[data-codex-capability-summary]')?.textContent.includes('32K'), 'model options for the edited context window');
    await click('发布更改');
    await until(() => feedback()?.dataset.operationPhase === 'failed', 'accepted publication checkpoint failure');
    const guidance = feedback().querySelector('[data-error-code="QODER_MODEL_BUDGET_CONFLICT"]');
    assert(guidance?.textContent.includes('停用 Qoder 的模型路由') && guidance.textContent.includes('再发布此计划并重新配置模型路由') && guidance.textContent.includes('任务协作无需停用'), 'Checkpoint failure lacks independent model recovery steps');
    const publication = calls('preview_plan_editor').at(-1)?.payload.input;
    assert(publication?.action === 'publish' && publication.editor.limits.context_window_tokens === 32000, 'The accepted request did not publish the edited budget');
    assert(contextWindow.value === '32000' && !contextWindow.disabled && !button('发布更改').disabled, 'Checkpoint failure discarded or locked the local edit');
    assert(calls('preview_agent_settings').length === 0 && calls('check_agent_live').length === 0, 'Checkpoint failure implicitly changed an Agent connection or called a model');
  }),
  scenario('desktop.home.reactivation-usage', ['home', 'sessions'], 'Returning home refreshes usage alongside recent activity without remounting the app', async () => {
    let input = 2400;
    const usage = () => document.querySelector('.v3-usage-body')?.textContent ?? '';
    await fresh(() => {
      c().handlers.observation_read = payload => {
        const result = c().fixtureResponse('observation_read', payload);
        if (payload.request.intent.view !== 'home_value' || payload.request.intent.query.session_id) return result;
        return { ...result, usage: result.usage.map(metric => metric.metric === 'input' ? { ...metric, known_sum: input } : metric) };
      };
    });
    await until(() => usage().includes('2,400'), 'initial usage');
    await click('会话');
    input = 9322197;
    const before = calls('observation_read').filter(call => call.payload.request.intent.view === 'home_value').length;
    await click('首页');
    await until(() => usage().includes('9,322,197'), 'new usage on ordinary home navigation');
    assert(calls('observation_read').filter(call => call.payload.request.intent.view === 'home_value').length > before, 'Returning home reused the old summary');
    assert(usage().includes('尚未计价') && !usage().includes('2,400'), 'Refresh fabricated a price or retained the old total');
  }),
  scenario('desktop.routing.reactivation-models', ['routing-editor'], 'Returning to an existing route reloads saved model choices and scopes quality to active models', async () => {
    await fresh(); await routing();
    await click('模型表现');
    await until(() => document.querySelector('.quality-model-scope')?.textContent.includes('Qwen'), 'active model scope');
    assert(document.querySelector('.quality-model-scope').textContent.includes('Qwen'), 'Active route models are absent');
    assert(!document.querySelector('.quality-model-filter'), 'Quality asks users to type internal model IDs');
    const before = calls('compute_management_snapshot').length;
    const optionsBefore = calls('plan_editor_options').length;
    await click('首页');
    const revisedName = 'Saved renamed model';
    c().management.sources.find(source => source.source_id === 'source/bailian/coding').models[0].display_name = revisedName;
    c().handlers.plan_editor_options = payload => {
      const options = c().fixtureResponse('plan_editor_options', payload);
      return { ...options, candidates: options.candidates.map(candidate => candidate.binding_id === 'binding/bailian/qwen-coder' ? { ...candidate, display_name: revisedName } : candidate) };
    };
    await click('智能路由');
    await until(() => calls('compute_management_snapshot').length > before && calls('plan_editor_options').length > optionsBefore, 'fresh saved choices after reactivation');
    assert(document.querySelector('.plan-identity-fields input'), 'Returning lost the existing editor');
    await until(() => document.querySelector('.quality-model-scope')?.textContent.includes(revisedName), 'latest saved model name is visible');
  }),
  scenario('desktop.models.documentation-retains-input', ['model-connections'], 'Model readiness stays bounded and a failed documentation link preserves input', async () => {
    await fresh(); await click('模型');
    await until(() => all('button').some(item => item.textContent.includes('GPT-6-Astra')), 'model row');
    all('button').find(item => item.textContent.includes('GPT-6-Astra')).click();
    await until(() => text().includes('接入就绪'), 'readiness status');
    assert(text().includes('不代表上游推理或工具调用已验证'), 'Local readiness claims upstream inference verification');
    await click('添加模型');
    await until(() => all('button').some(item => item.textContent.includes('新建 API 接入')), 'add API entry');
    all('button').find(item => item.textContent.includes('新建 API 接入')).click();
    await until(() => document.querySelector('[role="dialog"] input[type="password"]'), 'API key input');
    const key = document.querySelector('[role="dialog"] input[type="password"]');
    setInput(key, 'synthetic-input-preserved'); await pause(40);
    const link = all('a').find(item => item.textContent.includes('官方说明与获取 Key'));
    assert(link?.target === '_blank' && link.rel.includes('noreferrer') && link.href.startsWith('https://'), 'Web documentation link is unavailable');
    c().handlers.open_external_url = () => { throw new Error('BROWSER_UNAVAILABLE'); };
    link.click(); await until(() => text().includes('当前表单内容已保留'), 'native browser failure feedback');
    assert(calls('open_external_url').at(-1)?.payload.url === link.href, 'Native command did not receive the displayed documentation URL');
    assert(key.value === 'synthetic-input-preserved', 'Opening documentation lost the API key input');
    // Prevent an actual external navigation while observing the normal web default action.
    const internals = window.__TAURI_INTERNALS__;
    let preventedByProduct;
    const observeClick = event => { preventedByProduct = event.defaultPrevented; event.preventDefault(); };
    document.addEventListener('click', observeClick);
    try { delete window.__TAURI_INTERNALS__; link.click(); }
    finally { window.__TAURI_INTERNALS__ = internals; document.removeEventListener('click', observeClick); }
    assert(preventedByProduct === false && calls('open_external_url').length === 1, 'Web preview does not retain ordinary anchor behavior');
  }),
  scenario('desktop.routing.classifier-protocol', ['routing-editor'], 'Classifier choice and protocol copy/save expose visible outcomes', async () => {
    await fresh(); await routing();
    const external = button('决策模型');
    assert(external, 'Decision model choice is missing'); external.click(); await pause(40);
    assert(external.getAttribute('aria-pressed') === 'true', 'Decision choice is not visibly selected');
    const local = button('启发式规则');
    local.click(); await pause(40);
    assert(local.getAttribute('aria-pressed') === 'true' && external.getAttribute('aria-pressed') === 'false', 'Heuristic choice did not replace the decision selection');
    await click('自定义扩展');
    c().handlers.decision_services = () => ({ services: [] });
    await click('接入自定义扩展');
    assert(!document.querySelector('dialog[open]'), 'Connection detour discarded the route');
    await click('查看接入协议');
    await click('复制 curl');
    await until(() => c().clipboard.length === 1, 'curl copied');
    const curl = c().clipboard[0];
    assert(curl.includes('https://classifier.example/v1/decisions'), 'Curl targets the wrong API');
    for (const field of ['decision', 'latest_user', 'visible_conversation', 'history_partial', 'assessment_target']) assert(curl.includes(`"${field}"`), `Curl lacks ${field}`);
    assert(all('.classifier-protocol pre')[1].textContent.includes('ordinal') && !all('.classifier-protocol pre')[1].textContent.includes('assessment'), 'First decision fabricated an assessment');
    await click('自定义分支 · 分类并评分');
    assert(all('.classifier-protocol pre')[0].textContent.includes('"from": 0'), 'Assessment request has no prior stage');
    assert(all('.classifier-protocol pre')[1].textContent.includes('review') && text().includes('这里评分属于上一写稿阶段'), 'Assessment targets the new branch');
    let finish;
    c().handlers.save_classifier_openapi = () => new Promise(resolve => { finish = resolve; });
    await click('保存 OpenAPI');
    assert(button('保存 OpenAPI').disabled && text().includes('请选择保存位置'), 'Native save did not show a pending state');
    finish('cancelled'); await until(() => text().includes('已取消保存'), 'cancelled save outcome');
    c().handlers.save_classifier_openapi = () => 'saved';
    await click('保存 OpenAPI'); await until(() => text().includes('已保存：hiroute-decision.openapi.json'), 'saved file outcome');
    assert(calls('save_classifier_openapi').length === 2, 'Save did not use exactly one native command per click');
  }),
];
export const productShellScenarioCatalog = scenarioCatalog(scenarios);
export const runProductShellScenarios = (selection = { requiredIds: productShellRequired }) => runScenarios(scenarios, selection);
