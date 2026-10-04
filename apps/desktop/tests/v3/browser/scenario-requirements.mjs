// Reviewed capability gates. New discovered scenarios run in full mode automatically.
// Retire a required ID only with an explicit replacement of its unique guarantee.
export const agentTrustRequired = ['codex.restore.conflict-active',
  'codex.enable.draft-cancel',
  'claude.enable.shared-presets',
  'codex.enable.confirmed-command',
  'codex.routing.fixed-binding',
  'codex.routing.plan-only',
  'codex.routing.mixed-native-default',
  'agent.configuration.shared-interactions',
  'codex.recovery.token-controls',
  'codex.routing.preview-rejection',
  'agent.diagnostics.non-runnable',
  'agent.diagnostics.rescan',
  'agent.diagnostics.late-failure',
  'agent.recovery.non-runnable',
  'agent.configuration.independent-facets',
  'agent.configuration.prerequisite',
  'claude.collaboration.check-before-save',
  'claude.collaboration.retry-preserves-edit',
  'claude.collaboration.obsolete-blocker',
  'qoder.collaboration.enable-without-model',
  'qoder.collaboration.retry-and-restore',
  'qoder.routing.additional-plans',
  'qoder.routing.adjust-and-restore',
  'qoder.routing.resume-pending',
  'agent.diagnostics.configuration-state',
  'codex.routing.unproven-native-model'];

export const routingWorkerRequired = ['routing.capabilities.candidate-vs-fixed',
  'worker.discovery.visible-only',
  'worker.installation.harness-isolation',
  'worker.installation.replace',
  'worker.installation.latest-edit',
  'qoder.installation.single-cli',
  'routing.save.unobserved-editable',
  'routing.publish.qoder-budget-conflict'];

export const focusedRequirements = {
  'claude-collaboration': ['claude.collaboration.check-before-save', 'claude.collaboration.retry-preserves-edit', 'claude.collaboration.obsolete-blocker'],
  'worker-replacement': ['worker.installation.replace', 'worker.installation.latest-edit'],
  'qoder': ['qoder.collaboration.enable-without-model', 'qoder.collaboration.retry-and-restore', 'qoder.routing.additional-plans', 'qoder.routing.adjust-and-restore', 'agent.configuration.shared-interactions', 'qoder.routing.resume-pending', 'qoder.installation.single-cli', 'routing.publish.qoder-budget-conflict', 'desktop.routing.qoder-budget-checkpoint'],
  'route-save': ['routing.save.unobserved-editable', 'routing.publish.qoder-budget-conflict'],
};

export const productShellRequired = [
  'desktop.settings.persisted-scale',
  'desktop.operation.dismiss-observes',
  'desktop.operation.identity-retry-converges',
  'desktop.operation.repeated-submission',
  'desktop.models.documentation-retains-input',
  'desktop.routing.reactivation-models',
  'desktop.routing.qoder-budget-checkpoint',
  'desktop.routing.classifier-protocol',
];
