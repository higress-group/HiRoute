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
const sourceLink = file => `https://github.com/higress-group/HiRoute/blob/main/${file}`;
const maps = {
  mechanism: {
    'README.md': '/en/docs/decision-extensions/', 'README.zh-CN.md': '/docs/decision-extensions/',
    'api/README.md': '/en/docs/decision-api/', 'api/README.zh-CN.md': '/docs/decision-api/',
    'api/decision.openapi.json': '/api/decision.openapi.json',
    'api/decision-examples.json': '/api/decision-examples.json',
    'api/decision-design.md': sourceLink('decision-extensions/api/decision-design.md'),
    'api/system-one-design.md': sourceLink('decision-extensions/api/system-one-design.md'),
    'extensions/jev-decider/README.md': '/en/docs/jev-decider/',
    'extensions/jev-decider/README.zh-CN.md': '/docs/jev-decider/',
  },
  api: {
    'README.md': '/en/docs/decision-api/', 'README.zh-CN.md': '/docs/decision-api/',
    '../README.md': '/en/docs/decision-extensions/', '../README.zh-CN.md': '/docs/decision-extensions/',
    '../extensions/jev-decider/README.md': '/en/docs/jev-decider/',
    '../extensions/jev-decider/README.zh-CN.md': '/docs/jev-decider/',
    'decision.openapi.json': '/api/decision.openapi.json',
    'decision-examples.json': '/api/decision-examples.json',
    'decision-design.md': sourceLink('decision-extensions/api/decision-design.md'),
    'system-one-design.md': sourceLink('decision-extensions/api/system-one-design.md'),
  },
  jev: {
    'jev_decider/settings.py': sourceLink('decision-extensions/extensions/jev-decider/jev_decider/settings.py'),
    'jev_decider/protocol.py': sourceLink('decision-extensions/extensions/jev-decider/jev_decider/protocol.py'),
    'jev_decider/decision.py': sourceLink('decision-extensions/extensions/jev-decider/jev_decider/decision.py'),
    'jev_decider/server.py': sourceLink('decision-extensions/extensions/jev-decider/jev_decider/server.py'),
    'tests/README.md': sourceLink('decision-extensions/extensions/jev-decider/tests/README.md'),
    '../../../docs/code-map/decision-foundation.md': sourceLink('docs/code-map/decision-foundation.md'),
    '../../../crates/gateway/README.md': sourceLink('crates/gateway/README.md'),
    'README.md': '/en/docs/jev-decider/', 'README.zh-CN.md': '/docs/jev-decider/',
    '../../README.md': '/en/docs/decision-extensions/', '../../README.zh-CN.md': '/docs/decision-extensions/',
    '../../api/README.md': '/en/docs/decision-api/', '../../api/README.zh-CN.md': '/docs/decision-api/',
    '../../api/decision-design.md': sourceLink('decision-extensions/api/decision-design.md'),
    '../../api/system-one-design.md': sourceLink('decision-extensions/api/system-one-design.md'),
  },
};

function rewriteTarget(target, page, language) {
  if (/^(?:https?:|mailto:|#)/.test(target)) return target;
  if (target.startsWith('assets/') && target.endsWith('.png')) return `/decision-assets/${path.basename(target)}`;
  if (target === '../docs/smart-saving-model-classification.md'
      || target === '../docs/smart-saving-model-classification.zh-CN.md'
      || target === '../../docs/smart-saving-model-classification.md'
      || target === '../../docs/smart-saving-model-classification.zh-CN.md') {
    return route(language, 'model-routing');
  }
  const [file, fragment] = target.split('#', 2);
  const mapped = maps[page][file];
  if (!mapped) throw new Error(`unmapped relative decision-document link in ${page}: ${target}`);
  return mapped + (fragment === undefined ? '' : `#${fragment}`);
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
