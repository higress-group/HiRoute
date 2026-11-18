// Production form producers joined to the installed CLI/daemon acceptance test.
import assert from 'node:assert/strict';
import { registeredModelCandidates } from '../src/features/model-connections/registered-metadata.ts';
import { withRegisteredModels, buildCheckDraft, buildSaveChange } from '../src/features/model-connections/state.ts';

let bytes = '';
for await (const chunk of process.stdin) bytes += chunk;
const input = JSON.parse(bytes);
if (input.stage === 'save') {
  process.stdout.write(JSON.stringify(buildSaveChange(input.save)));
} else {
  const option = input.options.options.find(value => value.connection_option_id === 'openai.platform.global.v1');
  assert.ok(option);
  const cases = [];
  for (const modelId of ['gpt-6.2-astra', 'gpt-6.2-sol']) {
    for (const protocol of ['chat_completions', 'responses']) {
      const endpoint = option.endpoints.find(value => value.protocol === protocol);
      assert.ok(endpoint);
      const candidate = registeredModelCandidates({ ...option, endpoints: [endpoint] }, input.options.metadata_catalog)
        .find(value => value.upstream_model_id === modelId);
      assert.ok(candidate);
      for (const manual of [false, true]) {
        const key = `${modelId}-${protocol}-${manual ? 'manual' : 'automatic'}`;
        const models = withRegisteredModels([], [candidate]);
        if (manual) {
          models[0] = { ...models[0], capabilities: { ...models[0].capabilities,
            context_tokens: { value: 800_000, basis: 'user_declared' },
            max_output_tokens: { value: 64_000, basis: 'user_declared' },
          } };
        }
        // A metadata refresh adds missing models without replacing the user's edits.
        const refreshed = withRegisteredModels(models, [candidate]);
        const draft = buildCheckDraft({
          display_template_id: option.connection_option_id,
          candidate_ref: `candidate/input-limits/${key}`, lineage_ref: `lineage/input-limits/${key}`,
          inference_model_id: modelId, display_name: key, existing_source_id: null,
          edit_revision: 1, check_id: `check/input-limits/${key}`,
          base_url: endpoint.base_url, base_kind: 'api_root', request_path_override: endpoint.request_path,
          inventory_path_override: endpoint.inventory_path ?? null, protocol,
          protocol_profile_id: `profile/custom/${protocol}`, protocol_profile_revision: 1,
          authentication: endpoint.authentication_semantics, additional_endpoints: [],
          provenance: { kind: 'user_configured', configuration_revision: 1 }, models: refreshed,
        });
        cases.push({ key, modelId, protocol, manual, draft });
      }
    }
  }
  process.stdout.write(JSON.stringify(cases));
}
