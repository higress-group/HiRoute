import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { marked } from 'marked';

const website = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const repository = path.resolve(website, '../..');
const sources = {
  mechanism: { zh: 'decision-extensions/README.zh-CN.md', en: 'decision-extensions/README.md' },
  api: { zh: 'decision-extensions/api/README.zh-CN.md', en: 'decision-extensions/api/README.md' },
  jev: { zh: 'decision-extensions/extensions/jev-decider/README.zh-CN.md', en: 'decision-extensions/extensions/jev-decider/README.md' },
};

const route = (language, page) => `${language === 'en' ? '/en' : ''}/docs/${page}/`;
const maps = {
  mechanism: {
    'README.md': '/en/docs/decision-extensions/', 'README.zh-CN.md': '/docs/decision-extensions/',
    'api/README.md': '/en/docs/decision-api/', 'api/README.zh-CN.md': '/docs/decision-api/',
    'extensions/jev-decider/README.md': '/en/docs/jev-decider/',
    'extensions/jev-decider/README.zh-CN.md': '/docs/jev-decider/',
  },
  api: {
    'README.md': '/en/docs/decision-api/', 'README.zh-CN.md': '/docs/decision-api/',
    '../README.md': '/en/docs/decision-extensions/', '../README.zh-CN.md': '/docs/decision-extensions/',
    'decision.openapi.json': '/api/decision.openapi.json',
  },
  jev: {
    'README.md': '/en/docs/jev-decider/', 'README.zh-CN.md': '/docs/jev-decider/',
    '../../README.md': '/en/docs/decision-extensions/', '../../README.zh-CN.md': '/docs/decision-extensions/',
    '../../api/README.md': '/en/docs/decision-api/', '../../api/README.zh-CN.md': '/docs/decision-api/',
  },
};

function rewriteTarget(target, page, language) {
  if (/^(?:https?:|mailto:|#)/.test(target)) return target;
  if (target.startsWith('assets/') && target.endsWith('.png')) return `/decision-assets/${path.basename(target)}`;
  if (target === '../../docs/smart-saving-model-classification.md'
      || target === '../../docs/smart-saving-model-classification.zh-CN.md') {
    return route(language, 'model-routing');
  }
  const mapped = maps[page][target];
  if (!mapped) throw new Error(`unmapped relative decision-document link in ${page}: ${target}`);
  return mapped;
}

export async function renderDecisionDoc(page, language) {
  if (!(page in sources) || !['zh', 'en'].includes(language)) throw new Error('unknown decision documentation page');
  const markdown = await fs.readFile(path.join(repository, sources[page][language]), 'utf8');
  const renderer = new marked.Renderer();
  renderer.link = ({ href, title, text }) => {
    const target = rewriteTarget(href, page, language);
    const external = /^https?:/.test(target) ? ' target="_blank" rel="noreferrer"' : '';
    const titleAttribute = title ? ` title="${title.replaceAll('"', '&quot;')}"` : '';
    return `<a href="${target}"${titleAttribute}${external}>${text}</a>`;
  };
  renderer.image = ({ href, title, text }) => {
    const target = rewriteTarget(href, page, language);
    const titleAttribute = title ? ` title="${title.replaceAll('"', '&quot;')}"` : '';
    return `<img src="${target}" alt="${text.replaceAll('"', '&quot;')}"${titleAttribute} loading="lazy">`;
  };
  return marked.parse(markdown, { gfm: true, renderer });
}

export const decisionDocRoutes = { route };
