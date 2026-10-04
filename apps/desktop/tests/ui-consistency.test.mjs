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

// Interactive behavior is covered by stable IDs in v3/browser/*-scenarios.mjs.
// Keep this file for stylesheet conventions, native capabilities and byte contracts.
test('page layouts do not subtract fixed header heights from scaled content', () => {
  assert.doesNotMatch(read('src/occami/styles/pages.css'), /height:\s*calc\(100%\s*-\s*(?:72|104|112)px\)/);
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
});

test('the routing connection name is direct and documents its stable Agent identity', () => {
  const editor = read('src/plan-editor.tsx');
  assert.match(editor, /接入模型名/);
  assert.match(editor, /在 Agent 中使用此名称调用这条智能路由。创建后保持稳定。/);
  assert.doesNotMatch(editor, /Agent 调用名称|稳定模型名/);
});

test('selected classifier choices retain a distinct border and left-aligned copy', () => {
  const pages = read('src/occami/styles/pages.css');
  assert.match(pages, /button\.option-row\[aria-pressed='true'\]\s*\{[^}]*border-color:\s*var\(--accent\)/);
  assert.match(pages, /\.option-row > div\s*\{[^}]*text-align:\s*left/);
});

test('the native classifier download embeds the public OpenAPI contract byte for byte', () => {
  const contractPath = join(desktop, '..', '..', 'decision-extensions', 'api', 'decision.openapi.json');
  const openapi = JSON.parse(readFileSync(contractPath, 'utf8'));
  assert.equal(openapi.openapi, '3.1.0');
  assert.equal(openapi.info.title, 'HiRoute Decision API');
  assert.deepEqual(Object.keys(openapi.paths), ['/v1/decisions']);
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

test('the native documentation browser command is explicitly permitted', () => {
  const capability = JSON.parse(read('src-tauri/capabilities/main.json'));
  assert.ok(capability.permissions.includes('allow-open-external-url'));
});

test('native command authority agrees across handlers, manifest and main-window permissions', () => {
  const commandList = (source, pattern, itemPattern, label) => {
    const blocks = [...source.matchAll(pattern)];
    assert.equal(blocks.length, 1, `${label}: expected one command registry`);
    const items = blocks[0][1].split(',').map(item => item.trim()).filter(Boolean);
    const commands = items.map(item => {
      const match = item.match(itemPattern);
      assert.ok(match, `${label}: unrecognized registry item ${item}`);
      return match[1];
    });
    assert.ok(commands.length > 0, `${label}: command registry must not be empty`);
    assert.equal(new Set(commands).size, commands.length, `${label}: duplicate command`);
    return commands.sort();
  };
  const handlers = commandList(
    read('src-tauri/src/bridge.rs'), /tauri::generate_handler!\[([\s\S]*?)\]/g,
    /^([a-z][a-z0-9_]*)$/, 'Tauri handlers',
  );
  const manifest = commandList(
    read('src-tauri/build.rs'), /AppManifest::new\(\)\.commands\(&\[([\s\S]*?)\]\)/g,
    /^"([a-z][a-z0-9_]*)"$/, 'AppManifest',
  );
  assert.deepEqual(manifest, handlers, 'every bundled command needs a generated ACL manifest entry');
  const capability = JSON.parse(read('src-tauri/capabilities/main.json'));
  const permissions = capability.permissions.filter(permission => permission.startsWith('allow-')).sort();
  assert.deepEqual(permissions, handlers.map(command => `allow-${command.replaceAll('_', '-')}`).sort(),
    'the main window must explicitly permit each bundled product command');
  const generatedDirectory = 'src-tauri/permissions/autogenerated';
  assert.deepEqual(readdirSync(join(desktop, generatedDirectory)).filter(name => name.endsWith('.toml')).sort(),
    handlers.map(command => `${command}.toml`).sort(), 'generated permissions must cover the same commands');
  for (const command of handlers) {
    const permission = read(`${generatedDirectory}/${command}.toml`);
    const name = command.replaceAll('_', '-');
    assert.ok(permission.includes(`identifier = "allow-${name}"`), `${command}: missing allow permission`);
    assert.ok(permission.includes(`commands.allow = ["${command}"]`), `${command}: wrong allowed command`);
    assert.ok(permission.includes(`identifier = "deny-${name}"`), `${command}: missing deny permission`);
    assert.ok(permission.includes(`commands.deny = ["${command}"]`), `${command}: wrong denied command`);
  }
});
