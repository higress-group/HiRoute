import assert from 'node:assert/strict';
import { evaluate, waitFor } from './cdp-client.mjs';

// Production editor and styles, driven through the model-list quick entry.
export async function reasoningControlScenarios({ client, run, setup, click, shot, choose, current, editorReady }) {
  const key = async (key, code = key) => {
    const windowsVirtualKeyCode = { ArrowRight: 39, ArrowDown: 40 }[key];
    await client.send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode });
    await client.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode });
  };
  const save = async () => {
    await click('.toolbar-actions button', '保存并创建');
    await waitFor(client, 'productShell.commands.some(c=>c.command==="preview_plan_editor")');
    const writes = await evaluate(client, 'productShell.commands.filter(c=>c.command==="preview_plan_editor").map(c=>c.payload.input.editor)');
    assert.equal(writes.length, 1);
    return writes[0];
  };
  for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const [width, height] of [[1200,813], [755,600]]) {
    const id = `${language}-${theme}-${width}`;
    await run(`model-route.reasoning-control.layout-and-policy.${id}`, async () => {
      await setup({ language, theme, width, height, others: false });
      await click('.model-route-action button', language === 'zh' ? '创建智能路由' : 'Create routing');
      await editorReady();
      await waitFor(client, '!!document.querySelector("select.effort-select")');
      await evaluate(client, 'document.querySelector(".route-reasoning-policy").scrollIntoView({block:"start"});true');
      const policy = await evaluate(client, `(()=>{
        const e=document.querySelector('.route-reasoning-policy'),label=e.querySelector('.field-label');
        return {overflow:e.scrollWidth-e.clientWidth,font:getComputedStyle(label).fontSize,family:getComputedStyle(label).fontFamily,
          segments:[...e.querySelectorAll('.segment')].map(s=>({font:getComputedStyle(s).fontSize,family:getComputedStyle(s).fontFamily,height:s.getBoundingClientRect().height})),
          checked:e.querySelector('input:checked').value,count:e.querySelectorAll('input[type=radio]').length};
      })()`);
      assert.equal(policy.overflow, 0); assert.equal(policy.count, 2); assert.equal(policy.checked, 'follow');
      assert.equal(policy.font, '12px');
      assert.ok(policy.segments.every(s => s.font === policy.font && s.family === policy.family && s.height >= 30), JSON.stringify(policy));
      await shot(`reasoning-policy-${id}`);
      // Native radio keys stage the policy without altering the model setting.
      await evaluate(client, 'document.querySelector(".route-reasoning-policy input[value=follow]").focus();true');
      assert.equal(await evaluate(client, 'document.activeElement===document.querySelector(".route-reasoning-policy input[value=follow]")'), true);
      await key('ArrowRight');
      await waitFor(client, 'document.querySelector(".route-reasoning-policy input[value=fixed]").checked');
      assert.equal(await evaluate(client, 'document.querySelector("select.effort-select").value'), 'high');
      assert.equal((await current()).writes, 0);
      await evaluate(client, 'document.querySelector(".route-reasoning-field").scrollIntoView({block:"center"});document.querySelector("select.effort-select").focus();true');
      const field = await evaluate(client, `(()=>{
        const row=document.querySelector('.route-candidate'),e=row.querySelector('.route-reasoning-field'),label=e.querySelector('label'),select=e.querySelector('select'),arrow=e.querySelector('.icon'),controls=row.querySelector('.candidate-controls');
        const box=x=>{const r=x.getBoundingClientRect();return {left:r.left,top:r.top,right:r.right,bottom:r.bottom,width:r.width,height:r.height};};
        return {row:box(row),field:box(e),label:box(label),select:box(select),arrow:box(arrow),controls:box(controls),overflow:row.scrollWidth-row.clientWidth,
          fieldOverflow:e.scrollWidth-e.clientWidth,font:getComputedStyle(select).fontSize,labelWeight:getComputedStyle(label).fontWeight,
          focus:document.activeElement===select,outline:getComputedStyle(e).outlineStyle};
      })()`);
      assert.equal(field.overflow, 0); assert.equal(field.fieldOverflow, 0); assert.equal(field.font, '12px');
      assert.equal(field.labelWeight, '450'); assert.equal(field.focus, true); assert.equal(field.outline, 'solid');
      assert.ok(field.field.left >= field.row.left && field.field.right <= field.controls.left && field.field.bottom <= field.row.bottom && field.field.height >= 32, JSON.stringify(field));
      assert.ok(field.label.right <= field.select.left && field.arrow.right <= field.field.right && field.field.bottom <= height, JSON.stringify(field));
      await shot(`reasoning-level-focused-${id}`);
    });
  }
  await run('model-route.reasoning-control.inline-discrete-draft-and-save', async () => {
    await setup({ reasoning: { kind: 'discrete', parameter: 'reasoning_effort', profiles: ['low','medium','high','xhigh'] } });
    await click('.model-route-action button', '创建智能路由'); await click('.modal-foot .btn-primary', '一起加入候选'); await editorReady();
    await choose('.route-candidate select.effort-select', 'medium');
    assert.equal(await evaluate(client, 'document.querySelectorAll("[role=dialog]").length'), 0);
    assert.equal((await current()).writes, 0);
    assert.deepEqual(await evaluate(client, '[...document.querySelectorAll(".route-candidate select.effort-select")].map(e=>e.value)'), ['medium','xhigh']);
    await evaluate(client, 'document.querySelector("select.effort-select").focus();true'); await key('ArrowDown');
    await waitFor(client, 'document.querySelector("select.effort-select").value==="high"');
    const saved = await save();
    assert.equal(saved.follow_request_reasoning, true);
    assert.deepEqual(saved.candidates.map(c => c.reasoning), [{ kind: 'profile', profile: 'high' }, { kind: 'profile', profile: 'xhigh' }]);
  });
  await run('model-route.reasoning-control.toggle-is-typed-and-draft-only', async () => {
    await setup({ others: false, reasoning: { kind: 'toggle', parameter: 'thinking' } });
    await click('.model-route-action button', '创建智能路由'); await editorReady();
    await waitFor(client, '!!document.querySelector("select.effort-select")');
    assert.equal(await evaluate(client, 'document.querySelector(".route-reasoning-label").textContent'), '思考开关');
    await choose('select.effort-select', 'disabled'); assert.equal((await current()).writes, 0);
    assert.deepEqual((await save()).candidates[0].reasoning, { kind: 'toggle', enabled: false });
  });
  await run('model-route.reasoning-control.budget-validation-cancel-and-save', async () => {
    await setup({ others: false, reasoning: { kind: 'budget', parameter: 'thinking.budget_tokens', minimum_tokens: 1024, maximum_tokens: 8192, step_tokens: 1024 } });
    await click('.model-route-action button', '创建智能路由'); await editorReady();
    await click('button.effort-select', '1024'); await waitFor(client, '!!document.querySelector("#route-reasoning input")');
    const fill = async value => evaluate(client, `(()=>{const e=document.querySelector('#route-reasoning input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
    for (const value of ['0', '1200', '9000']) {
      await fill(value); await click('.modal-foot button', '应用');
      await waitFor(client, '!!document.querySelector("#route-reasoning [role=alert]")'); assert.equal((await current()).writes, 0);
    }
    await fill('4096'); await click('.modal-foot button', '取消');
    assert.match(await evaluate(client, 'document.querySelector("button.effort-select").textContent'), /1024/);
    await click('button.effort-select', '1024'); await fill('4096'); await click('.modal-foot button', '应用');
    await waitFor(client, '!document.querySelector("[role=dialog]")'); assert.equal((await current()).writes, 0);
    assert.match(await evaluate(client, 'document.querySelector("button.effort-select").textContent'), /4096/);
    assert.deepEqual((await save()).candidates[0].reasoning, { kind: 'budget', tokens: 4096 });
  });
  await run('model-route.reasoning-control.fixed-is-plain-text', async () => {
    await setup({ others: false, reasoning: { kind: 'fixed', profile: 'provider-default' } });
    await click('.model-route-action button', '创建智能路由'); await editorReady();
    await waitFor(client, 'document.querySelector(".route-reasoning-value")?.textContent==="供应商默认"');
    assert.match(await evaluate(client, 'document.querySelector(".route-reasoning-readonly").textContent'), /思考供应商默认/);
    assert.equal(await evaluate(client, 'document.querySelector(".route-reasoning-field").querySelectorAll("button,select,input").length'), 0);
    assert.equal((await save()).candidates[0].reasoning, undefined);
  });
  await run('model-route.reasoning-control.unsupported-saved-setting-stays-visible', async () => {
    await setup({ others: false, existing: true, savedReasoning: { kind: 'profile', profile: 'retained-custom' } });
    await click('.model-route-action button', '创建智能路由'); await click('.modal-foot .btn-primary', '编辑已有路由'); await editorReady();
    await waitFor(client, 'document.querySelector("select.effort-select")?.value==="retained-custom"');
    assert.equal(await evaluate(client, 'document.querySelector("select.effort-select").getAttribute("aria-invalid")'), 'true');
    assert.equal((await current()).writes, 0);
    assert.equal(await evaluate(client, 'document.querySelector("select.effort-select option:checked").disabled'), true);
    await choose('select.effort-select', 'low'); assert.equal((await current()).writes, 0);
    await click('.editor-actions button', '发布更改'); await waitFor(client, 'productShell.commands.some(c=>c.command==="preview_plan_editor")');
    assert.deepEqual(await evaluate(client, 'productShell.commands.find(c=>c.command==="preview_plan_editor").payload.input.editor.candidates[0].reasoning'), { kind: 'profile', profile: 'low' });
  });
}
