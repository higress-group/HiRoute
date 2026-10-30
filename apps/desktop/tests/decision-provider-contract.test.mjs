import test from 'node:test';
import assert from 'node:assert/strict';
import {
  chooseProvider, currentBundle, independentPresets, newNativeDraft,
} from '../src/features/decision-services/form-state.ts';

test('OpenAI authoring uses one complete decision preset and unknown connections require a codec', () => {
  assert.equal(new Set(independentPresets.map(preset => preset.id)).size, independentPresets.length);
  const draft = chooseProvider(newNativeDraft('decision/example'), 'openai');
  assert.deepEqual(currentBundle(draft, null), {
    model: 'gpt-6-luna', endpoint: 'https://api.openai.com/v1/decisions', wire_protocol: 'openai_decisions',
  });
  assert.equal(currentBundle(chooseProvider(draft, 'compatible'), null).wire_protocol, null);
});
