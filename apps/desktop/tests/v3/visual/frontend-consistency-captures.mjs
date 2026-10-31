import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

const shellDriver = `async ({language, state, baseline}) => {
  const c = window.productShell, zh = language === 'zh';
  const t = (a,b) => zh ? a : b;
  const pause = () => new Promise(resolve => setTimeout(resolve, 25));
  const visible = node => Boolean(node?.getClientRects().length);
  const all = selector => [...document.querySelectorAll(selector)].filter(visible);
  const until = async (read, label) => { for (let i=0;i<300;i++) { if (read()) return; await pause(); } throw Error('Capture: '+label); };
  const button = label => all('button').find(node => node.textContent.trim() === label);
  const click = async label => { await until(()=>button(label),label); button(label).click(); await pause(); };
  if (state.startsWith('settings-')) {
    await click(t('设置','Settings'));
    await until(()=>document.querySelector('.settings-wrap'),'Settings');
    const area = state.slice('settings-'.length);
    if (!baseline) {
      await click(({general:t('通用','General'),runtime:t('服务与接入','Service & access'),records:t('记录与诊断','Records & diagnostics')})[area]);
    } else if (area !== 'general') {
      const heading = area === 'runtime' ? 'Worker' : t('本地记录','Local records');
      const group = all('.settings-group').find(node=>node.querySelector('h2')?.textContent===heading);
      group.scrollIntoView({block:'start'});
    }
    await until(()=>document.querySelector('input[aria-label="'+t('并发任务上限','Concurrent task limit')+'"]')?.value==='10','Settings readback');
    return '.settings-wrap';
  }
  if (state === 'agents') {
    await click('Agent');
    await until(()=>all('[data-agent-facet="model"]').length||all('.native-list .list-row').some(node=>node.textContent.includes('Codex')),'Agent list or selected detail');
    const row=all('.native-list .list-row').find(node=>node.textContent.includes('Codex'));
    if (row) row.click();
    await until(()=>all('[data-agent-facet="model"]').length===1,'Agent detail');
    return '.agents-page .detail-pane';
  }
  await click(t('模型','Models'));
  await click(t('决策模型','Decision models'));
  await until(()=>button(t('编辑','Edit'))&&!button(t('编辑','Edit')).disabled,'decision detail');
  await click(t('编辑','Edit'));
  await until(()=>document.querySelector('.decision-connection-form'),'decision editor');
  return '.decision-connection-form';
}`;

const seed = `async ({language, theme, state}) => {
  const c = window.productShell;
  c.reset();
  localStorage.setItem('hiroute.language', language); localStorage.setItem('hiroute.theme', theme);
  (await import('/frontend-consistency-fixtures.mjs')).consistencySettingsReads(c);
  const agent = c.agents.agents.find(item=>item.agent_id==='agent_codex_default');
  if (agent?.settings?.collaboration) agent.settings.collaboration.restore_point_ref='restore/codex-task';
  const id='source/openai/decisions';
  const source=c.decisionSource(id,{display_name:'Team OpenAI'});
  const service={schema:'hiroute.decision-service/v2',id:'decision/review',revision:1,name:language==='zh'?'团队决策':'Team decisions',
    connection:state==='decision-extension'?{kind:'custom',endpoint:'https://decisions.example.test/route',timeout_ms:10000,auth_header:null}
    :{kind:'native',provider:'openai',wire_protocol:'openai_decisions',model:'gpt-6-luna',endpoint:'https://api.openai.test/v1/decisions',timeout_ms:10000,
      authentication:{kind:'shared_source',source_id:id,source_lineage_digest:source.lineage_digest}}};
  c.handlers.decision_services_v2=()=>({services:[c.decisionView(service,[source])]});
  c.handlers.decision_connection_sources=()=>({sources:[source]});
  c.remount();
}`;

export async function frontendConsistencyCaptures(client, baseUrl, output, { baseline = false } = {}) {
  const results = [];
  const states = ['agents', 'connection', 'decision-native', 'decision-extension', 'settings-general', 'settings-runtime', 'settings-records'];
  for (const language of ['zh','en']) for (const theme of ['light','dark']) for (const viewport of [{width:1200,height:813},{width:755,height:600}]) for (const state of states) {
    const url = new URL(state==='connection'?`connection-flow.html?kind=custom&lang=${language}&theme=${theme}`:'product-shell.html',baseUrl);
    url.searchParams.set('comparison',crypto.randomUUID());
    await navigate(client,url.href,viewport);
    await waitFor(client,`location.href===${JSON.stringify(url.href)} && Boolean(${state==='connection'?'window.connectionFlowTrace':'window.productShell'})`);
    let selector;
    if (state==='connection') {
      await waitFor(client,'Boolean(document.querySelector("#mc-connection-form input[type=password]") && !document.querySelector(".oc-spinner"))');
      selector='[role="dialog"]';
    } else {
      await evaluate(client,`(${seed})(${JSON.stringify({language,theme,state})})`);
      await waitFor(client,`Boolean(document.querySelector('.sidebar-health .status-dot.good') && document.querySelector('.hr-ui')?.lang===${JSON.stringify(language === 'zh' ? 'zh-CN' : language)})`);
      selector=await evaluate(client,`(${shellDriver})(${JSON.stringify({language,state,baseline})})`);
    }
    await evaluate(client,'document.fonts.ready');
    await evaluate(client,'Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>undefined)))');
    const geometry=await evaluate(client,`(() => {
      const root=document.querySelector(${JSON.stringify(selector)});
      const box=node=>{const b=node.getBoundingClientRect();return {left:b.left,right:b.right,top:b.top,bottom:b.bottom,width:b.width,height:b.height};};
      const result={viewport:{width:innerWidth,height:innerHeight},root:box(root),pageOverflow:document.documentElement.scrollWidth>innerWidth+1,overflow:root.scrollWidth>root.clientWidth+1};
      if (${JSON.stringify(state)}==='connection') {
        const fields=[...document.querySelectorAll('#mc-connection-form .connection-fields > .field')].filter(n=>n.getClientRects().length);
        const base=fields.find(n=>n.querySelector('.field-label')?.textContent==='Base URL');
        const protocol=fields.find(n=>['协议','Protocol'].includes(n.querySelector('.field-label')?.textContent));
        const prefill=[...document.querySelectorAll('summary')].find(n=>/使用接入资料预填|Use connection prefill/.test(n.textContent));
        result.fieldGap=box(fields[1]).top-box(fields[0]).bottom;
        result.requiredBeforeOptional=box(protocol).top<box(prefill).top&&box(base).top<box(prefill).top;
        result.protocol=box(protocol); result.body=box(root.querySelector('.modal-body')); result.footer=box(root.querySelector('.modal-foot'));
      }
      if (${JSON.stringify(state)}==='agents') {
        result.actions=[...root.querySelectorAll('[data-agent-facet]')].filter(n=>n.getClientRects().length).map(action=>({facet:action.dataset.agentFacet,role:action.getAttribute('role'),
          buttons:[...action.closest('.detail-section').querySelectorAll('.detail-section-head button')].map(n=>({text:n.textContent.trim(),...box(n)}))}));
        result.visibleTaskDisable=[...root.querySelectorAll('button')].some(n=>n.getClientRects().length&&n.getAttribute('data-agent-disable')==='collaboration');
      }
      if (${JSON.stringify(state)}.startsWith('decision-')) result.footerButtons=[...root.querySelectorAll('.decision-form-actions button')].map(n=>({text:n.textContent.trim(),primary:n.classList.contains('btn-primary'),...box(n)}));
      if (${JSON.stringify(state)}.startsWith('settings-')) {
        result.visiblePanels=[...document.querySelectorAll('[data-settings-panel]')].filter(n=>n.getClientRects().length).map(n=>n.dataset.settingsPanel);
        result.content={client:root.closest('.page-content').clientHeight,scroll:root.closest('.page-content').scrollHeight};
        result.controls=[...root.querySelectorAll('button,input,select')].filter(n=>n.getClientRects().length).map(n=>({label:n.getAttribute('aria-label')||n.textContent.trim(),...box(n)}));
      }
      return result;
    })()`);
    if (!baseline) {
      if (geometry.pageOverflow||geometry.overflow) throw Error(`${state}: horizontal overflow ${JSON.stringify(geometry)}`);
      if (state==='connection' && (geometry.fieldGap>20||!geometry.requiredBeforeOptional
        ||(viewport.width===1200&&geometry.protocol.bottom>geometry.body.bottom))) throw Error(`Connection density: ${JSON.stringify(geometry)}`);
      if (state.startsWith('settings-') && (geometry.visiblePanels.length!==1||geometry.controls.some(b=>b.left<0||b.right>viewport.width+1))) throw Error(`Settings layout: ${JSON.stringify(geometry)}`);
      if (state.startsWith('decision-')) {
        const buttons=geometry.footerButtons;
        if (!/取消|Cancel/.test(buttons[0]?.text)||!buttons.at(-1)?.primary||buttons.some(b=>b.top<0||b.bottom>viewport.height+1||b.right>viewport.width+1)) throw Error(`Editor actions: ${JSON.stringify(geometry)}`);
      }
    }
    const id=`consistency-${state}-${language}-${theme}-${viewport.width}`;
    const screenshot=`${id}.png`;
    const shot=await client.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
    await writeFile(path.join(output,screenshot),Buffer.from(shot.data,'base64'));
    results.push({id,state:'green',baseline,screen:state,language,theme,viewport,geometry,screenshot});
  }
  return results;
}
