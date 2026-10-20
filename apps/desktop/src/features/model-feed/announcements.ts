import bundled from '../../../../../assets/model-feed/featured.json' with { type: 'json' };
import type { ModelAnnouncement, ModelAvailabilityResult } from './types';

/** The maintained editorial input is also shipped with the client. Model facts
 * and executable bindings still come exclusively from the service's catalog. */
export function modelAnnouncements(data: ModelAvailabilityResult) {
  const fromFeed = data.featured.length > 0;
  const announcements = fromFeed ? data.featured : bundled as ModelAnnouncement[];
  return {
    fromFeed,
    entries: announcements.filter(entry => data.models.some(model => model.model_configuration_id === entry.model_configuration_id)),
  };
}
