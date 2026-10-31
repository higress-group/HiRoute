import { consistencySettingsReads } from './frontend-consistency-fixtures.mjs';

export async function loseInteractionAuthority(c) {
  c.desktop.native_connection = 'recovery_required'; c.desktop.trusted_authority = false;
  c.handlers.observation_read = () => { throw { source: 'native', code: 'TRUSTED_AUTHORITY_UNAVAILABLE' }; };
  await c.callHost('observation_read', {}).catch(() => {});
}

// Synthetic replies at the host boundary. Screens and all navigation remain production.
export async function seedInteractionAudit(c, { language = 'zh', theme = 'light', state = '' } = {}) {
  c.reset();
  localStorage.setItem('hiroute.language', language);
  localStorage.setItem('hiroute.theme', theme);
  consistencySettingsReads(c);
  const sourceId = c.management.sources.find(source => source.provenance === 'registered').source_id;
  const source = c.decisionSource(sourceId, { display_name: 'Team API' });
  const service = { schema: 'hiroute.decision-service/v2', id: 'decision/review', revision: 1,
    name: language === 'zh' ? '团队决策' : 'Team decisions',
    connection: state.includes('extension') ? { kind: 'custom', endpoint: 'https://decisions.example.test/route', timeout_ms: 10000, auth_header: null }
      : { kind: 'native', provider: 'openai', wire_protocol: 'openai_decisions', model: 'gpt-6-luna',
        endpoint: 'https://api.openai.test/v1/decisions', timeout_ms: 10000,
        authentication: { kind: 'shared_source', source_id: sourceId, source_lineage_digest: source.lineage_digest } } };
  c.handlers.decision_services_v2 = () => ({ services: [c.decisionView(service, [source])] });
  c.handlers.decision_connection_sources = () => ({ sources: [source] });
  const task = { task_id: 'task/review', title: language === 'zh' ? '检查路由边界' : 'Review routing boundaries',
    brief: language === 'zh' ? '检查路由配置，整理发现和测试结果。' : 'Review routing configuration and report findings and tests.',
    content_availability: 'available', plan_id: c.desktop.catalog.plans[0].agent_plan_id, plan_revision: 1,
    latest_admission_sequence: 1, latest_run_id: 'run/review', resumable_until_ms: null,
    session_ids: [], session_links_complete: true, created_at_ms: 1793505600000,
    run: { task_id: 'task/review', run_id: 'run/review', state: 'running', state_revision: 1,
      cleanup: 'pending', result_available: false, admission_sequence: 1, accepted_at_ms: 1793505600000,
      accepted_time_state: 'recorded', executor: { harness: 'codex_cli', display_name: 'Codex CLI', basis: 'frozen_plan' } } };
  const envelope = data => ({ status: 'succeeded', data, next_actions: [], error: null });
  c.handlers.worker_task_list = () => envelope({ schema: 'hiroute.worker-task-list/v1', tasks: [task], next_cursor: null });
  c.handlers.worker_task_status = () => envelope({ schema: 'hiroute.worker-task-status/v1', task });
  if (state === 'settings-error') c.handlers.cli_entry_status = () => { throw { code: 'CLI_ENTRY_UNAVAILABLE' }; };
  if (state === 'home-empty') {
    c.management.sources = []; c.desktop.catalog = { plans: [], drafts: [] };
    c.handlers.compute_management_snapshot = () => c.management;
    c.handlers.observation_read = payload => payload.request?.intent?.view === 'sessions'
      ? { sessions: [], next_cursor: null } : c.fixtureResponse('observation_read', payload);
  }
  c.remount();
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
}

export async function openInteractionAudit(state, language = 'zh') {
  const zh = language === 'zh', t = (cn, en) => zh ? cn : en;
  const visible = node => Boolean(node?.getClientRects().length);
  const all = selector => [...document.querySelectorAll(selector)].filter(visible);
  const pause = () => new Promise(resolve => setTimeout(resolve, 25));
  const until = async (read, label) => { for (let i = 0; i < 300; i++) { if (read()) return; await pause(); } throw Error('Audit: ' + label); };
  const button = copy => all('button,summary').find(node => node.getAttribute('aria-label') === copy
    || (node.querySelector('.disclosure-label')?.textContent.trim() ?? node.textContent.trim()) === copy
    || (node.classList.contains('add-choice') && node.querySelector('strong')?.textContent.trim() === copy));
  const click = async copy => { await until(() => button(copy) && !button(copy).disabled, copy); button(copy).click(); await pause(); };
  if (state.startsWith('home')) return '.home-wrap';
  if (state.startsWith('settings')) {
    await click(t('设置', 'Settings'));
    await click(state === 'settings-records' ? t('记录与诊断', 'Records & diagnostics') : state === 'settings-general' ? t('通用', 'General') : t('服务与接入', 'Service & access'));
    await until(() => document.querySelector('input[aria-label="' + t('并发任务上限', 'Concurrent task limit') + '"]')?.value === '10', 'Settings replies');
    if (state === 'settings-error') await until(() => document.body.textContent.includes('CLI_ENTRY_UNAVAILABLE'), 'CLI read error');
    return '.settings-wrap';
  }
  if (state.startsWith('routing')) {
    await click(t('智能路由', 'Smart routing'));
    await until(() => all('.routing-page .master-list .list-row').length, 'routes');
    const row = all('.routing-page .master-list .list-row')[0];
    if (!all('.plan-editor').length) { row.click(); await pause(); }
    await until(() => all('.plan-editor').length, 'routing editor');
    if (state === 'routing-new') await click(t('新建智能路由', 'New smart routing'));
    if (state === 'routing-quality') await click(t('模型表现', 'Model performance'));
    if (state === 'routing-mapping') await click(t('映射 Agent', 'Map an Agent'));
    if (state === 'routing-worker') {
      const heading = all('h3').find(node => node.textContent === t('任务委派', 'Task delegation'));
      if (heading) heading.scrollIntoView({ block: 'start' });
    }
    if (state === 'routing-readonly') {
      await loseInteractionAuthority(window.productShell);
      await until(()=>all('.routing-page input:disabled').length,'read-only route fields');
    }
    return state === 'routing-mapping' ? '[role="dialog"]' : '.routing-page';
  }
  if (state.startsWith('agent') || state.startsWith('task')) {
    await click('Agent');
    await until(() => all('.agents-page .master-list .list-row').length || all('[data-agent-facet="model"]').length, 'Agents');
    if (state.startsWith('task')) {
      await click(t('任务记录', 'Task history'));
      await until(() => all('.task-workspace .list-row').length, 'task list');
      all('.task-workspace .list-row')[0].click();
      await until(() => all('.task-detail').length, 'task detail');
      if (state === 'task-cancel') await click(t('取消任务', 'Cancel task'));
      return state === 'task-cancel' ? '[role="dialog"]' : '.task-workspace';
    }
    const row = all('.agents-page .master-list .list-row').find(node => node.textContent.includes('Codex'));
    if (!all('[data-agent-facet="model"]').length) { row.click(); await pause(); }
    await until(() => all('[data-agent-facet="model"]').length, 'Agent detail');
    if (state === 'agent-editor') { all('[data-agent-facet="model"]')[0].click(); await until(() => all('[role="dialog"]').length, 'Agent editor'); }
    if (state === 'agent-token') {
      await click(t('连接详情与恢复', 'Connection details and recovery'));
      await click(t('修改令牌', 'Change token'));
      await until(() => all('.agent-token-form').length, 'token draft');
      all('.agent-token-form')[0].scrollIntoView({ block: 'center' });
    }
    return state === 'agent-editor' ? '[role="dialog"]' : '.agents-page';
  }
  if (state.startsWith('sessions')) {
    await click(t('会话', 'Sessions'));
    await until(() => all('.session-list .session-item').length || all('.session-item').length || all('.transcript-head').length, 'sessions');
    const row = all('.session-item')[0];
    if (!all('.transcript-head').length && row) row.click();
    await until(() => all('.transcript-head').length, 'transcript');
    if (state === 'sessions-facts') await click(t('运行记录', 'Run record'));
    if (state === 'sessions-cleanup') await click(t('清理此会话', 'Clear this session'));
    if (state === 'sessions-quality') await click(t('本会话的模型表现', 'Model performance in this session'));
    return state === 'sessions-cleanup' ? '[role="dialog"]' : '.sessions-page';
  }
  await click(t('模型', 'Models'));
  if (state.startsWith('decision')) {
    await click(t('决策模型', 'Decision models'));
    await until(() => button(t('编辑', 'Edit')), 'decision detail');
    if (state.includes('editor') || state.includes('readonly')) {
      await click(t('编辑', 'Edit'));
      await until(() => all('.decision-connection-form').length, 'decision editor');
      if (state.includes('readonly')) {
        const c = window.productShell;
        await loseInteractionAuthority(c);
        await until(() => all('.decision-connection-form fieldset:disabled').length, 'read-only fields');
      }
    }
    if (state === 'decision-remove') await click(t('移除模型', 'Remove model'));
    return state === 'decision-remove' ? '[role="dialog"]' : '.decision-models-page';
  }
  await until(() => all('.models-feature .master-list .list-row').length || all('.models-feature .detail-hero').length, 'models');
  if (state === 'models-add') { await click(t('添加模型', 'Add models')); return '[role="dialog"]'; }
  if (state === 'models-directory') {
    await click(t('添加模型', 'Add models')); await click(t('新建 API 接入', 'New API connection'));
    await until(() => all('.connection-provider-card').length, 'connection directory'); return '[role="dialog"]';
  }
  const row = all('.models-feature .master-list .list-row').find(node => node.textContent.includes('Qwen'));
  if (row) row.click();
  await until(() => all('.models-feature .detail-hero').length, 'model detail');
  if (state === 'models-credentials' || state === 'models-credentials-discard') await click(t('管理凭据', 'Manage credentials'));
  if (state === 'models-credentials-discard') {
    await click(t('移除', 'Remove'));
    await click(t('取消', 'Cancel'));
    await until(()=>all('dialog[open]').length || !all('[role="dialog"]').length,'credential Cancel result');
  }
  if (state === 'models-remove') await click(t('移除模型', 'Remove model'));
  if (state === 'models-network') {
    await until(()=>all('.native-network .disclosure-label strong').length,'network settings');
    all('.native-network .disclosure-label strong')[0].closest('button,summary').click();
  }
  return state === 'models-remove' ? '[role="dialog"]' : '.models-feature';
}
