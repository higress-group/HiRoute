export function trialModelNavigationScenarios(scenario) {
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  async function until(predicate, label) {
    const deadline = Date.now() + 6000;
    while (!predicate()) { if (Date.now() > deadline) throw new Error(`Timed out: ${label}`); await pause(25); }
    await pause(30);
  }
  const visible = element => element.getClientRects().length > 0;
  const button = (selector, text) => [...document.querySelectorAll(selector)].find(element => visible(element) && !element.disabled && element.textContent.includes(text));
  const input = (element, value) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(element, value);
    element.dispatchEvent(new Event('input', { bubbles: true }));
  };
  return [scenario('desktop.trial.connected-models-and-models-return', ['trial-model-navigation'], 'The model chooser opens Models for additions and preserves the trial on return', async () => {
    const c = window.productShell;
    try {
      c.reset();
      // This shell fixture predates the coherent trial read. Derive only this
      // scenario's response from its existing authoritative fixture objects.
      let addedModel = false;
      const addedBinding = 'binding/fixture/added-while-away';
      const addedCandidate = { binding_id: addedBinding, model_configuration_id: 'fixture-added-model', display_name: 'Added while away', reasoning: { kind: 'fixed', profile: 'provider-default' }, billing_class: 'paid', routable: true, ingress_protocols: ['responses'] };
      c.handlers.trial_setup_snapshot = () => ({
        schema: 'hiroute.trial-setup-snapshot/v1',
        options: { ...c.fixtureResponse('plan_editor_options', { input: { authoring: {} } }), candidates: [...c.fixtureResponse('plan_editor_options', { input: { authoring: {} } }).candidates, ...(addedModel ? [addedCandidate] : [])] },
        management: {
          revisions: c.management.revisions,
          retained_api_connections: [],
          sources: c.management.sources.map(source => ({
            source_id: source.source_id, revision: source.revision, display_name: source.display_name,
            provenance: source.provenance, state: source.state, actions: source.actions,
            access_kind: source.connection_identity?.access_kind ?? 'unknown',
            models: source.models.map(model => ({
              model_ref: model.model_ref, binding_id: model.binding_id, revision: model.revision,
              upstream_model_id: model.upstream_model_id, display_name: model.display_name,
              membership: model.membership, catalog_configuration_id: model.catalog_configuration_id,
              catalog_admitted: false, availability: model.presentation?.availability ?? 'unknown',
            })),
          })),
        },
      });
      await until(() => button('.new-models button', '试用其他模型'), 'trial entry');
      button('.new-models button', '试用其他模型').click();
      await until(() => document.querySelector('[data-trial-connected-model]'), 'connected-model control');
      document.querySelector('[data-trial-connected-model]').click();
      await until(() => document.querySelector('.trial-picker .trial-choice:not(:disabled)'), 'saved-model choices');
      assert(!document.querySelector('.trial-picker .trial-model-card'), 'Recommendations entered the saved-model chooser');
      document.querySelector('.trial-picker .trial-choice:not(:disabled)').click();
      await until(() => !document.querySelector('.trial-picker') && document.querySelector('[data-trial-connected-model]')?.dataset.trialBinding, 'exact saved selection');
      const binding = document.querySelector('[data-trial-connected-model]').dataset.trialBinding;
      input(document.querySelector('[data-trial-target=name] input'), 'Kept trial route');
      input(document.querySelector('.trial-setup .trust-slider'), '723');
      document.querySelector('[data-trial-connected-model]').click();
      await until(() => button('.trial-picker .actions button', '添加模型'), 'add-model action');
      button('.trial-picker .actions button', '添加模型').click();
      await until(() => document.querySelector('.v3-add-choices') && button('.trial-model-return button', '返回试用'), 'actual Models page');
      assert(document.querySelector('.trial-page-host').hidden, 'Trial was still presented over Models');
      button('[role=dialog] .modal-head button', '').click();
      await until(() => !document.querySelector('[role=dialog]'), 'Models chooser closed');
      // A completed model-page change becomes visible on the next coherent read.
      c.management.sources[1].models.push({ model_ref: 'model-ref/added', binding_id: addedBinding, revision: 1, upstream_model_id: 'fixture-added-model', display_name: 'Added while away', membership: 'user_declared', presentation: { availability: 'available' } });
      addedModel = true;
      button('.trial-model-return button', '返回试用').click();
      await until(() => !document.querySelector('.trial-page-host').hidden && !document.querySelector('.trial-setup')?.disabled, 'resumed trial');
      assert(document.querySelector('[data-trial-target=name] input').value === 'Kept trial route', 'Model addition discarded the trial name');
      assert(document.querySelector('.trial-setup .trust-slider').value === '723', 'Model addition reset initial trust');
      assert(document.querySelector('[data-trial-connected-model]').dataset.trialBinding === binding, 'Return silently selected another model');
      assert(!document.querySelector('[data-trial-connected-model]').closest('[inert]'), 'A hidden Models dialog retained input ownership');
      document.querySelector('[data-trial-connected-model]').click();
      await until(() => button('.trial-picker .trial-choice', 'Added while away'), 'freshly added connected model');
      assert(!c.commands.some(call => ['preview_compute_save', 'apply_compute_save', 'preview_plan_editor'].includes(call.command)), 'Browsing or returning wrote configuration');
    } finally { c.reset(); await pause(50); }
  })];
}
