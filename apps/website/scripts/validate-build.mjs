import fs from 'node:fs';
import path from 'node:path';
import { LEGACY_CHINESE_PATHS } from '../src/lib/locale.mjs';
import { guidePages, guideRoute } from '../src/lib/user-guides.mjs';
import { validateAgentGuidePage } from '../src/lib/agent-guide-contract.mjs';

const root = path.resolve(import.meta.dirname, '..');
const dist = path.join(root, 'dist');
const site = 'https://hiroute.ai';
// Every page lives under its language prefix in both languages. The root and the
// former unprefixed Chinese URLs are redirect pages.
const localized = [
  '', 'news/', 'news/astra-qwen-smart-routing/', 'news/hiroute-open-source/', 'download/', 'changelog/',
  'docs/', 'docs/install-macos/', 'docs/install-linux/', 'docs/install-windows/', 'docs/models/',
  'docs/model-trial/', 'docs/model-routing/', 'docs/task-routing/', 'docs/cli/',
  'docs/decision-extensions/', 'docs/decision-api/', 'docs/jev-decider/',
];
const redirects = new Map([['/', '/en/'], ...LEGACY_CHINESE_PATHS.map(route => [route, `/zh${route}`])]);
const output = route => `${route.slice(1)}index.html`;
const required = [
  ...localized.flatMap(route => [`zh/${route}index.html`, `en/${route}index.html`]),
  ...[...redirects.keys()].map(output),
  'api/decision.openapi.json', 'api/decision-examples.json',
  'install.sh', 'install/standalone.py', 'releases.json', 'releases-v3.json', '404.html', 'sitemap-index.xml',
];
for (const relative of required) {
  if (!fs.existsSync(path.join(dist, relative))) throw new Error(`missing website output: ${relative}`);
}
for (const slug of Object.keys(guidePages)) for (const language of ['zh', 'en']) {
  const route = guideRoute(slug, language);
  validateAgentGuidePage(fs.readFileSync(path.join(dist, output(route)), 'utf8'), route);
}
// Baidu re-checks its site verification on the homepage, so losing it unverifies the site.
if (!fs.readFileSync(path.join(dist, 'index.html'), 'utf8').includes('<meta name="baidu-site-verification" content="codeva-4Bp3IgU4gn">')) {
  throw new Error('missing Baidu site verification on the homepage');
}
for (const [page, language] of [['zh/index.html', 'zh-CN'], ['en/index.html', 'en']]) {
  const html = fs.readFileSync(path.join(dist, page), 'utf8');
  const performance = html.match(/<section\b[^>]*id="performance"[^>]*>[\s\S]*?<\/section>/)?.[0];
  for (const capture of ['readme-session', 'readme-report']) {
    if (!performance?.includes(`src="/decision-assets/${capture}-${language}.png"`)) {
      throw new Error(`missing localized ${capture} capture: ${page}`);
    }
  }
  const caption = language === 'en' ? 'A real session: primary steps in after a low score' : '真实会话：低分后主力接手';
  if (!performance.includes(caption)) throw new Error(`missing session context: ${page}`);
}
const htmlFiles = [];
for (const entry of fs.readdirSync(dist, { recursive: true, withFileTypes: true })) {
  if (entry.isFile() && entry.name.endsWith('.html')) htmlFiles.push(path.join(entry.parentPath, entry.name));
}
const route = file => `/${path.relative(dist, file).split(path.sep).join('/').replace(/index\.html$/, '')}`;
const languageOf = file => /^\/(zh|en)\//.exec(route(file))?.[1] ?? (route(file) === '/404.html' ? 'en' : undefined);
// A redirect page refreshes immediately to its canonical destination, after language.js
// has had the chance to keep the query and fragment or open a saved Chinese choice.
const redirectFiles = new Set();
for (const file of htmlFiles) {
  const html = fs.readFileSync(file, 'utf8');
  const from = route(file);
  const target = html.match(/<html\b[^>]*\bdata-redirect="([^"]+)"/)?.[1];
  const refresh = html.match(/<meta http-equiv="refresh" content="0; url=([^"]+)">/)?.[1];
  if (!redirects.has(from) && !target && !refresh) {
    if (!languageOf(file)) throw new Error(`page outside a language prefix: ${from}`);
    continue;
  }
  redirectFiles.add(file);
  const script = html.indexOf('<script src="/scripts/language.js">');
  const chinese = html.match(/\bdata-redirect-zh="([^"]+)"/)?.[1];
  if (target !== redirects.get(from) || refresh !== target || chinese !== (from === '/' ? '/zh/' : undefined)
    || html.match(/<link rel="canonical" href="([^"]+)"/)?.[1] !== site + target
    || script < 0 || script > html.indexOf('http-equiv="refresh"')) throw new Error(`invalid redirect page: ${from}`);
}
for (const file of htmlFiles) {
  const html = fs.readFileSync(file, 'utf8');
  const notFound = path.relative(dist, file) === '404.html';
  if ((html.includes('noindex') && !notFound) || html.includes('官网视觉原型')) throw new Error(`prototype marker leaked into ${file}`);
  for (const [, target] of html.matchAll(/href="(\/[^"?#]*)/g)) {
    if (redirects.has(target)) throw new Error(`link to a redirect page in ${file}: ${target}`);
  }
  for (const match of html.matchAll(/(?:href|src)="(\/[^"?#]+)["?#]/g)) {
    const target = decodeURIComponent(match[1]);
    if (target.startsWith('/releases/')) continue;
    const candidate = target.endsWith('/') ? path.join(dist, target, 'index.html') : path.join(dist, target);
    if (!fs.existsSync(candidate)) throw new Error(`broken local reference in ${file}: ${target}`);
  }
}

for (const file of htmlFiles.filter(file => languageOf(file) === 'en')) {
  // Canonical protocol payloads are intentionally identical in both languages.
  // Validate the surrounding prose, not localized user input inside JSON samples.
  const source = fs.readFileSync(file, 'utf8');
  const prose = path.relative(dist, file) === path.join('en', 'docs', 'decision-api', 'index.html')
    ? source.replace(/<pre><code class="language-json">[\s\S]*?<\/code><\/pre>/gi, '') : source;
  const html = prose.replaceAll('简体中文', '').replaceAll('中文', '');
  if (/[\u3400-\u9fff]/u.test(html)) throw new Error(`Chinese copy leaked into English page: ${file}`);
}

const untranslatedChinesePageCopy = [
  'Intelligent routing for long-horizon agents.',
  'ONE TASK. MANY DECISIONS.',
  'Powered by TypeSafe Jev',
  'Evidence for the next decision',
  'From evidence to better delegation',
  'Agent ecosystem',
  'Your policy, one interface',
  'Start with HiRoute',
  'Questions',
  'CHANGELOG',
  'DOWNLOAD',
];
for (const file of htmlFiles.filter(file => languageOf(file) === 'zh')) {
  // Sharing metadata may intentionally use English; visible page copy stays localized.
  const html = fs.readFileSync(file, 'utf8').replace(/<head\b[^>]*>[\s\S]*?<\/head>/i, '');
  const leaked = untranslatedChinesePageCopy.find(value => html.includes(value));
  if (leaked) throw new Error(`English copy leaked into Chinese page ${file}: ${leaked}`);
}
// Search metadata: each public page has its own title and snippet, a self canonical,
// and the zh/en pair of its counterpart with English as x-default; the sitemap agrees.
const languageLinks = (source, pattern, where) => {
  const pairs = [...source.matchAll(pattern)].map(match => match.slice(1));
  const links = Object.fromEntries(pairs);
  if (Object.keys(links).length !== pairs.length) throw new Error(`repeated hreflang in ${where}`);
  return links;
};
const pages = new Map();
for (const file of htmlFiles) {
  const relative = path.relative(dist, file);
  if (relative === '404.html' || redirectFiles.has(file)) continue;
  const head = fs.readFileSync(file, 'utf8').match(/<head\b[^>]*>([\s\S]*?)<\/head>/i)?.[1] ?? '';
  const url = `${site}/${relative.split(path.sep).join('/').replace(/index\.html$/, '')}`;
  pages.set(url, {
    relative,
    title: head.match(/<title>([^<]+)<\/title>/)?.[1],
    description: head.match(/<meta name="description" content="([^"]+)"/)?.[1],
    canonical: head.match(/<link rel="canonical" href="([^"]+)"/)?.[1],
    alternates: languageLinks(head, /<link rel="alternate" hreflang="([^"]+)" href="([^"]+)"/g, relative),
    structuredData: [...head.matchAll(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/g)].map(match => match[1]),
  });
}
for (const field of ['title', 'description']) {
  const seen = new Map();
  for (const page of pages.values()) {
    if (!page[field]) throw new Error(`missing ${field}: ${page.relative}`);
    if (seen.has(page[field])) throw new Error(`duplicate ${field}: ${seen.get(page[field])} and ${page.relative}`);
    seen.set(page[field], page.relative);
  }
}
for (const [url, page] of pages) {
  const { zh, en, 'x-default': fallback, ...unexpected } = page.alternates;
  if (page.canonical !== url) throw new Error(`canonical differs from the page URL: ${page.relative}`);
  if (![zh, en].includes(url) || fallback !== en || Object.keys(unexpected).length) throw new Error(`incomplete language alternates: ${page.relative}`);
  for (const counterpart of [zh, en].map(alternate => pages.get(alternate))) {
    if (counterpart?.alternates.zh !== zh || counterpart.alternates.en !== en) throw new Error(`language alternates are not reciprocal: ${page.relative}`);
  }
  for (const data of page.structuredData) {
    if (JSON.parse(data)['@context'] !== 'https://schema.org') throw new Error(`structured data lacks the schema.org context: ${page.relative}`);
  }
}
const listed = new Set();
for (const name of fs.readdirSync(dist).filter(name => /^sitemap-\d+\.xml$/.test(name))) {
  for (const [, entry] of fs.readFileSync(path.join(dist, name), 'utf8').matchAll(/<url>([\s\S]*?)<\/url>/g)) {
    const url = entry.match(/<loc>([^<]+)<\/loc>/)?.[1];
    const page = pages.get(url);
    const links = languageLinks(entry, /hreflang="([^"]+)" href="([^"]+)"/g, `sitemap ${url}`);
    if (!page) throw new Error(`sitemap lists a page without search metadata: ${url}`);
    if (JSON.stringify(Object.entries(links).sort()) !== JSON.stringify(Object.entries(page.alternates).sort())) throw new Error(`sitemap alternates differ from the page: ${url}`);
    const lastmod = entry.match(/<lastmod>([^<]+)<\/lastmod>/)?.[1];
    if (lastmod !== undefined && Number.isNaN(Date.parse(lastmod))) throw new Error(`invalid sitemap lastmod for ${url}: ${lastmod}`);
    listed.add(url);
  }
}
const unlisted = [...pages.keys()].filter(url => !listed.has(url));
if (unlisted.length) throw new Error(`pages missing from the sitemap: ${unlisted.join(', ')}`);
console.log(`Validated ${htmlFiles.length} static HTML pages, including ${redirectFiles.size} redirect pages, and ${required.length} required outputs.`);
