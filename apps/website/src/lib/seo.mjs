import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { decisionPages, decisionSources } from './decision-docs.mjs';
import { localePath } from './locale.mjs';
import { posts } from './news.mjs';
import { guidePages } from './user-guides.mjs';

export const SITE = 'https://hiroute.ai';
export const GITHUB_URL = 'https://github.com/higress-group/HiRoute';
const website = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const repository = path.resolve(website, '../..');
const absolute = route => new URL(route, SITE).href;
const organization = {
  '@type': 'Organization', name: 'HiRoute', url: absolute('/'),
  logo: { '@type': 'ImageObject', url: absolute('/brand/app-icon.png') },
};

// JSON-LD is embedded in a script element, so no "<" may close it early.
export const jsonLd = data => JSON.stringify(data).replace(/</g, '\\u003c');

export function breadcrumbs(items) {
  return {
    '@context': 'https://schema.org', '@type': 'BreadcrumbList',
    itemListElement: items.map(([name, route], index) => ({ '@type': 'ListItem', position: index + 1, name, item: absolute(route) })),
  };
}

export function homeStructuredData({ language, description, operatingSystems }) {
  const english = language === 'en';
  return {
    '@context': 'https://schema.org',
    '@graph': [
      { '@type': 'WebSite', '@id': absolute('/#website'), name: 'HiRoute', url: absolute('/'), inLanguage: ['zh-CN', 'en'], publisher: { '@id': absolute('/#organization') } },
      { ...organization, '@id': absolute('/#organization') },
      {
        '@type': 'SoftwareApplication', '@id': absolute('/#software'), name: 'HiRoute', description,
        url: absolute(localePath(language)), downloadUrl: absolute(localePath(language, '/download/')),
        applicationCategory: 'DeveloperApplication',
        ...(operatingSystems.length ? { operatingSystem: operatingSystems.join(', ') } : {}),
        image: absolute(`/brand/social-card-${english ? 'en' : 'zh'}.png`),
        license: 'https://www.apache.org/licenses/LICENSE-2.0', isAccessibleForFree: true,
        offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
        publisher: { '@id': absolute('/#organization') }, sameAs: [GITHUB_URL],
      },
    ],
  };
}

export function articleStructuredData({ title, description, language, route, published, modified, image }) {
  return {
    '@context': 'https://schema.org', '@type': 'BlogPosting', headline: title, description,
    inLanguage: language === 'en' ? 'en' : 'zh-CN', url: absolute(route), mainEntityOfPage: absolute(route),
    image: [absolute(image)],
    ...(published ? { datePublished: published } : {}), ...(modified ? { dateModified: modified } : {}),
    author: { '@type': 'Organization', name: 'HiRoute', url: absolute('/') }, publisher: organization,
  };
}

// The first raster illustration represents an article; otherwise its language's social card does.
export function articleImage(html, language) {
  return html.match(/<img\b[^>]*\bsrc="(\/[^"]+\.(?:png|jpe?g|webp))"/i)?.[1] ?? `/brand/social-card-${language === 'en' ? 'en' : 'zh'}.png`;
}

// Repository files whose committed changes alter a page's main content. Shared layout
// changes are deliberately excluded so that sitemap lastmod tracks real content updates.
// Unprefixed URLs are redirect pages without content of their own.
export function pageSources(pathname) {
  const [, language, route] = pathname.match(/^\/(en|zh)(\/.*)$/) ?? [];
  if (!language) return [];
  const app = file => `apps/website/${file}`;
  if (route === '/') return [app('src/components/HomePage.astro'), 'news/index.json', app('data/releases.json')];
  if (route === '/download/') return [app('src/components/DownloadPage.astro'), app('data/releases.json'), app('data/short-downloads.json')];
  if (route === '/changelog/') return [app('src/components/ChangelogPage.astro'), app('data/releases.json')];
  if (route === '/news/') return ['news/index.json'];
  if (route === '/docs/') return [app(`content/guides/quickstart.${language}.md`)];
  const [, section, slug] = route.match(/^\/(news|docs)\/([^/]+)\/$/) ?? [];
  if (section === 'news') {
    const post = posts.find(item => item.slug === slug);
    return post ? [`news/${post.files[language]}`] : [];
  }
  if (section === 'docs') {
    if (slug in guidePages) return [app(`content/guides/${slug}.${language}.md`)];
    const page = Object.keys(decisionPages).find(key => decisionPages[key].slug === slug);
    if (page) return [decisionSources[page][language]];
  }
  return [];
}

let fullHistory;
const commitTimes = new Map();
// Commit time of the newest change to any of the files, or undefined when it cannot be
// known: a shallow clone would report its single commit for every file.
export function lastCommitTime(files) {
  if (!files.length) return undefined;
  fullHistory ??= git(['rev-parse', '--is-shallow-repository']) === 'false';
  if (!fullHistory) return undefined;
  const key = files.join('\n');
  if (!commitTimes.has(key)) commitTimes.set(key, git(['log', '-1', '--format=%cI', '--', ...files]));
  return commitTimes.get(key);
}

function git(args) {
  try {
    return execFileSync('git', args, { cwd: repository, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim() || undefined;
  } catch {
    return undefined;
  }
}
