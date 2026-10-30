import { branchRouting } from '../../../src/features/decision-services/types.ts';
import { defaultJudgment } from '../../../src/features/decision-services/types.ts';
import stock from '../../../../../crates/domain/src/routing/fixtures/system_one_c_v1.json' with { type: 'json' };
import guidance from '../../../../../assets/decision-services/default-assessment-guidance.json' with { type: 'json' };
import { resolveAssessment } from '../../../src/features/decision-services/native-protocol.ts';
import { scenario } from './scenario-selection.mjs';

export function cProtocolScenarios({ fresh, routing, click, until, all, text, calls, setInput, control }) {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const service = { id:'decision/ui-fixture',revision:7,name:'Saved decision fixture',connection:{kind:'system_one',provider:'compatible',model:'synthetic-model',endpoint:'https://decision.example/systemone',timeout_ms:3000,auth_header:{name:'Authorization',value_secret_ref:'decision/ui-fixture/r7'}} };
  async function open(language, { branches=false, custom=false }={}) {
    await fresh(() => {
      control().handlers.decision_services=()=>({services:[service]});
      let selectedPlan;
      control().handlers.desktop_snapshot=payload=>{
        const snapshot=structuredClone(control().fixtureResponse('desktop_snapshot',payload));
        const plan=snapshot.catalog.plans[0],strategy=plan.desired.strategy;
        strategy.classifier={kind:'decision_service',service};
        if(custom){strategy.judgment.competence.standard='custom';strategy.judgment.competence.instructions='  Keep this exact text.\nSecond line.  ';strategy.judgment.competence.criteria[1]+='  ';}
        plan.desired.delegation_enabled=false;
        if(branches){
          const route=branchRouting(service);route.judgment=structuredClone(strategy.judgment);
          route.branches.forEach((b,i)=>{b.name='Category '+i;b.condition='Work '+i;b.candidates=structuredClone(strategy.economy);b.primary_candidates=structuredClone(strategy.primary);b.judgment=i===0?structuredClone(strategy.judgment):null;});
          plan.desired.mode='custom_branches';plan.desired.strategy={mode:'branches',routing:route};
        }
        selectedPlan=plan;
        return snapshot;
      };
      control().handlers.plan_editor_options=payload=>{
        const result=structuredClone(control().fixtureResponse('plan_editor_options',payload));
        if(payload?.input?.authoring){
          control().handlers.desktop_snapshot({});
          const editor=result.authoring.editor,strategy=structuredClone(selectedPlan.desired.strategy);
          editor.delegation_enabled=false;
          editor.mode=selectedPlan.desired.mode;
          const native=j=>resolveAssessment(j,true);
          if(editor.mode==='custom_branches'){
            editor.branch_routing=strategy.routing;
            editor.branch_routing.judgment=native(editor.branch_routing.judgment);
            editor.branch_routing.branches=editor.branch_routing.branches.map(b=>b.judgment?{...b,judgment:native(b.judgment)}:b);
          }else{
            editor.smart={...editor.smart,...strategy,judgment:native(strategy.judgment)};
          }
        }
        return result;
      };
    });
    localStorage.setItem('hiroute.language',language);control().remount();
    if(language==='zh')await routing();else {await click('Smart routing');await until(()=>all('.master-list .list-row').length,'routes');all('.master-list .list-row')[0].click();}
    await until(()=>document.querySelector('.decision-judgment-fields'),'native judgment settings');
    await until(()=>calls('plan_editor_options').some(c=>(branches?c.payload.input.editor?.branch_routing?.classifier:c.payload.input.editor?.smart.classifier)?.kind==='decision_service'),'loaded route options');
  }
  function assertCopy(){
    const visible=text();
    assert(!/hiroute\.system-one|sha256:|Stock wording|Stock C|C decisions|Question 3|第三问|恢复版本|Confirm assessments|审阅/.test(visible),'Implementation or review wording remains visible');
    assert(!document.querySelector('[data-assessment-transition]'),'Extra transition review remains');
  }
  function geometry(){
    assert(!document.querySelector('.native-protocol-editor'),'Retired capability section remains');
    for(const e of all('.decision-judgment-fields .field-label,.decision-judgment-fields textarea,.decision-judgment-fields button'))assert(e.scrollWidth<=e.clientWidth+1,'Judgment content overflows');
    for(const row of all('.decision-thresholds')){
      const inputs=[...row.querySelectorAll('input')].map(e=>e.getBoundingClientRect()).filter(r=>r.width&&r.height);
      if(inputs.length===2&&inputs[1].left>inputs[0].left){
        assert(Math.abs(inputs[0].top-inputs[1].top)<=1,'Side-by-side threshold inputs are vertically misaligned');
        assert(Math.abs(inputs[0].height-inputs[1].height)<=1,'Threshold input height depends on helper wrapping');
        const help=[...row.querySelectorAll('.field-help')].map(e=>e.getBoundingClientRect());
        assert(Math.abs(help[0].top-help[1].top)<=1,'Threshold helper text does not start on the same row');
      }
    }
  }
  async function publish(t){
    const before=calls('preview_plan_editor').length;
    await click(t('发布更改','Publish changes'));
    await until(()=>calls('preview_plan_editor').length>before,'ordinary publication dispatch');
    const input=calls('preview_plan_editor').at(-1).payload.input;
    assert(!('assessment_choices' in input)&&!JSON.stringify(input.editor).includes('assessment_review'),'Review metadata entered publication');
    assert(!calls('plan_editor_transition_preview').length,'Extra Preview operation remains');
    return input;
  }
  return [scenario('desktop.routing.native-default',['routing-editor','decision-services','c-protocol'],'Native editing has one default and ordinary publication without capability controls',async()=>{
    let previousDefinition;
    for(const language of ['zh','en']){
      const t=(zh,en)=>language==='zh'?zh:en;await open(language);
      assertCopy();geometry();
      for(const field of all('.decision-judgment-fields'))for(const detail of field.querySelectorAll('details'))if(!detail.open)detail.querySelector('summary').click();
      const criteria=stock.q1.criteria;
      assert(document.querySelector('[data-decision-field="smart-criterion-0"]').value===criteria[0],'Released default did not resolve to current native default');
      assert(document.querySelector('[data-decision-field="smart-criterion-2"]').value===guidance.highest_criterion,'Positive-feedback criterion missing');
      const instructions=document.querySelector('[data-decision-field="smart-competence-instructions"]').value;
      assert(instructions.endsWith(guidance.feedback),'Feedback guidance missing from displayed model instructions');
      setInput(document.querySelector('[data-decision-field="smart-floor"]'),'0.51');
      await until(()=>document.querySelector('[data-decision-field="smart-floor"]').value==='0.51','ordinary edit');
      const input=await publish(t);
      const definition=input.editor.smart.judgment.competence;
      assert(definition.instructions===instructions,'Displayed and saved model instructions differ');
      if(previousDefinition) assert(JSON.stringify(definition)===JSON.stringify(previousDefinition),'UI locale changed the saved default definition');
      previousDefinition=definition;
      for(const key of ['historical_applicability','native_protocol','prior_context_max_turns'])assert(!(key in input.editor.smart),'Retired field entered editor input: '+key);
      assert(input.editor.smart.judgment.competence.floor_millis===510&&input.editor.smart.judgment.degree.simple_threshold_millis===800,'Threshold changed');
      assert(!calls('test_classifier_decision').length,'Opening/publication dispatched a model request');
    }
  }),scenario('desktop.routing.custom-assessment',['routing-editor','decision-services','c-protocol'],'Custom text survives native editing and reset restores current defaults',async()=>{
    for(const language of ['zh','en'])for(const branches of [false,true]){
      const t=(zh,en)=>language==='zh'?zh:en;await open(language,{custom:true,branches});
      const selector=`[data-decision-field="${branches?'branch-0':'smart'}-competence-instructions"]`;
      const field=document.querySelector(selector),original=field.value;
      assert(original==='  Keep this exact text.\nSecond line.  ','Opening rewrote custom bytes');
      const parents=[];for(let p=field.parentElement;p;p=p.parentElement)if(p.tagName==='DETAILS')parents.unshift(p);for(const p of parents)if(!p.open)p.querySelector('summary').click();
      const disclosure=field.closest('details');const reset=[...disclosure.querySelectorAll('button')].find(b=>b.textContent===t('恢复默认','Reset to default'));assert(reset,'Reset missing');reset.click();
      await until(()=>document.querySelector(selector).value!==original,'reset current default');assertCopy();geometry();
      const input=await publish(t);const j=branches?input.editor.branch_routing.branches[0].judgment:input.editor.smart.judgment;
      assert(j.competence.standard==='default','Reset did not record intent');assert(JSON.stringify(j.competence.criteria)===JSON.stringify([...stock.q1.criteria.slice(0,2),guidance.highest_criterion]),'Reset returned legacy defaults');
      assert(!calls('test_classifier_decision').length,'Edit implicitly probed');
    }
  }),scenario('desktop.decisions.connectivity-test',['decision-services','c-protocol'],'The connection page retains its original single V1 connectivity test',async()=>{
    for(const language of ['zh','en']){
      await fresh(()=>{control().handlers.decision_services=()=>({services:[service]});control().handlers.test_classifier_decision=({input})=>{assert(input.classifier?.service.revision===7&&!input.request,'Connection switched contract');return {schema:'hiroute.classifier-decision-test-result/v1',outcome:'passed',duration_millis:1};};});
      localStorage.setItem('hiroute.language',language);control().remount();const t=(zh,en)=>language==='zh'?zh:en;
      await click(t('模型','Models'));await click(t('决策模型','Decision models'));await until(()=>document.querySelector('.decision-heading h2')?.textContent===service.name,'connection details');await click(t('测试连接','Test connection'));
      await until(()=>text().includes(t('连接测试通过','Connection test passed')),'connection test');
      assert(!all('select').some(e=>e.getAttribute('aria-label')?.includes('shape')),'Shape selector remains');assert(calls('test_classifier_decision').length===1,'Connection sent more than one request');assertCopy();
    }
  }),scenario('desktop.quality.c-observations', ['routing-editor', 'sessions', 'c-protocol'], 'Retained C counts, denominators, partial coverage and empty states remain readable', async () => {
    for (const language of ['zh', 'en']) {
      const t = (zh, en) => language === 'zh' ? zh : en;
      let partial = false, empty = false;
      let releaseRefresh = null;
      const cohort = { protocol: 'hiroute.system-one-c/v1', content_digest: 'sha256:' + 'a'.repeat(64), bundle_digest: 'sha256:' + 'b'.repeat(64),
        scope_id: 'synthetic-category-with-long-visible-name', standard: 'custom', provider: 'fixture-provider', model: 'synthetic-c-model-with-long-name', simple_threshold_millis: 800, competence_floor_millis: 500 };
      await fresh(() => {
        control().handlers.observation_read = async payload => {
          if (payload.request.intent.view !== 'plan_quality') return control().fixtureResponse('observation_read', payload);
          assert(payload.request.schema === 'hiroute.observation.query/v2' && payload.request.intent.query.include_routing_observations === true, 'Desktop did not use the current versioned query');
          if (partial) await new Promise(resolve => { releaseRefresh = resolve; });
          return { samples: [], summary: { models: [], scored_stage_count: 0, unrated_stage_count: 0, session_count: 0, available_revisions: [8] }, next_cursor: null,
            routing_observations: { coverage: partial ? 'partial' : 'complete', covered_from_ms: 0, covered_to_ms: Date.now(), retained_decisions: empty ? 0 : 11,
              known_non_c_decisions: 1, legacy_or_unknown_decisions: partial ? 2 : 0, unknown_scope_decisions: 0, missing_projection_events: partial ? 1 : 0, scan_limited: partial,
              cohorts: empty ? [] : [{ cohort, counts: { c_decisions: 10, inherited: 1, requested: 7, unavailable: 4, no_target_omissions: 1, partial_target_omissions: 1,
                required_question_missing: 1, gate_opportunities: 4, suppressed: 1, unavailable_per_requested: partial ? null : 4 / 7,
                suppressed_per_opportunity: partial ? null : 1 / 4, suppressed_per_c_decision: partial ? null : 1 / 10 } }] } };
        };
      });
      localStorage.setItem('hiroute.language', language); control().remount();
      if (language === 'zh') await routing();
      else { await click('Smart routing'); await until(() => all('.master-list .list-row').length, 'saved routes'); all('.master-list .list-row')[0].click(); }
      await click(t('模型表现', 'Model performance'));
      await until(() => all('.routing-observations summary').length, 'C observation entry');
      all('.routing-observations summary')[0].click();
      await until(() => text().includes('57.1%') && text().includes('25.0%') && text().includes('10.0%'), 'retained denominator rates');
      assert(all('.routing-counts')[0].textContent.includes('4 / 7'), 'Unavailable numerator or request denominator lost');
      for (const element of all('.routing-counts dt, .routing-counts dd, .routing-cohort strong, .quality-diagnostic-identity')) {
        assert(element.scrollWidth <= element.clientWidth + 1, 'C observation identity or count overflows');
      }
      await until(() => !all('.quality-refresh')[0].disabled, 'initial quality read settled');
      partial = true; all('.quality-refresh')[0].click();
      await until(() => releaseRefresh, 'pending observation refresh');
      await until(() => !all('.routing-counts').length, 'stale counts cleared during refresh');
      releaseRefresh(); releaseRefresh = null;
      await until(() => text().includes(t('覆盖不完整，比例不可用', 'Partial coverage; rates unavailable')), 'partial coverage');
      assert(document.querySelector('.routing-observations').open, 'Refreshing collapsed the observation details');
      assert(text().includes(t('来源无法确认的决策 2', 'Decisions of unconfirmed origin 2')), 'Unconfirmed decision origins are not explained');
      assert(!all('.routing-counts')[0].textContent.includes('%') && all('.routing-counts')[0].textContent.includes('4 / 7'), 'Partial coverage hides counts or claims a rate');
      await until(() => !all('.quality-refresh')[0].disabled, 'partial quality read settled');
      empty = true; partial = false; all('.quality-refresh')[0].click();
      await until(() => text().includes(t('此范围内暂无历史问题适用性决策记录', 'No historical problem applicability decisions are available')), 'empty retained scope');
    }
  })];
}
