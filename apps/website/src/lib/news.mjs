import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { marked } from 'marked';

export const newsRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../news');
export const posts = JSON.parse(await fs.readFile(path.join(newsRoot, 'index.json'), 'utf8'));
const seen = new Set();
for (const post of posts) {
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(post.slug) || seen.has(post.slug)) throw Error('invalid or duplicate news slug');
  seen.add(post.slug);
  if (post.date !== undefined && !/^\d{4}-\d{2}-\d{2}$/.test(post.date)) throw Error('invalid news date');
  for (const language of ['zh', 'en']) {
    if (!post.title[language] || !post.description[language] || !/^[A-Za-z0-9.-]+\.md$/.test(post.files[language])) throw Error('incomplete news translation');
  }
}
// Undated backfills follow dated news and do not replace the latest homepage article.
// Same-day news keeps its index order, so the index decides which one the homepage features.
posts.sort((a, b) => (b.date ?? '').localeCompare(a.date ?? ''));
export const newsRoute = (slug, language) => `${language === 'en' ? '/en' : ''}/news/${slug ? slug + '/' : ''}`;

export function articleLink(href) {
  if (href.startsWith('assets/')) return '/news-assets/' + href.slice(7);
  if (href.startsWith('../experiments/') || href.startsWith('../decision-extensions/')) {
    return 'https://github.com/higress-group/HiRoute/blob/main/' + href.slice(3);
  }
  return href;
}

// Images shared with the READMEs are published under /decision-assets/; links keep pointing to GitHub.
export function articleImage(href) {
  if (href.startsWith('../decision-extensions/assets/')) return '/decision-assets/' + href.slice('../decision-extensions/assets/'.length);
  return articleLink(href);
}

export async function renderNews(slug, language) {
  const post = posts.find(p => p.slug === slug);
  if (!post || !['zh', 'en'].includes(language)) throw Error('unknown article or language');
  let source = await fs.readFile(path.join(newsRoot, post.files[language]), 'utf8');
  if (!source.startsWith('# ' + post.title[language] + '\n')) throw Error('article title differs from index');
  source = source.slice(source.indexOf('\n') + 1);
  const renderer = new marked.Renderer();
  renderer.link = function(token) { return marked.Renderer.prototype.link.call(this, { ...token, href: articleLink(token.href) }); };
  renderer.image = function(token) { return marked.Renderer.prototype.image.call(this, { ...token, href: articleImage(token.href) }); };
  return marked.parse(source, { gfm: true, renderer });
}
