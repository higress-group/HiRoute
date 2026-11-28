export type ConfigurationBundle = {
  schema: 'hiroute.configuration-bundle/v1';
  worker_settings: { schema: 'hiroute.worker-settings/v1'; max_concurrent: number };
  observation_capacity: { max_active_bytes: number | null };
  model_feed: { retrieval_enabled: boolean; acknowledge_disclosure: boolean };
};

export type ConfigurationImportState = 'applied' | 'unchanged' | 'failed';
export type ConfigurationImportItem = { state: ConfigurationImportState; error_code?: string };
export type ConfigurationImportResult = {
  schema: string;
  worker_settings: ConfigurationImportItem;
  observation_capacity: ConfigurationImportItem;
  model_feed: ConfigurationImportItem;
};

/** Light client-side check before sending the bundle; the daemon re-validates authoritatively. */
export function validConfigurationBundle(value: unknown): value is ConfigurationBundle {
  if (!value || typeof value !== 'object') return false;
  const bundle = value as Record<string, unknown>;
  if (bundle.schema !== 'hiroute.configuration-bundle/v1') return false;
  const worker = bundle.worker_settings as Record<string, unknown> | undefined;
  if (!worker || worker.schema !== 'hiroute.worker-settings/v1') return false;
  if (!Number.isInteger(worker.max_concurrent)) return false;
  if ((worker.max_concurrent as number) < 1 || (worker.max_concurrent as number) > 1000) return false;
  const capacity = bundle.observation_capacity as Record<string, unknown> | undefined;
  if (!capacity) return false;
  if (capacity.max_active_bytes === null) return isBooleans(bundle.model_feed);
  if (typeof capacity.max_active_bytes !== 'number') return false;
  if (!Number.isSafeInteger(capacity.max_active_bytes) || capacity.max_active_bytes <= 0) return false;
  return isBooleans(bundle.model_feed);
}

/** Parse and admit an exported file; malformed JSON and rejected bundles are indistinguishable to the caller. */
export function parseConfigurationBundle(text: string): ConfigurationBundle | undefined {
  try {
    const value: unknown = JSON.parse(text);
    return validConfigurationBundle(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

function isBooleans(modelFeed: unknown): boolean {
  if (!modelFeed || typeof modelFeed !== 'object') return false;
  const feed = modelFeed as Record<string, unknown>;
  return typeof feed.retrieval_enabled === 'boolean' && typeof feed.acknowledge_disclosure === 'boolean';
}
