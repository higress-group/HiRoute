export type ModelFeedStatus = {
  recovery_available?: boolean;
  retrieval_enabled: boolean;
  held: { sequence: number; issued_at: string; retrieved_at: number } | null;
  highest_accepted_sequence: number | null;
  last_attempt: { at: number; result: string; reason: string | null } | null;
  verification: { kind: 'validated' | 'none' } | { kind: 'discarded'; reason: string };
  counts: { applied: number; ignored_embedded: number; newer_client: number; unresolved: number };
  next_automatic_attempt_at: number | null;
  disclosure: string;
  disclosure_acknowledged: boolean;
};
export type ModelAvailability = { kind: 'executable' | 'needs_connection' } | { kind: 'needs_newer_client'; version: string } | { kind: 'used_by_published_plans'; count: number };
export type ModelOrigin = { kind: 'embedded'; client_version: string } | { kind: 'feed'; sequence: number; included_on: string } | { kind: 'published' };
export type ModelReasoning = { kind: 'toggle' | 'budget' } | { kind: 'profiles'; profiles: string[] };
export type ModelFacts = { publisher_id: string; context_tokens: number; tool: boolean; vision: boolean; reasoning: ModelReasoning | null };
export type AvailableModel = { model_configuration_id: string; display_name: string; origin: ModelOrigin; identity_stability: string | null; availability: ModelAvailability; binding_ids: string[]; facts: ModelFacts | null };
export type ModelAnnouncement = { id: string; kind: 'new_model' | 'model_update'; model_configuration_id: string; headline: { en: string; zh: string }; note: { en: string; zh: string }; links: string[]; included_on: string };
export type ModelAvailabilityResult = { featured: ModelAnnouncement[]; models: AvailableModel[] };
export type FeedRemovalPreview = {
  sources: { source_id: string; display_name: string; model_configuration_ids: string[]; published_revisions: { plan_id: string; content_revision: number }[] }[];
  published_revisions: { plan_id: string; content_revision: number }[];
  retained_sequence: number | null;
  retrieval_enabled: boolean;
  digest: string;
};
