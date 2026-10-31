import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

export async function runConsoleScenarios(client, baseUrl, output) {
  const results = [];
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const settleVisual = () => evaluate(client, `Promise.all(document.getAnimations()
    .filter(animation => animation.effect?.getTiming().iterations !== Infinity)
    .map(animation => animation.finished.catch(() => undefined)))
    .then(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))`);
  const open = async (state, language = 'zh', theme = 'light', width = 1200, height = 813, extra = {}) => {
    const url = new URL('console-entry.html', baseUrl);
    for (const [key, value] of Object.entries({ case: state, language, theme, ...extra })) url.searchParams.set(key, value);
    await navigate(client, url.href, { width, height });
    await waitFor(client, 'Boolean(window.consoleComponent)');
    await waitFor(client, state === 'ready' || state === 'launch' ? 'Boolean(document.querySelector(".sidebar"))' : 'Boolean(document.querySelector(".console-entry h1")) && !document.querySelector(".console-entry [role=status]")');
    if (state === 'ready' || state === 'launch') {
      await waitFor(client, 'document.querySelector(".console-transport-bar")?.getBoundingClientRect().height > 0');
      assert(await evaluate(client, '!window.consoleComponent.errors.length'), 'Incomplete console component fixture: ' + await evaluate(client, 'JSON.stringify(window.consoleComponent.errors)'));
    }
    if (state === 'login' || state === 'expired') {
      const guidance = language === 'zh' ? '获取一次性登录链接' : 'single-use sign-in link';
      assert(await evaluate(client, `document.body.innerText.includes(${JSON.stringify(guidance)})`), 'Console entry still describes a local-only launch link');
    }
    if (state === 'rate-limited' || state === 'session-limit') {
      await evaluate(client, 'Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value").set.call(document.querySelector("input"), "c".repeat(64)); document.querySelector("input").dispatchEvent(new Event("input",{bubbles:true}))');
      await evaluate(client, 'document.querySelector("form").requestSubmit()');
      await waitFor(client, 'Boolean(document.querySelector("[role=alert]")) && !document.querySelector("input").disabled');
      assert(await evaluate(client, '!document.querySelector("input").value && !sessionStorage.getItem("hiroute.console.session")'), 'Rejected sign-in retained its input or granted a session');
      const hint = state === 'rate-limited' ? (language === 'zh' ? '一分钟' : 'one minute') : (language === 'zh' ? '退出其他设备' : 'Sign out');
      assert(await evaluate(client, `document.body.innerText.includes(${JSON.stringify(hint)})`), 'Sign-in recovery hint is missing');
    }
  };
  const check = async (id, run) => { try { await run(); results.push({ id, state: 'green' }); } catch (error) { results.push({ id, state: 'red', error: String(error) }); } };
  await check('console.session.launch', async () => {
    await open('launch', 'zh', 'light', 1200, 813, { startup: 'held' });
    try {
      await waitFor(client, 'window.consoleComponent.desktopPending() > 0');
      const startup = await evaluate(client, 'window.consoleComponent.invoke("startup_status", {})');
      assert(startup.state === 'ready', 'Authenticated console invented a startup failure before its first snapshot: ' + JSON.stringify(startup));
      assert(await evaluate(client, '!document.querySelector(".startup-recovery")'), 'Authenticated console displays a false startup failure while its first snapshot is pending');
    } finally { await evaluate(client, 'window.consoleComponent.releaseDesktop()'); }
    await waitFor(client, 'Boolean(document.querySelector(".sidebar-health .good"))');
    assert(await evaluate(client, 'Boolean(document.querySelector(".console-transport-bar.plaintext"))'), 'Network launch login lost its persistent plaintext warning');
    assert(await evaluate(client, 'location.pathname === "/" && !location.search && window.consoleComponent.requests.filter(r => r.path.endsWith("/launch")).length === 1 && window.consoleComponent.requests.every(r => !r.url.includes("code="))'), 'Launch code was not removed before exchange');
    assert(await evaluate(client, 'sessionStorage.getItem("hiroute.console.session")?.length === 64 && !localStorage.getItem("hiroute.console.session") && !document.cookie'), 'Session escaped sessionStorage');
    assert(await evaluate(client, 'window.consoleComponent.requests.filter(r => !r.path.endsWith("/launch")).every(r => r.headers.authorization?.startsWith("Bearer ") && r.credentials === "omit" && /^[a-f0-9]{32}$/.test(r.headers["x-hiroute-context"]))'), 'Console request authority differs from the Bearer/context contract');
  });
  await check('console.session.token', async () => {
    await open('login');
    await waitFor(client, 'document.querySelector(".console-entry-brand img")?.complete ?? true');
    const brand = await evaluate(client, `(() => {
      const mark = document.querySelector('.console-entry-brand .brand-mark');
      const image = mark?.querySelector('img');
      const box = image?.getBoundingClientRect();
      return { naturalWidth: image?.naturalWidth ?? 0, width: box?.width ?? 0, height: box?.height ?? 0, text: mark?.textContent.trim() };
    })()`);
    assert(brand.naturalWidth > 0 && Math.abs(brand.width - 34) < 1 && Math.abs(brand.height - 34) < 1 && !brand.text,
      'Console entry does not render the HiRoute mark: ' + JSON.stringify(brand));
    assert(await evaluate(client, 'document.querySelector("[role=note]")?.textContent.includes("明文")'), 'Plaintext token warning is missing');
    await evaluate(client, 'Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value").set.call(document.querySelector("input"), "c".repeat(64)); document.querySelector("input").dispatchEvent(new Event("input",{bubbles:true}))');
    await evaluate(client, 'document.querySelector("form").requestSubmit()');
    await waitFor(client, 'Boolean(document.querySelector(".sidebar"))');
    assert(await evaluate(client, 'window.consoleComponent.requests.filter(r => r.path.endsWith("/token")).length === 1 && !document.querySelector("input[name=access-token]")'), 'Token exchange did not clear the form');
  });
  await check('console.session.expired', async () => {
    await open('expired');
    assert(await evaluate(client, '!sessionStorage.getItem("hiroute.console.session") && document.body.innerText.includes("会话已结束")'), '401 did not discard the bearer and render expiry');
    const startupError = await evaluate(client, 'window.consoleComponent.invoke("startup_status", {}).then(() => null, error => error.code)');
    assert(startupError === 'CONSOLE_AUTH_REQUIRED', 'Expired console session produced a startup verdict without authentication');
  });
  await check('console.host-capabilities', async () => {
    await open('ready');
    await evaluate(client, '[...document.querySelectorAll("button")].find(b => b.textContent.trim() === "设置").click()');
    await waitFor(client, 'Boolean(document.querySelector(".settings-group"))');
    assert(await evaluate(client, '!document.body.innerText.includes("安装命令行") && !document.body.innerText.includes("检查更新")'), 'Browser exposes host-only CLI or update actions');
    assert(await evaluate(client, 'Boolean(document.querySelector(".console-transport-bar.plaintext"))'), 'Persistent plaintext banner is missing');
  });
  for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const [width, height] of [[1200, 813], [755, 600]]) {
    for (const state of ['login', 'expired', 'unavailable', 'assets', 'ready', 'rate-limited', 'session-limit']) {
      await open(state, language, theme, width, height);
      const geometry = await evaluate(client, `(() => {
        const element = document.querySelector('.console-entry-card') ?? document.querySelector('.console-transport-bar');
        const box = element.getBoundingClientRect();
        const page = document.querySelector('.main .page');
        return { overflow: element.scrollWidth > element.clientWidth + 1, left: box.left, right: box.right, viewport: innerWidth, theme: document.querySelector('.hr-ui').dataset.theme,
          bannerHeight: document.querySelector('.console-transport-bar')?.getBoundingClientRect().height,
          pageTop: page?.getBoundingClientRect().top, pageBottom: page?.getBoundingClientRect().bottom };
      })()`);
      assert(!geometry.overflow && geometry.left >= 0 && geometry.right <= geometry.viewport + 1 && geometry.theme === theme, `Console layout failed: ${state}/${language}/${theme}/${width}: ${JSON.stringify(geometry)}`);
      if (state === 'ready') assert(geometry.bannerHeight < 110 && geometry.pageTop < 200 && geometry.pageBottom <= height + 1, 'Console banner displaced the product page: ' + JSON.stringify(geometry));
      await settleVisual();
      const screenshot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
      const name = `console-${state}-${language}-${theme}-${width}x${height}.png`;
      await writeFile(path.join(output, name), Buffer.from(screenshot.data, 'base64'));
      results.push({ id: `console.layout.${state}.${language}.${theme}.${width}`, state: 'green', screenshot: name, geometry });
      if (state === 'ready') {
        const click = async (label, selector = 'button') => { await waitFor(client, `[...document.querySelectorAll(${JSON.stringify(selector)})].some(e => e.textContent.includes(${JSON.stringify(label)}))`); await evaluate(client, `[...document.querySelectorAll(${JSON.stringify(selector)})].find(e => e.textContent.includes(${JSON.stringify(label)})).click()`); };
        await click(language === 'zh' ? '模型' : 'Models', '.sidebar button');
        await click(language === 'zh' ? '添加模型' : 'Add models');
        await click(language === 'zh' ? '新建 API 接入' : 'New API connection', '.add-choice');
        await click(language === 'zh' ? '百炼' : 'Alibaba Cloud Model Studio', '.connection-provider-card');
        await click('Coding Plan', '.connection-product-card');
        await waitFor(client, 'Boolean(document.querySelector(".console-secret-warning"))');
        assert(await evaluate(client, 'document.querySelector("input[type=password]").getBoundingClientRect().width > 100 && document.querySelector(".console-secret-warning").getBoundingClientRect().width > 100'), 'Plaintext secret form is unreadable');
        await settleVisual();
        const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
        const secretName = `console-secret-${language}-${theme}-${width}x${height}.png`;
        await writeFile(path.join(output, secretName), Buffer.from(shot.data, 'base64'));
        results.push({ id: `console.secret.${language}.${theme}.${width}`, state: 'green', screenshot: secretName });
      }
    }
  }
  await writeFile(path.join(output, 'console-report.json'), JSON.stringify({ evidence: 'Production React/style components with synthetic HTTP. Native installation and real daemon acceptance are separate.', results }, null, 2) + '\n');
  for (const result of results) process.stdout.write(`${result.state}: ${result.id}${result.error ? ': ' + result.error : ''}\n`);
  if (results.some(result => result.state !== 'green')) throw new Error('Console scenarios failed');
}
