import type { ModelAvailabilityResult } from './types';

/** Recommendations come only from the selected official feed. An empty list
 * withdraws recommendations while the model catalog remains available. */
export function modelAnnouncements(data: ModelAvailabilityResult) {
  const fromFeed = data.featured.length > 0;
  return {
    fromFeed,
    entries: data.featured.filter(entry => data.models.some(model => model.model_configuration_id === entry.model_configuration_id)),
  };
}
