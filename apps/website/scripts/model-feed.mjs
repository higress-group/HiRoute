import { createHash } from 'node:crypto';
import { appendFile, lstat, mkdir, open, rename, rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const FEED_URL = 'https://hiroute.ai/model-feed/v2.json';
export const MAX_FEED_BYTES = 16 * 1024 * 1024;
const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const U64_MAX = (1n << 64n) - 1n;
const recordIdFields = {
  model: 'model_configuration_id', model_endpoint_capability: 'capability_id',
  offer: 'offer_id', price_rate: 'price_rate_id',
};

function requireThat(condition, message) {
  if (!condition) throw new Error(message);
}

function object(value, label) {
  requireThat(value !== null && typeof value === 'object' && !Array.isArray(value), `${label}: expected an object`);
}

function fields(value, required, optional, label) {
  object(value, label);
  requireThat(required.every(key => Object.hasOwn(value, key))
    && Object.keys(value).every(key => required.includes(key) || optional.includes(key)), `${label}: invalid fields`);
}

function text(value, label, maximum = 1024) {
  requireThat(typeof value === 'string' && value.trim().length > 0
    && [...value].length <= maximum && !/[\u0000-\u001f\u007f]/u.test(value), `${label}: invalid text`);
}

function rejectDuplicateKeys(source) {
  // JSON.parse checks grammar first. Walk its lexical tokens to reject ambiguous
  // duplicate fields, including keys written with different Unicode escapes.
  const stack = [];
  for (const [token] of source.matchAll(/"(?:[^"\\]|\\.)*"|[{}[\],:]|[^{}[\],:\s]+/g)) {
    if (token === '{') stack.push({ keys: new Set(), wantsKey: true });
    else if (token === '[') stack.push({ keys: null });
    else if (token === '}' || token === ']') stack.pop();
    else {
      const frame = stack.at(-1);
      if (frame?.keys) {
        if (token === ',') frame.wantsKey = true;
        else if (frame.wantsKey && token.startsWith('"')) {
          const key = JSON.parse(token);
          requireThat(!frame.keys.has(key), 'duplicate JSON field');
          frame.keys.add(key);
          frame.wantsKey = false;
        }
      }
    }
  }
}

export function parseJson(bytes, label) {
  requireThat(bytes.length <= MAX_FEED_BYTES, `${label}: exceeds 16 MiB`);
  let source;
  try { source = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes); }
  catch { throw new Error(`${label}: invalid UTF-8`); }
  try {
    const parsed = JSON.parse(source, (key, value, context) => {
      // Preserve the entire u64 range, including sequences beyond JS safe integers.
      if (key === 'sequence' || key === 'data_revision') {
        requireThat(typeof value === 'number' && context?.source?.length <= 20
          && /^[1-9]\d*$/.test(context.source), 'invalid sequence');
        const sequence = BigInt(context.source);
        requireThat(sequence <= U64_MAX, 'invalid sequence');
        return sequence;
      }
      if (typeof value === 'number') {
        requireThat(/^-?\d+$/.test(context.source), 'decimal rates must be strings');
        const integer = BigInt(context.source);
        requireThat(integer >= -(1n << 63n) && integer <= U64_MAX, 'integer exceeds contract bounds');
        return Number.isSafeInteger(value) ? value : integer;
      }
      return value;
    });
    rejectDuplicateKeys(source);
    structural(parsed);
    return parsed;
  } catch { throw new Error(`${label}: invalid JSON or sequence`); }
}

function date(value, label) {
  requireThat(typeof value === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(value)
    && Number.isFinite(Date.parse(`${value}T00:00:00Z`))
    && new Date(`${value}T00:00:00Z`).toISOString().slice(0, 10) === value, `${label}: invalid date`);
}

function publication(value) {
  requireThat(typeof value.sequence === 'bigint' && value.sequence >= 1n && value.sequence <= U64_MAX, 'publication: invalid sequence');
  requireThat(typeof value.issued_at === 'string'
    && /^\d{4}-\d{2}-\d{2}T(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z|\+00:00)$/.test(value.issued_at)
    && Number.isFinite(Date.parse(value.issued_at)), 'publication: issued_at must be RFC 3339 UTC');
  date(value.issued_at.slice(0, 10), 'publication issued_at');
}

function version(value, label) {
  const numeric = '(?:0|[1-9][0-9]*)';
  const prerelease = '(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)';
  requireThat(typeof value === 'string' && new RegExp(`^${numeric}\\.${numeric}\\.${numeric}(?:-${prerelease}(?:\\.${prerelease})*)?(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$`).test(value), `${label}: invalid version`);
}

function featuredEntries(entries) {
  requireThat(Array.isArray(entries), 'featured: expected an array');
  const ids = new Set();
  for (const entry of entries) {
    fields(entry, ['id', 'kind', 'model_configuration_id', 'headline', 'note', 'links', 'included_on'], [], 'featured entry');
    text(entry.id, 'featured id');
    requireThat(!ids.has(entry.id), 'featured: duplicate id');
    ids.add(entry.id);
    requireThat(['new_model', 'model_update'].includes(entry.kind), 'featured: unknown kind');
    text(entry.model_configuration_id, 'featured model');
    for (const key of ['headline', 'note']) {
      fields(entry[key], ['en', 'zh'], [], `featured ${key}`);
      for (const locale of ['en', 'zh']) text(entry[key][locale], `featured ${key}.${locale}`);
    }
    date(entry.included_on, 'featured included_on');
    requireThat(Array.isArray(entry.links), 'featured links: expected an array');
    for (const link of entry.links) {
      text(link, 'featured link', 2048);
      let url;
      try { url = new URL(link); } catch { throw new Error('featured link: invalid URL'); }
      requireThat(link.startsWith('https://hiroute.ai/') && url.origin === 'https://hiroute.ai'
        && !url.username && !url.password && !link.includes('\\'), 'featured link: must stay on https://hiroute.ai/');
    }
  }
}

// These checks complement the Rust producer's shared admission validation. Nested
// model facts and release compatibility remain owned by that production validator.
export function canonicalData(value) {
  if (value === null || typeof value !== 'object') return typeof value === 'bigint' ? value.toString() : JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalData).join(',')}]`;
  const keys = Object.keys(value).sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
  return `{${keys.map(k => `${JSON.stringify(k)}:${canonicalData(value[k])}`).join(',')}}`;
}

function structural(value, depth = 0) {
  requireThat(depth <= 64, 'JSON exceeds depth bound');
  if (typeof value === 'string') requireThat(value.isWellFormed() && Buffer.byteLength(value) <= 16 * 1024, 'string exceeds bound or is not Unicode');
  else if (Array.isArray(value)) {
    for (const entry of value) {
      if (entry && typeof entry === 'object' && !Array.isArray(entry)) requireThat(Buffer.byteLength(canonicalData(entry)) <= 256 * 1024, 'record exceeds bound');
      structural(entry, depth + 1);
    }
  } else if (value && typeof value === 'object') {
    for (const [key, entry] of Object.entries(value)) {
      requireThat(key.isWellFormed() && Buffer.byteLength(key) <= 256, 'field key exceeds bound or is not Unicode');
      structural(entry, depth + 1);
    }
  }
}

function identities(values, key, maximum) {
  requireThat(Array.isArray(values) && values.length <= maximum, `${key}: collection exceeds bound`);
  const ids = new Set();
  for (const entry of values) {
    object(entry, key);
    text(entry[key], key, 256);
    requireThat(!ids.has(entry[key]), `${key}: duplicate identity`);
    ids.add(entry[key]);
  }
  return ids;
}

function finiteWindow(condition) {
  const allowed = ['kind','timezone','days','windows','effective_from','effective_until'];
  if (condition.kind !== 'time-window' || Object.keys(condition).some(k => !allowed.includes(k))
    || !Object.hasOwn(condition,'effective_from') && !Object.hasOwn(condition,'effective_until')) return;
  const integer = value => (typeof value === 'bigint' || Number.isSafeInteger(value))
    && BigInt(value) >= -(1n << 63n) && BigInt(value) <= (1n << 63n)-1n;
  requireThat(integer(condition.effective_from) && integer(condition.effective_until)
    && BigInt(condition.effective_from) < BigInt(condition.effective_until), 'invalid price validity');
  const days = ['monday','tuesday','wednesday','thursday','friday','saturday','sunday'];
  requireThat(typeof condition.timezone === 'string' && Array.isArray(condition.days) && condition.days.length > 0
    && condition.days.length <= 7 && new Set(condition.days).size === condition.days.length
    && condition.days.every(day => days.includes(day)), 'invalid price days');
  requireThat(Array.isArray(condition.windows) && condition.windows.length > 0 && condition.windows.length <= 32,
    'invalid price windows');
  for (const window of condition.windows) {
    fields(window,['start','end'],[],'price window');
    requireThat([window.start,window.end].every(value => typeof value === 'string' && /^(?:[01][0-9]|2[0-3]):[0-5][0-9]$/.test(value))
      && window.start !== window.end, 'invalid price window boundary');
  }
}

export function checkPayload(bytes) {
  const payload = parseJson(bytes, 'payload');
  if (payload === null) return null;
  fields(payload, ['schema', 'sequence', 'issued_at', 'source_revision', 'data_revision', 'data_digest', 'data', 'featured'], [], 'payload');
  requireThat(payload.schema === 'hiroute.model-feed/v2', 'payload: unsupported schema');
  publication(payload);
  requireThat(typeof payload.source_revision === 'string' && /^[0-9a-f]{40}$/.test(payload.source_revision), 'payload: invalid source_revision');
  requireThat(typeof payload.data_revision === 'bigint' && payload.data_revision > 0n && payload.data_revision <= U64_MAX, 'invalid data revision');
  const data = payload.data;
  fields(data, ['schema','models','model_endpoint_capabilities','offers','price_rates','rating_snapshot','metadata_catalog','pricing_facts','connection_hints'], [], 'complete data');
  requireThat(data.schema === 'hiroute.official-model-data/v1', 'unsupported data schema');
  const models = identities(data.models, 'model_configuration_id', 16384);
  identities(data.model_endpoint_capabilities, 'capability_id', 16384);
  const offers = identities(data.offers, 'offer_id', 16384);
  identities(data.price_rates, 'price_rate_id', 50000);
  for (const c of data.model_endpoint_capabilities) requireThat(models.has(c.model_configuration_id), 'missing capability model');
  for (const o of data.offers) requireThat(Array.isArray(o.model_configuration_ids) && o.model_configuration_ids.length && o.model_configuration_ids.every(id => models.has(id)), 'invalid offer membership');
  for (const p of data.price_rates) requireThat(models.has(p.model_configuration_id) && offers.has(p.offer_ref), 'invalid price references');
  const metadata = data.metadata_catalog;
  requireThat(metadata.schema === 'hiroute.model-metadata-catalog/v1', 'unsupported metadata schema');
  const collections = [
    ['evidence_sources','source_key',1000], ['client_runs','source_key',64], ['access_products','product_key',1000],
    ['canonical_models','model_key',10000], ['endpoint_bindings','binding_key',50000], ['dynamic_routes','route_key',1000],
    ['inference_rules','rule_key',10000], ['provider_records','provider_record_key',10000], ['model_records','model_record_key',100000],
  ];
  const sets = new Map(collections.map(([name,key,max]) => [name, identities(metadata[name], key, max)]));
  identities(data.pricing_facts, 'pricing_key', 50000);
  identities(data.connection_hints, 'connection_option_id', 1000);
  const priceScopes = new Set();
  for (const price of data.pricing_facts) {
    const scope = JSON.stringify([price.product_key, price.model_key, price.upstream_model_id, price.currency]);
    requireThat(!priceScopes.has(scope), 'duplicate documented price scope');
    priceScopes.add(scope);
    requireThat(sets.get('access_products').has(price.product_key) && sets.get('canonical_models').has(price.model_key)
      && price.evidence_refs.every(ref => sets.get('evidence_sources').has(ref)), 'invalid documented price scope/evidence');
    identities(price.schedules, 'schedule_key', 64);
    for (const schedule of price.schedules) {
      object(schedule.condition, 'price condition'); text(schedule.condition.kind, 'price condition kind');
      finiteWindow(schedule.condition);
      object(schedule.rates, 'price rates');
      requireThat(Object.values(schedule.rates).every(rate => typeof rate === 'string' && /^\d+(?:\.\d+)?$/.test(rate) && rate.length <= 64), 'invalid decimal price');
    }
  }
  requireThat(typeof payload.data_digest === 'string' && payload.data_digest === `sha256:${createHash('sha256').update(canonicalData(data)).digest('hex')}`, 'semantic data digest mismatch');
  structural(payload);
  featuredEntries(payload.featured);
  requireThat(payload.featured.length <= 128, 'featured exceeds bound');
  return payload;
}

export function payloadDigest(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

async function readBounded(filename) {
  const file = await open(filename, 'r');
  try {
    const bytes = Buffer.alloc(MAX_FEED_BYTES + 1);
    let total = 0;
    while (total < bytes.length) {
      const { bytesRead } = await file.read(bytes, total, bytes.length - total, null);
      if (bytesRead === 0) break;
      total += bytesRead;
    }
    requireThat(total <= MAX_FEED_BYTES, 'feed input: exceeds 16 MiB');
    return bytes.subarray(0, total);
  } finally { await file.close(); }
}

export async function checkFeed({ sourceDirectory = path.join(repository, 'assets/model-feed'), payloadPath = path.join(sourceDirectory, 'payload.json') } = {}) {
  const payloadBytes = await readBounded(payloadPath);
  const payload = checkPayload(payloadBytes);
  const policy = parseJson(await readBounded(path.join(sourceDirectory, 'policy.json')), 'policy');
  fields(policy, ['minimum_feed_client'], [], 'policy');
  version(policy.minimum_feed_client, 'minimum_feed_client');
  const metadata = parseJson(await readBounded(path.join(sourceDirectory, 'publication.json')), 'publication');
  fields(metadata, ['sequence', 'issued_at'], [], 'publication');
  publication(metadata);
  const featured = parseJson(await readBounded(path.join(sourceDirectory, 'featured.json')), 'featured');
  featuredEntries(featured);
  if (payload) {
    requireThat(payload.sequence === metadata.sequence && payload.issued_at === metadata.issued_at, 'payload: publication metadata differs; regenerate the feed');
  }
  return { payloadBytes, payload, digest: payloadDigest(payloadBytes) };
}

async function servedFeed(fetchImpl) {
  let response;
  try {
    response = await fetchImpl(FEED_URL, { redirect: 'error', credentials: 'omit', cache: 'no-store',
      signal: AbortSignal.timeout(30_000), headers: { Accept: 'application/json', 'Cache-Control': 'no-cache' } });
  } catch { throw new Error('served feed: network request failed (redirects are forbidden)'); }
  if (response.status === 404) {
    await response.body?.cancel();
    return null;
  }
  requireThat(response.status === 200 && !response.redirected, `served feed: unexpected HTTP status ${response.status}`);
  const length = response.headers.get('content-length');
  requireThat(length === null || (/^\d+$/.test(length) && BigInt(length) <= BigInt(MAX_FEED_BYTES)), 'served feed: exceeds 16 MiB or invalid content length');
  requireThat(response.body, 'served feed: missing body');
  const reader = response.body.getReader();
  const chunks = [];
  let total = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.length;
      requireThat(total <= MAX_FEED_BYTES, 'served feed: exceeds 16 MiB');
      chunks.push(value);
    }
  } finally { await reader.cancel(); }
  const bytes = Buffer.concat(chunks, total);
  const payload = checkPayload(bytes);
  requireThat(payload !== null, 'served feed: bootstrap payload cannot be published');
  return { payload, digest: payloadDigest(bytes) };
}

async function assertMissing(filename) {
  try { await lstat(filename); }
  catch (error) { if (error.code === 'ENOENT') return; throw error; }
  throw new Error('publication: dist already contains a feed; use the validated website build artifact');
}

function resultFor(status, checked) {
  return { status, sequence: checked.payload?.sequence.toString() ?? null,
    data_revision: checked.payload?.data_revision.toString() ?? null, data_digest: checked.payload?.data_digest ?? null,
    payload_digest: checked.payload ? checked.digest : null, source_revision: checked.payload?.source_revision ?? null };
}

export async function publishFeed({ sourceDirectory, payloadPath, distDirectory = path.join(repository, 'apps/website/dist'),
  env = process.env, fetchImpl = fetch } = {}) {
  requireThat(env.GITHUB_EVENT_NAME === 'push' || env.GITHUB_EVENT_NAME === 'workflow_dispatch', 'publication: only a production push or dispatch may publish');
  requireThat(env.GITHUB_REF === 'refs/heads/main', 'publication: only main may publish');
  const destination = path.join(distDirectory, 'model-feed/v2.json');
  await assertMissing(destination);
  const checked = await checkFeed({ sourceDirectory, payloadPath });
  let result;
  let outputBytes;
  if (!checked.payload) result = resultFor('bootstrap', checked);
  else {
    const served = await servedFeed(fetchImpl);
    if (served?.digest === checked.digest) result = resultFor('already_current', checked);
    else {
      requireThat(!served || checked.payload.sequence > served.payload.sequence,
        'publication: proposed sequence must exceed the served sequence when payload content differs');
      requireThat(!served || checked.payload.data_revision > served.payload.data_revision
        || (checked.payload.data_revision === served.payload.data_revision && checked.payload.data_digest === served.payload.data_digest),
        'publication: semantic data revision cannot roll back or change content at the same revision');
      outputBytes = checked.payloadBytes;
      result = resultFor(served ? 'published' : 'first_publication', checked);
    }
  }
  if (env.GITHUB_STEP_SUMMARY) {
    await appendFile(env.GITHUB_STEP_SUMMARY, `\n### Model feed\n\n- Status: ${result.status}\n- Sequence: ${result.sequence ?? 'none'}\n- Data revision: ${result.data_revision ?? 'none'}\n- Data digest: ${result.data_digest ?? 'none'}\n- Payload SHA-256: ${result.payload_digest ?? 'none'}\n- Source revision: ${result.source_revision ?? 'none'}\n`);
  }
  if (outputBytes) {
    await mkdir(path.dirname(destination), { recursive: true });
    const temporary = `${destination}.${process.pid}.tmp`;
    try {
      const file = await open(temporary, 'wx', 0o644);
      try { await file.writeFile(outputBytes); await file.sync(); } finally { await file.close(); }
      await rename(temporary, destination);
    } finally { await rm(temporary, { force: true }); }
  }
  return result;
}

async function main() {
  const [command, ...args] = process.argv.slice(2);
  requireThat(['check', 'publish'].includes(command), 'usage: model-feed.mjs check|publish [--payload PATH]');
  requireThat(args.length === 0 || (args.length === 2 && args[0] === '--payload'), 'usage: model-feed.mjs check|publish [--payload PATH]');
  const options = args.length ? { payloadPath: path.resolve(args[1]) } : {};
  const result = command === 'publish' ? await publishFeed(options)
    : resultFor('checked', await checkFeed(options));
  if (command === 'check' && result.sequence === null) result.status = 'bootstrap';
  console.log(JSON.stringify(result));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(`Model feed: ${error.message}`); process.exitCode = 1; });
}
