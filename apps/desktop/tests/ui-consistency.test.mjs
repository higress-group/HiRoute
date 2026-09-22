import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const desktop = join(dirname(fileURLToPath(import.meta.url)), '..');
const read = relative => readFileSync(join(desktop, relative), 'utf8');

test('all fixed product font declarations participate in text scaling', () => {
  const styleDirectory = join(desktop, 'src', 'occami');
  const paths = [];
  const visit = directory => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) visit(path);
      else if (entry.name.endsWith('.css')) paths.push(path);
    }
  };
  visit(styleDirectory);
  paths.push(join(desktop, 'src', 'features', 'model-reference', 'model-reference.css'));
  for (const path of paths) {
    const css = readFileSync(path, 'utf8');
    for (const declaration of css.matchAll(/font-size\s*:\s*([^;]+);/g)) {
      if (/\d(?:\.\d+)?px/.test(declaration[1])) {
        assert.match(declaration[1], /--hr-text-scale/, `${path}: ${declaration[0]}`);
      }
    }
    for (const declaration of css.matchAll(/font\s*:\s*([^;]+);/g)) {
      if (/\d(?:\.\d+)?px/.test(declaration[1])) {
        assert.match(declaration[1], /--hr-text-scale/, `${path}: ${declaration[0]}`);
      }
    }
  }
});

test('the product settings expose all three persisted text scales', () => {
  const settings = read('src/product/SettingsPage.tsx');
  assert.match(settings, /\(\[1, 1\.5, 2\] as const\)/);
  assert.match(settings, /onTextScaleChange\(scale\)/);
  const app = read('src/product/DesktopApp.tsx');
  assert.match(app, /textScale=\{preferences\.textScale\}/);
  assert.match(app, /onTextScaleChange=\{preferences\.setTextScale\}/);
  const pages = read('src/occami/styles/pages.css');
  assert.doesNotMatch(pages, /height:\s*calc\(100%\s*-\s*(?:72|104|112)px\)/);
});

test('the shared sidebar keeps every navigation action reachable at high text scale', () => {
  const shell = read('src/occami/styles/shell.css');
  const sidebar = shell.slice(shell.indexOf('.sidebar {'), shell.indexOf('.brand {'));
  assert.match(sidebar, /overflow-y:\s*auto/);
  for (const selector of ['.brand {', '.nav {', '.sidebar-foot {']) {
    const start = shell.indexOf(selector);
    const block = shell.slice(start, shell.indexOf('}', start));
    assert.match(block, /flex-shrink:\s*0/, `${selector} must scroll instead of shrinking out of reach`);
  }
});

test('product collapsibles use the shared accessible Disclosure', () => {
  const sourceRoot = join(desktop, 'src');
  const offenders = [];
  const visit = directory => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) visit(path);
      else if (/\.tsx$/.test(entry.name) && entry.name !== 'Disclosure.tsx' && readFileSync(path, 'utf8').includes('<details')) offenders.push(path);
    }
  };
  visit(sourceRoot);
  assert.deepEqual(offenders, []);
  const disclosure = read('src/ui/Disclosure.tsx');
  assert.match(disclosure, /<summary aria-expanded=\{open\}>/);
  assert.match(disclosure, /disclosureActionLabel\(open, language\)/);
  assert.match(disclosure, /<div className="disclosure-body">\{children\}<\/div>/);
});

test('the routing connection name is direct and documents its stable Agent identity', () => {
  const editor = read('src/plan-editor.tsx');
  assert.match(editor, /接入模型名/);
  assert.match(editor, /在 Agent 中使用此名称调用这条智能路由。创建后保持稳定。/);
  assert.doesNotMatch(editor, /Agent 调用名称|稳定模型名/);
});

test('classifier choices expose an unmistakable selected state and left-aligned copy', () => {
  const editor = read('src/plan-editor.tsx');
  assert.match(editor, /className="option-row classifier-choice"/);
  assert.match(editor, /如何判断任务复杂度/);
  assert.match(editor, /内置规则/);
  assert.match(editor, /自定义分类服务/);
  assert.match(editor, /Jev、LLM 或其他自定义策略/);
  assert.doesNotMatch(editor, /BERT/);
  assert.match(editor, /classifier\.kind === 'local_rules' \? text\('已选择', 'Selected'\)/);
  assert.match(editor, /classifier\.kind === 'rest' \? text\('已选择', 'Selected'\)/);
  const pages = read('src/occami/styles/pages.css');
  assert.match(pages, /button\.option-row\[aria-pressed='true'\]\s*\{[^}]*border-color:\s*var\(--accent\)/);
  assert.match(pages, /\.option-row > div\s*\{[^}]*text-align:\s*left/);
});

test('the custom classifier protocol dialog exposes curl and the native OpenAPI save action', () => {
  const dialog = read('src/features/ClassifierProtocolDialog.tsx');
  assert.match(dialog, /查看|接入协议/);
  assert.match(dialog, /复制 curl/);
  assert.match(dialog, /保存 OpenAPI/);
  assert.match(dialog, /save_classifier_openapi/);
  assert.match(dialog, /请选择保存位置/);
  assert.match(dialog, /已取消保存/);
  assert.doesNotMatch(dialog, /createObjectURL|document\.createElement\('a'\)/);
  assert.match(dialog, /hiroute-decision\.openapi\.json/);
  for (const field of ['branches', 'latest_user', 'visible_conversation', 'history_partial', 'assessment_from']) {
    assert.match(dialog, new RegExp(`"${field}"`));
  }

  const contractPath = join(desktop, '..', '..', 'decision-extensions', 'api', 'decision.openapi.json');
  const openapi = JSON.parse(readFileSync(contractPath, 'utf8'));
  assert.equal(openapi.openapi, '3.1.0');
  assert.equal(openapi.info.title, 'HiRoute Decision API');
  assert.deepEqual(Object.keys(openapi.paths), ['/v1/decisions']);
  assert.match(dialog, /https:\/\/classifier\.example\/v1\/decisions/);
  const native = read('src-tauri/src/bridge/classifier.rs');
  assert.match(native, /hiroute-decision\.openapi\.json/);
  const embedded = native.match(/include_str!\("([^"]+)"\)/)[1];
  assert.equal(readFileSync(join(desktop, 'src-tauri/src/bridge', embedded), 'utf8'), readFileSync(contractPath, 'utf8'));
  assert.deepEqual(openapi.components.schemas.ClassifierRequest.required, [
    'branches',
    'latest_user',
    'visible_conversation',
    'history_partial',
    'assessment_from',
  ]);
  assert.equal(openapi.components.schemas.ClassifierRequest.additionalProperties, false);
  assert.deepEqual(openapi.components.schemas.ClassifierResponse.required, ['branch_id']);
  assert.equal(openapi.components.schemas.ClassifierResponse.additionalProperties, false);
  assert.equal(openapi.components.schemas.CompetenceAssessment.properties.score.minimum, 0);
  assert.equal(openapi.components.schemas.CompetenceAssessment.properties.score.maximum, 1);
});

test('the classifier diagnostic warning is fully localized', () => {
  const editor = read('src/plan-editor.tsx');
  assert.match(editor, /要求选择省钱分支的固定合成问题/);
  assert.match(editor, /asks for the economy branch/);
  assert.doesNotMatch(editor, /This is a synthetic connectivity test/);
});

test('plan performance automatically scopes model presentation without a typed model id', () => {
  const editor = read('src/plan-editor.tsx');
  const quality = read('src/features/PlanQuality.tsx');
  assert.match(editor, /currentModels=\{activePlanQualityModels\(plan, options\?\.candidates \?\? \[\]\)\}/);
  assert.match(quality, /当前生效版本模型/);
  assert.doesNotMatch(quality, /quality-model-filter|实际模型 ID|Executed model ID/);
});

test('native window configuration keeps macOS controls and shared localization', () => {
  const config = JSON.parse(read('src-tauri/tauri.conf.json'));
  const main = config.app.windows.find(window => window.label === 'main');
  assert.equal(main.decorations, true);
  assert.equal(main.titleBarStyle, 'Overlay');
  assert.equal(main.hiddenTitle, true);
  const capability = JSON.parse(read('src-tauri/capabilities/main.json'));
  assert.ok(capability.permissions.includes('core:window:allow-set-theme'));
  assert.ok(capability.permissions.includes('core:window:allow-start-dragging'));
  assert.ok(capability.permissions.includes('allow-perform-titlebar-double-click'));
  const app = read('src/product/DesktopApp.tsx');
  assert.match(app, /data-tauri-drag-region="deep"/);
  assert.match(app, /perform_titlebar_double_click/);
  const plist = read('src-tauri/Info.plist');
  assert.match(plist, /CFBundleLocalizations/);
  assert.match(plist, /<string>en<\/string>/);
  assert.match(plist, /<string>zh-Hans<\/string>/);
});

test('external links use the bounded native browser command and retain web preview behavior', () => {
  const form = read('src/features/model-connections/ModelConnectionForm.tsx');
  assert.match(form, /"__TAURI_INTERNALS__" in window/);
  assert.match(form, /invoke\('open_external_url', \{ url \}\)/);
  assert.match(form, /target="_blank"/);
  assert.match(form, /当前表单内容已保留/);
  const capability = JSON.parse(read('src-tauri/capabilities/main.json'));
  assert.ok(capability.permissions.includes('allow-open-external-url'));
});

test('Codex routing form only shows configured native overrides and explains the chosen default', () => {
  const agents = read('src/agents.tsx');
  assert.match(agents, /const fixedModelRows = editorValues\.fixedModels\.map/);
  assert.match(agents, /fixedModelRows\.length > 0 && <fieldset data-agent-fixed-models>/);
  assert.doesNotMatch(agents, /hiddenCatalogModelCount/);
  assert.match(agents, /defaultChoice\.kind === 'plan'[\s\S]*Codex 将默认使用所选智能路由/);
  assert.match(agents, /保留当前默认模型名称；请求仍经过 HiRoute/);
  assert.match(agents, /data-agent-service-responsibility/);
  assert.match(agents, /保存不会设置登录项或保证 HiRoute 服务以后持续在线/);
});

test('Agent home does not offer duplicate paid per-client live probes', () => {
  const agents = read('src/agents.tsx');
  assert.doesNotMatch(agents, /check_agent_live|data-agent-surface-status|真实验证（最多/);
  assert.match(agents, /data-codex-shared-scope/);
});

test('Claude routing promises the normal CLI entry and does not ask for a special launcher', () => {
  const agents = read('src/agents.tsx');
  assert.match(agents, /直接启动 claude，使用 Opus、Sonnet、Haiku 原生预设/);
  assert.match(agents, /账号 Default 仍未知；保存不会验证它/);
  assert.match(agents, /路由已配置 · 调用未验证/);
  assert.doesNotMatch(agents, /hiroute agent launch --agent claude-code/);
});

test('model readiness copy does not claim that an upstream call was verified', () => {
  const models = read('src/features/models/Models.tsx');
  assert.match(models, /available: \['good', zh \? '接入就绪' : 'Connection ready'\]/);
  assert.match(models, /不代表上游推理或工具调用已验证/);
});

test('returning to the routing editor refreshes saved-model choices', () => {
  const editor = read('src/plan-editor.tsx');
  assert.match(editor, /if \(!active\) return;[\s\S]*'compute_management_snapshot'[\s\S]*\}, \[active, language\]\)/);
  assert.match(editor, /'plan_editor_options'[\s\S]*\}, \[active, editor, language, optionsRetry\]\)/);
});

test('closing operation feedback does not stop or cancel observation', () => {
  const app = read('src/product/DesktopApp.tsx');
  const dismiss = app.slice(app.indexOf('function dismissOperation'), app.indexOf('function retryOperationObservation'));
  assert.match(dismiss, /setDismissedOperationId/);
  assert.doesNotMatch(dismiss, /setObserving|stop_observing|cancel/);
});

test('terminal success converges while an unknown result keeps an explicit retry', () => {
  const app = read('src/product/DesktopApp.tsx');
  assert.match(app, /operationView\.phase !== 'succeeded'/);
  assert.match(app, /setTimeout\(\(\) => setDismissedOperationId\(feedbackId\), 4500\)/);
  assert.match(app, /operationView\.phase === 'unverified'[\s\S]*retryOperationObservation/);
  assert.match(app, /\['pending', 'unverified'\]\.includes\(operationView\.phase\) && <span className="oc-spinner"/);
  assert.match(app, /OBSERVATION_GRACE_MS/);
  assert.match(app, /noteUnconfirmed\('OPERATION_IDENTITY_MISMATCH'\)/);
  assert.doesNotMatch(app, /setOperationError\(pendingOperationId \? '' : OPERATION_RESULT_UNVERIFIED\)/);
  assert.match(app, /currentPendingHint\(snapshotPending, supersededPendingKey\.current\)/);
  assert.match(app, /data-observation-error-code=\{presentedOperationError \|\| undefined\}/);
  assert.doesNotMatch(app, /presentedOperationError && <div className="callout bad"/);
});

test('every accepted or unverified operation restarts the polling effect', () => {
  const app = read('src/product/DesktopApp.tsx');
  const accept = app.slice(app.indexOf('function acceptOperation'), app.indexOf('function dismissOperation'));
  assert.match(accept, /setPollingRevision\(current => current \+ 1\)/);
  assert.match(accept, /function acceptUnverifiedOperation/);
  const pollingEffectEnd = app.indexOf('useEffect(() => {\n    const pendingOperationId');
  const pollingEffect = app.slice(app.indexOf('const generation = ++pollingGeneration.current'), pollingEffectEnd);
  assert.match(pollingEffect, /pollingRevision/);
});
