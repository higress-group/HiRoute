import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { posts, newsRoot, renderNews, newsRoute } from '../src/lib/news.mjs';

test('every indexed article renders both languages with real assets and public evidence links', async () => {
  assert.ok(posts.length > 0);
  for (const post of posts) for (const language of ['zh', 'en']) {
    const html = await renderNews(post.slug, language);
    assert.ok(html.length > 0);
    assert.doesNotMatch(html, /href="\.\.\/|src="assets\//);
    for (const match of html.matchAll(/src="\/news-assets\/([^"]+)"/g)) await fs.access(path.join(newsRoot, 'assets', match[1]));
    if (language === 'en') assert.doesNotMatch(html, /[\u3400-\u9fff]/u);
    assert.equal(newsRoute(post.slug, language), `${language === 'en' ? '/en' : ''}/news/${post.slug}/`);
  }
});

test('news rejects unknown paths and unsupported language instead of filesystem traversal', async () => {
  await assert.rejects(renderNews('../../README', 'en'));
  await assert.rejects(renderNews(posts[0].slug, '../zh'));
});

test('routing case keeps numerical evidence and the public reproduction link', async () => {
  for (const language of ['zh', 'en']) {
    const html = await renderNews('astra-qwen-smart-routing', language);
    for (const metric of [/92\.01%/, /91\.39%/, /343/]) assert.match(html, metric);
    assert.match(html, /github\.com\/higress-group\/HiRoute\/blob\/main\/experiments\//);
  }
});

test('the backfilled open-source introduction leaves dated news ahead of it', () => {
  const introduction = posts.findIndex(post => post.slug === 'hiroute-open-source');
  const caseStudy = posts.findIndex(post => post.slug === 'astra-qwen-smart-routing');
  assert.ok(caseStudy >= 0 && introduction > caseStudy);
  assert.equal(posts[introduction].date, undefined);
  assert.ok(posts[0].date, 'the homepage latest-article entry must remain dated news');
});
