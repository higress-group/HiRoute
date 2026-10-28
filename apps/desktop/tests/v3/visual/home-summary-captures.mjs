import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

// Production shell/styles with controlled IPC: no native or real-service claim.
export async function homeSummaryCaptures(client, baseUrl, output) {
  const results = [];
  for (const [language, theme] of [['zh', 'dark'], ['en', 'light']]) {
    for (const viewport of [{ width: 1200, height: 813 }, { width: 755, height: 600 }]) {
      await navigate(client, new URL('product-shell.html', baseUrl).href, viewport);
      await waitFor(client, 'Boolean(window.productShell)');
      await evaluate(client, `(() => {
        const c = window.productShell;
        localStorage.setItem('hiroute.language', ${JSON.stringify(language)});
        localStorage.setItem('hiroute.theme', ${JSON.stringify(theme)});
        c.handlers.observation_read = payload => payload.request?.intent?.view === 'sessions'
          ? { sessions: [], next_cursor: null } : c.fixtureResponse('observation_read', payload);
        c.handlers.agent_model_associations = () => ({ complete: false, agents: [] });
        c.remount();
      })()`);
      const savedText = language === 'zh' ? '已保存设置，使用前检查' : 'Settings saved; checked before use';
      await waitFor(client, `document.body.innerText.includes(${JSON.stringify(savedText)})`);
      await evaluate(client, `Array.from(document.querySelectorAll('.v3-step')).find(el => el.innerText.includes(${JSON.stringify(savedText)})).scrollIntoView({ block:'center' })`);
      const homeGeometry = await evaluate(client, `(() => {
        const box = Array.from(document.querySelectorAll('.v3-step')).find(el => el.innerText.includes(${JSON.stringify(savedText)}));
        const b = box.getBoundingClientRect();
        return { overflow: box.scrollWidth > box.clientWidth + 1, bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1,
          visible: b.left >= 0 && b.right <= innerWidth + 1 && b.top >= 0 && b.bottom <= innerHeight + 1, text: box.innerText };
      })()`);
      if (homeGeometry.overflow || homeGeometry.bodyOverflow || !homeGeometry.visible) throw Error(`Home summary geometry: ${JSON.stringify({ language, theme, viewport, homeGeometry })}`);
      let id = `home-summary-${language}-${theme}-${viewport.width}`;
      let shot = await client.send('Page.captureScreenshot', { format:'png', captureBeyondViewport:false });
      await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
      results.push({ id, state:'green', language, theme, viewport, geometry:homeGeometry, screenshot:`${id}.png` });

      const routeLabel = language === 'zh' ? '智能路由' : 'Smart routing';
      await evaluate(client, `document.querySelector(${JSON.stringify(`button[aria-label="${routeLabel}"]`)}).click()`);
      await waitFor(client, 'Boolean(document.querySelector(".master-list .list-row"))');
      await evaluate(client, `document.querySelector('.master-list .list-row').click()`);
      const partialText = language === 'zh' ? '部分 Agent 关联暂不可用' : 'Some agent associations are unavailable';
      await waitFor(client, `document.body.innerText.includes(${JSON.stringify(partialText)})`);
      await evaluate(client, `Array.from(document.querySelectorAll('.editor-section')).find(el => el.innerText.includes(${JSON.stringify(partialText)})).scrollIntoView({block:'center'})`);
      const routeGeometry = await evaluate(client, `(() => {
        const box = Array.from(document.querySelectorAll('.editor-section')).find(el => el.innerText.includes(${JSON.stringify(partialText)}));
        const b = box.getBoundingClientRect();
        return { overflow: box.scrollWidth > box.clientWidth + 1, bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1,
          visible: b.left >= 0 && b.right <= innerWidth + 1 && b.top >= 0 && b.bottom <= innerHeight + 1, text: box.innerText };
      })()`);
      if (routeGeometry.overflow || routeGeometry.bodyOverflow || !routeGeometry.visible) throw Error(`Route association geometry: ${JSON.stringify({ language, theme, viewport, routeGeometry })}`);
      id = `route-association-${language}-${theme}-${viewport.width}`;
      shot = await client.send('Page.captureScreenshot', { format:'png', captureBeyondViewport:false });
      await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
      results.push({ id, state:'green', language, theme, viewport, geometry:routeGeometry, screenshot:`${id}.png` });

      await navigate(client, new URL('product-shell.html', baseUrl).href, viewport);
      await waitFor(client, 'Boolean(window.productShell)');
      await evaluate(client, `(() => {
        const c = window.productShell;
        localStorage.setItem('hiroute.language', ${JSON.stringify(language)});
        localStorage.setItem('hiroute.theme', ${JSON.stringify(theme)});
        c.management.sources = [];
        c.desktop.catalog.plans = [];
        c.desktop.catalog.drafts = [];
        c.agents.agents = [{ agent_id:'agent_codex_default', context_id:'context/unready',
          configuration_state:'registered', settings:null, status_error:null }];
        c.handlers.observation_read = payload => payload.request?.intent?.view === 'sessions'
          ? { sessions: [], next_cursor: null } : c.fixtureResponse('observation_read', payload);
        c.remount();
      })()`);
      const unavailableText = language === 'zh' ? '暂时无法确认当前配置' : 'Current setup cannot be confirmed';
      await waitFor(client, `document.body.innerText.includes(${JSON.stringify(unavailableText)})`);
      const unavailableGeometry = await evaluate(client, `(() => {
        const box = document.querySelector('.v3-ready'); const b = box.getBoundingClientRect();
        return { overflow: box.scrollWidth > box.clientWidth + 1, bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1,
          visible: b.left >= 0 && b.right <= innerWidth + 1 && b.top >= 0 && b.bottom <= innerHeight + 1,
          retryVisible: Boolean(box.querySelector('button')?.getClientRects().length), text: box.innerText };
      })()`);
      if (unavailableGeometry.overflow || unavailableGeometry.bodyOverflow || !unavailableGeometry.visible || !unavailableGeometry.retryVisible) throw Error(`Unavailable Home geometry: ${JSON.stringify({ language, theme, viewport, unavailableGeometry })}`);
      id = `home-unavailable-${language}-${theme}-${viewport.width}`;
      shot = await client.send('Page.captureScreenshot', { format:'png', captureBeyondViewport:false });
      await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
      results.push({ id, state:'green', language, theme, viewport, geometry:unavailableGeometry, screenshot:`${id}.png` });
    }
  }
  return results;
}
