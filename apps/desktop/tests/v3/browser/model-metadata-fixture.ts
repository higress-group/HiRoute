// Presentation only: installed daemon tests own persisted effects and admission.
import { readyManagement } from './product-fixtures';
import officialData from '../../../../../assets/release-facts/current/bundle/official-model-data.json';
const catalog = officialData.data.metadata_catalog;
const modelRecords = [
  catalog.model_records.find(record => record.model_record_key === 'hermes-agent/ai-gateway/google%2Fgemini-3-flash')!,
  catalog.model_records.find(record => record.model_record_key === 'hermes-agent/openai-codex/gpt-5.3-codex-spark')!,
];
const snapshot = {generation:1,data_revision:officialData.data_revision,data_digest:officialData.data_digest};
const state = { fail: false, stale: false, evidenceFail: false, evidenceStale: false, linkFail: false };
Object.assign(window,{metadataFixture:state});
export function metadataFixtureInvoke(command: string, payload: any, params: URLSearchParams): unknown {
  if(command === 'open_external_url') {
    if(state.linkFail) throw Error('EXTERNAL_URL_OPEN_FAILED');
    return null;
  }
  if (command === 'compute_defaults') {
    if(state.fail) throw Error('DAEMON_UNAVAILABLE');
    const source = readyManagement.sources.find(s=>s.source_id===payload.input.source_id)!;
    const values = {tool:true,vision:false,streaming:true,context_tokens:128000,max_output_tokens:32000,native_reasoning:{kind:'fixed',profile:'provider-default'}};
    return {source_id:source.source_id,source_revision:source.revision,revisions:readyManagement.revisions,reviewed_on:'2026-10-21',models:source.models.map(model=>({binding_id:model.binding_id,model_configuration_id:model.catalog_configuration_id,fields:Object.fromEntries(Object.entries(values).map(([key,value])=>[key,{saved_value:key==='context_tokens'?64000:value,effective_value:value,intent:key==='vision'?'preserve_saved':'follow_official',origin:key==='vision'?'preserved':'website_cache',locator:{model_configuration_id:'model/qwen',upstream_model_id:model.upstream_model_id,connector_id:'connector/fixture',endpoint_profile_id:'endpoint/fixture',protocol_endpoint_id:'endpoint/fixture/chat'}}]))}))};
  }
  if (command === 'preview_compute_save_v3') {
    if(state.fail) throw Error('CHANGE_PREVIEW_STALE');
    return {spec:{schema_version:{major:1,minor:0},command_id:'compute.connection.apply',resource_id:payload.change.subject.source_id,desired_state:payload.change},accept_digest:'sha256:fixture',expected_revisions:readyManagement.revisions,summary:[],source_id:payload.change.subject.source_id};
  }
  if (command === 'list_model_metadata') {
    if (state.fail) throw Error('DAEMON_UNAVAILABLE');
    if(payload.input.cursor && state.stale) {state.stale=false;throw {message_key:'application.error.change_preview_stale'};}
    const kind=payload.input.kind;
    const value:Record<string,unknown>=kind==='price'?{upstream_model_id:'Example model',product_key:'Example API',currency:'USD',schedules:[{schedule_key:'standard',condition:{kind:'unconditional'},application:'whole-request',rates:{input:'0',output:'1.5','cache-read':'0.025','cache-write':'0.125'}}]}:kind==='evidence_source'?{title:'Provider model documentation',locator:'https://example.test/models',collected_on:'2026-10-21'}:modelRecords[payload.input.cursor?1:0];
    return {snapshot,records:[{kind,key:kind==='model_metadata'?value.model_record_key:payload.input.cursor?'second':'first',origin:'website_cache',qualification:'descriptive_only',value}],total:2,next_cursor:payload.input.cursor?null:'next',complete:!!payload.input.cursor};
  }
  if (command === 'get_model_metadata') {
    if(state.evidenceFail) throw Error('DAEMON_UNAVAILABLE');
    if(state.evidenceStale) {state.evidenceStale=false;throw {message_key:'application.error.change_preview_stale'};}
    const {kind,key}=payload.input;
    const records:Record<string,unknown[]>= {inference_rule:catalog.inference_rules,client_run:catalog.client_runs,evidence_source:catalog.evidence_sources};
    const field=kind==='inference_rule'?'rule_key':'source_key';
    const value=records[kind]?.find(record=>(record as Record<string,unknown>)[field]===key);
    if(!value) throw Error('INVALID_ARGUMENTS');
    return {snapshot,record:{kind,key,origin:'website_cache',qualification:'descriptive_only',value}};
  }
  if (command === 'model_feed_remove_preview') {
    const paged=params.has('metadata'), next=paged && !payload?.input?.cursor;
    return {sources:[{source_id:'source/feed',display_name:'Team experimental model connection with a deliberately long readable name',model_configuration_ids:['model.trial'],published_revision_count:1}],impact:[{source_id:'source/feed',source_name:'Team experimental model connection with a deliberately long readable name',binding_id:'binding/feed',model_configuration_id:'model.trial',model_display_name:'Example model',changes:{[next?'context_tokens':'prices:USD:usage_estimate']:{current:128000,fallback:64000}}}],total:paged?2:1,next_cursor:next?'next':null,complete:!next,published_revision_count:1,retained_sequence:12,retrieval_enabled:true,digest:'sha256:preview'};
  }
  return undefined;
}
