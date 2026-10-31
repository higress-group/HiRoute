// README capture only: the trial and model-performance fixtures with English source and
// plan names, plus an illustrative cost comparison. Not part of the production build.
import { readyManagement, dailyPlan } from './tests/v3/browser/product-fixtures';

const english = new URLSearchParams(location.search).get('lang') === 'en';
if (english) {
  const names: Record<string, string> = {
    'Codex 订阅': 'Codex subscription', '百炼 Coding Plan': 'Bailian Coding Plan',
    'OpenRouter 免费模型': 'OpenRouter free models', '智谱免费模型': 'Zhipu free models',
    '硅基流动免费模型': 'SiliconFlow free models',
  };
  for (const source of readyManagement.sources) source.display_name = names[source.display_name] ?? source.display_name;
  dailyPlan.desired.display_name = 'Daily coding';
  dailyPlan.desired.purpose = 'Implement features, fix bugs and design complex code';
}

await import('./tests/v3/browser/new-model-trial.tsx');

const internals = (window as any).__TAURI_INTERNALS__;
const invoke = internals.invoke.bind(internals);
internals.invoke = async (command: string, payload: any, options: any) => {
  const value = await invoke(command, payload, options);
  if (command === 'decision_services') value.services = value.services.map((service: any) => ({ ...service, name: 'System One' }));
  if (command === 'observation_read' && payload?.request?.intent?.view === 'plan_trust') {
    value.cost = { kind: 'available', difference_micros: -8_440_000, comparison: { currency: 'USD', coverage: 'complete',
      request_count: 412, compared_request_count: 412, missing_request_count: 0, excluded_request_count: 0,
      routed_estimate_micros: 3_420_000, reference_estimate_micros: 11_860_000,
      compared_routed_subtotal_micros: 3_420_000, compared_reference_subtotal_micros: 11_860_000,
      retention_boundary_partial: false, arithmetic_overflow: false } };
    value.announcements = [];
  }
  return value;
};
