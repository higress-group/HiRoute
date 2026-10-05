import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import { BrandIcon, Disclosure, UiIcon } from '../ui';
import type { PlanOperation } from '../plan-editor-persistence';
import {
  recommendedWorkerDependencies,
  selectedWorkerDependencies,
  workerDependencyRevision,
  workerDependencySelectionMatches,
  workerDependencyComponents,
  workerDependencySelectionComplete,
  workerDependencySelectionState,
  workerEnvelopeData,
  type WorkerDependenciesView,
  type WorkerDependencyComponent,
  type WorkerDependencySelectionRequest,
  type WorkerHarness,
  type WorkerMachineEnvelope,
} from './worker-dependencies-state';

type Confirmation = {
  schema: string;
  confirmation_id: string;
  selection: WorkerDependencySelectionRequest;
  expires_at_ms: number;
};

type ExecutorAvailability = {
  executors: {
    harness: WorkerHarness;
    state: 'ready' | 'unavailable' | 'unknown';
    reason?: string | null;
  }[];
};

function failureCode(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'code' in error && typeof error.code === 'string') {
    return error.code.toUpperCase();
  }
  return 'WORKER_DEPENDENCIES_UNAVAILABLE';
}

export function WorkerDependencies({
  harness,
  language,
  active,
  onOperation,
}: {
  harness: WorkerHarness;
  language: 'zh' | 'en';
  active: boolean;
  onOperation?: (operation: PlanOperation | null) => void;
}) {
  const zh = language === 'zh';
  const text = (cn: string, en: string) => zh ? cn : en;
  const name = { codex_cli: 'Codex CLI', claude_code: 'Claude Code', qoder_cli: 'Qoder CLI', pi: 'Pi' }[harness];
  const components = workerDependencyComponents(harness);
  const [view, setView] = useState<WorkerDependenciesView | null>(null);
  const [selection, setSelection] = useState<WorkerDependencySelectionRequest | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [manualDirty, setManualDirty] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const generation = useRef(0);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  useEffect(() => {
    if (!active) return;
    let disposed = false;
    void invoke<ExecutorAvailability>('worker_executor_availability')
      .then(value => {
        if (disposed) return;
        const executor = value.executors.find(item => item.harness === harness);
        if (executor && executor.reason !== 'installation_not_configured') void discover(true);
      })
      .catch(() => {});
    return () => { disposed = true; };
  }, [active, harness]);

  function errorMessage(cause: unknown) {
    const code = failureCode(cause);
    if (code.includes('MISSING')) return text('所选安装缺少文件，请重新检测或修正路径。', 'The selected installation is missing files. Detect again or correct the paths.');
    if (code.includes('INVALID')) return harness === 'qoder_cli'
      ? text('所选路径不是可用的 Qoder CLI，请重新检测或修正路径。', 'The selected path is not a usable Qoder CLI. Detect again or correct the path.')
      : text('所选路径不是可用的程序或连接组件。', 'A selected path is not a usable executable or adapter.');
    if (code.includes('EXPIRED')) return text('本次保存已过期，请重新尝试。', 'This save expired. Try again.');
    if (code.includes('STALE') || code.includes('CONFLICT')) return text('安装选择已经变化，请重新检测后确认。', 'The installation selection changed. Detect and confirm again.');
    if (code.includes('AUTHORITY') || code.includes('DENIED')) return text('当前连接只能查看，不能修改本机安装选择。', 'The current connection can view but cannot change the local installation selection.');
    return text('未能完成检测或保存，原有安装选择没有被替换。', 'Detection or saving could not complete. The existing installation selection was not replaced.');
  }

  async function discover(automatic = false) {
    if (!active || refreshing) return;
    const requestGeneration = ++generation.current;
    setRefreshing(true);
    setError('');
    if (!automatic) setNotice('');
    try {
      const envelope = await invoke<WorkerMachineEnvelope<WorkerDependenciesView>>('worker_dependencies_discover', {
        input: { harness },
      });
      const found = workerEnvelopeData(envelope);
      if (!mounted.current || requestGeneration !== generation.current) return;
      setView(found);
      setSelection(current => {
        const recommendation = recommendedWorkerDependencies(found, harness);
        if (!manualDirty) return recommendation;
        return current ? { ...current, expected_selection_revision: workerDependencyRevision(found, harness) } : recommendation;
      });
    } catch (cause) {
      if (mounted.current && requestGeneration === generation.current) setError(errorMessage(cause));
    } finally {
      if (mounted.current && requestGeneration === generation.current) setRefreshing(false);
    }
  }

  function edit(patch: Partial<WorkerDependencySelectionRequest>) {
    setSelection(current => ({
      harness,
      cli_path: '',
      ...(harness === 'qoder_cli' ? {} : { ...(harness === 'pi' ? {} : { adapter_path: '' }), node_path: null }),
      expected_selection_revision: view ? workerDependencyRevision(view, harness) : 0,
      ...current,
      ...patch,
    }));
    setManualDirty(true);
    setError('');
    setNotice('');
  }

  async function saveSelection() {
    if (!selection || !workerDependencySelectionComplete(selection) || submitting) return;
    setSubmitting(true);
    setError('');
    setNotice('');
    try {
      const input = harness === 'qoder_cli'
        ? { harness, cli_path: selection.cli_path, expected_selection_revision: selection.expected_selection_revision }
        : harness === 'pi' ? { harness, cli_path: selection.cli_path, node_path: selection.node_path, expected_selection_revision: selection.expected_selection_revision } : selection;
      const prepared = await invoke<Confirmation>('worker_dependencies_select_prepare', { input });
      if (!mounted.current) {
        await invoke('worker_dependencies_select_cancel', {
          input: { confirmation_id: prepared.confirmation_id },
        }).catch(() => {});
        return;
      }
      const envelope = await invoke<WorkerMachineEnvelope<WorkerDependenciesView>>('worker_dependencies_select_confirm', {
        input: { confirmation_id: prepared.confirmation_id },
      });
      const selected = workerEnvelopeData(envelope);
      const operation = envelope.operation;
      onOperation?.(operation ? { ...operation, safe_error_code: null } : null);
      if (!mounted.current) return;
      setView(selected);
      setSelection(recommendedWorkerDependencies(selected, harness));
      setManualDirty(false);
      setNotice(text('安装已配置；本实例中使用该执行 Agent 的计划会共享此选择。', 'Installation configured. Plans using this execution agent share the selection in this instance.'));
    } catch (cause) {
      if (mounted.current) {
        setError(errorMessage(cause));
      }
    } finally {
      if (mounted.current) setSubmitting(false);
    }
  }

  async function copyCommand(command: string) {
    setError('');
    try {
      await navigator.clipboard.writeText(command);
      setNotice(text('安装命令已复制。请在终端执行后返回重新检测。', 'Installation command copied. Run it in a terminal, then return and detect again.'));
    } catch {
      setError(text('无法写入剪贴板，请手动复制命令。', 'Could not write to the clipboard. Copy the command manually.'));
    }
  }

  async function openInstallationGuide(event: React.MouseEvent<HTMLAnchorElement>) {
    if (!("__TAURI_INTERNALS__" in window)) return;
    event.preventDefault();
    try {
      await invoke('open_external_url', { url: event.currentTarget.href });
    } catch {
      setError(text('无法打开官方安装说明，请手动打开链接；已填写的路径会保留。', 'Could not open the official installation guide. Open the link manually; your paths are retained.'));
    }
  }

  const state = view ? workerDependencySelectionState(view, harness) : null;
  const savedSelection = view ? selectedWorkerDependencies(view, harness) : null;
  const replacingSelection = Boolean(view && selection && savedSelection
    && !workerDependencySelectionMatches(view, selection));
  const componentName = (component: WorkerDependencyComponent) => ({
    cli: name,
    adapter: text('连接组件', 'ACP adapter'),
    node: 'Node.js',
  })[component];
  const candidates = (component: WorkerDependencyComponent) => (view?.candidates ?? [])
    .filter(candidate => candidate.harness === harness && candidate.component === component);
  const inputValue = (component: WorkerDependencyComponent) => component === 'cli'
    ? selection?.cli_path ?? ''
    : component === 'adapter' ? selection?.adapter_path ?? '' : selection?.node_path ?? '';
  const selectCandidate = (component: WorkerDependencyComponent, path: string) => edit(component === 'cli'
    ? { cli_path: path }
    : component === 'adapter' ? { adapter_path: path } : { node_path: path || null });

  return <div className="worker-dependencies">
    <div className="editor-section-heading">
      <div>
        <h3>{text(`${name} 本机安装`, `${name} local installation`)}</h3>
        <p>{text('这里只选择已有安装，不会自动运行安装命令。缺少依赖也不影响保存路由。', 'This selects an existing installation and never runs install commands. Missing dependencies do not block saving the route.')}</p>
      </div>
      {state === 'configured' && <span className="badge good no-dot">{text('已配置', 'Configured')}</span>}
    </div>

    {harness === 'qoder_cli' && <p className="field-help" data-qoder-installation-guide>
      {text('选择已安装的 Qoder CLI（qoder 或 qodercli）。需要登录时，请在终端正常启动该 CLI 并完成登录，再返回使用。', 'Select your installed Qoder CLI (qoder or qodercli). When sign-in is needed, start that CLI normally in a terminal and sign in, then return here.')} {' '}
      <a href="https://docs.qoder.com/cli/installation" target="_blank" rel="noreferrer" onClick={event => void openInstallationGuide(event)}>{text('官方安装与升级说明', 'Official installation and upgrade guide')}</a>
    </p>}
    {harness === 'pi' && <p className="field-help" data-pi-installation-guide>{text('使用官方 npm 安装的 Pi 和 Node.js。检测安装位置后，保存时会检查所需能力；也可查看安装命令。', 'Use the official npm installation of Pi and Node.js. Detect installation paths; saving checks the required capabilities. Installation commands are also available.')}</p>}
    {!view && <div className="option-panel"><div className="option-row"><div><strong>{text('检测本机安装', 'Detect local installation')}</strong><span>{harness === 'qoder_cli'
      ? text('查找当前环境中的 Qoder CLI。', 'Find Qoder CLI in the current environment.')
      : harness === 'pi' ? text('查找 Pi 和 Node.js。', 'Find Pi and Node.js.') : text(`查找 ${name}、连接组件和必要的 Node.js。`, `Find ${name}, its adapter, and Node.js when required.`)}</span></div><button className="btn btn-primary" type="button" disabled={refreshing} onClick={() => void discover()}>{refreshing ? text('正在检测…', 'Detecting…') : text('一键检测', 'Detect')}</button></div></div>}

    {view && <div className={`callout ${state === 'configured' ? 'good' : state === 'found' ? '' : 'warn'}`}>
      <UiIcon name={state === 'incomplete' ? 'warning' : 'check'} />
      <div>
        <strong>{state === 'configured'
          ? text('安装已配置', 'Installation configured')
          : state === 'found' ? text(`已找到 ${name} 及所需组件`, `Found ${name} and required components`)
            : text('还缺少部分组件', 'Some components are still missing')}</strong>
        <p>{state === 'configured'
          ? text('此选择由本实例中使用同一执行 Agent 的计划共享，只影响后续任务。', 'This selection is shared by plans using the same execution agent in this instance and affects future tasks only.')
          : state === 'found' ? text('点击“使用此安装”后保存为本实例的安装选择。', 'Select “Use this installation” to save it for this instance.')
            : text('按下面的受控提示安装后重新检测，或在高级设置中填写路径。', 'Use the verified hints below, then detect again, or enter paths in Advanced settings.')}</p>
      </div>
    </div>}

    {view && state === 'incomplete' && view.install_hints.filter(hint => hint.harness === harness && components.includes(hint.component)).map((hint, index) => <div className="option-row" key={`${hint.component}:${hint.reason_code}:${index}`}><div><strong>{text(`需要 ${componentName(hint.component)}`, `${componentName(hint.component)} required`)}</strong><span>{hint.reason_code === 'worker.dependencies.scan_incomplete' ? text('本次检测未能完整扫描，请重新检测。', 'The scan was incomplete. Detect again.') : text('当前没有找到可用路径。', 'No usable path was found.')}</span></div>{hint.command && <button className="btn" type="button" onClick={() => void copyCommand(hint.command!)}>{text('复制安装命令', 'Copy install command')}</button>}</div>)}

    {view && <div className="editor-actions worker-dependency-actions">
      <button className="btn" type="button" disabled={refreshing || submitting} onClick={() => void discover()}>{refreshing ? text('正在检测…', 'Detecting…') : text('重新检测', 'Detect again')}</button>
      {(!savedSelection || replacingSelection) && <button className="btn btn-primary" type="button" disabled={!workerDependencySelectionComplete(selection) || refreshing || submitting} onClick={() => void saveSelection()}>{submitting ? text('正在保存…', 'Saving…') : savedSelection ? text('更换安装', 'Replace installation') : text('使用此安装', 'Use this installation')}</button>}
      <button className="btn btn-quiet" type="button" onClick={() => setAdvanced(value => !value)}>{advanced ? text('收起高级设置', 'Hide Advanced') : savedSelection ? text('更换或高级设置', 'Replace or Advanced') : text('高级设置', 'Advanced')}</button>
    </div>}

    {advanced && view && <Disclosure className="native-details" defaultOpen label={text('安装路径与候选', 'Installation paths and candidates')} language={language}>{components.map(component => <div className="field" key={component}>
      <label className="field-label" htmlFor={`worker-${harness}-${component}`}>{componentName(component)} {component === 'node' && harness !== 'pi' && <span className="muted">{text('按连接组件需要选填', 'Optional when the adapter does not require it')}</span>}</label>
      <input id={`worker-${harness}-${component}`} className="input" value={inputValue(component)} placeholder={text('输入绝对路径', 'Enter an absolute path')} onChange={event => selectCandidate(component, event.target.value)} />
      {!!candidates(component).length && <div className="native-list">{candidates(component).map(candidate => <button className={`list-row${inputValue(component) === candidate.path ? ' active' : ''}`} type="button" key={`${component}:${candidate.path}`} onClick={() => selectCandidate(component, candidate.path)}><span className="row-main"><span className="row-title">{candidate.path}</span><span className="row-meta">{candidate.source} · {candidate.state}</span></span>{inputValue(component) === candidate.path && <UiIcon name="check" />}</button>)}</div>}
    </div>)}</Disclosure>}

    {notice && <div className="callout" role="status"><UiIcon name="info" /><span>{notice}</span></div>}
    {error && <div className="callout bad" role="alert"><UiIcon name="warning" /><span>{error}</span></div>}
  </div>;
}
