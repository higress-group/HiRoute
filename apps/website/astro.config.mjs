import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import { LEGACY_CHINESE_PATHS } from './src/lib/locale.mjs';
import { lastCommitTime, pageSources } from './src/lib/seo.mjs';

const redirects = new Set(['/', ...LEGACY_CHINESE_PATHS]);

export default defineConfig({
  site: 'https://hiroute.ai',
  output: 'static',
  trailingSlash: 'always',
  integrations: [sitemap({
    // Redirect pages stay out of the sitemap, and each listed page has a zh/en pair.
    filter: page => !redirects.has(new URL(page).pathname),
    i18n: {
      defaultLocale: 'en',
      locales: { en: 'en', zh: 'zh' },
    },
    // Match each page's head: English is x-default, and lastmod is the newest commit
    // to the page's own sources, omitted when that time is unknown. A language pair
    // shares one links array, so replace it rather than appending in place.
    serialize(item) {
      const english = item.links?.find(link => link.lang === 'en');
      if (english) item.links = [...item.links.filter(link => link.lang !== 'x-default'), { lang: 'x-default', url: english.url }];
      const lastmod = lastCommitTime(pageSources(new URL(item.url).pathname));
      if (lastmod) item.lastmod = lastmod;
      return item;
    },
  })],
});
