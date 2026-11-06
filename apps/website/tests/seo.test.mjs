import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import { decisionPages } from '../src/lib/decision-docs.mjs';
import { LEGACY_CHINESE_PATHS, localePath } from '../src/lib/locale.mjs';
import { posts } from '../src/lib/news.mjs';
import { articleImage, jsonLd, lastCommitTime, pageSources } from '../src/lib/seo.mjs';
import { guidePages, guideRoute } from '../src/lib/user-guides.mjs';

const repository = new URL('../../../', import.meta.url);
// Every page is published under both language prefixes.
const routes = ['/', '/download/', '/changelog/', '/news/',
  ...posts.map(post => `/news/${post.slug}/`),
  ...Object.keys(guidePages).map(guide => guideRoute(guide, 'zh').replace(/^\/zh/, '')),
  ...Object.values(decisionPages).map(page => `/docs/${page.slug}/`)];

test('structured data cannot close its script element', () => {
  const html = jsonLd({ headline: '</script><script>alert(1)</script>' });
  assert.doesNotMatch(html, /</);
  assert.equal(JSON.parse(html).headline, '</script><script>alert(1)</script>');
});

test('every published page maps to its own existing content sources', () => {
  for (const route of routes) {
    for (const pathname of [localePath('zh', route), localePath('en', route)]) {
      const sources = pageSources(pathname);
      assert.ok(sources.length, `${pathname} has content sources`);
      for (const file of sources) assert.ok(fs.existsSync(new URL(file, repository)), `${pathname} source exists: ${file}`);
    }
  }
  assert.deepEqual(pageSources('/en/docs/cli/'), ['apps/website/content/guides/cli.en.md']);
  assert.deepEqual(pageSources('/zh/docs/decision-api/'), ['decision-extensions/api/README.zh-CN.md']);
  // The root and the former Chinese URLs are redirect pages without content of their own.
  for (const pathname of ['/', '/docs/decision-api/', '/zh/unknown/']) assert.deepEqual(pageSources(pathname), []);
  assert.equal(lastCommitTime([]), undefined);
});

test('every former Chinese URL redirects to a current Chinese page', () => {
  assert.equal(new Set(LEGACY_CHINESE_PATHS).size, LEGACY_CHINESE_PATHS.length);
  for (const path of LEGACY_CHINESE_PATHS) assert.ok(routes.includes(path) && path !== '/', `${path} has a Chinese page`);
});

test('articles use their first raster illustration or the localized social card', () => {
  assert.equal(articleImage('<p><img src="/news-assets/a.svg"><img alt="" src="/news-assets/b.png"></p>', 'zh'), '/news-assets/b.png');
  assert.equal(articleImage('<p>No image</p>', 'en'), '/brand/social-card-en.png');
});
