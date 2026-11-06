import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { marked } from 'marked';
import { localePath } from './locale.mjs';

const website = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const repository = path.resolve(website, '../..');
export const decisionSources = {
  mechanism: { zh: 'decision-extensions/README.zh-CN.md', en: 'decision-extensions/README.md' },
  api: { zh: 'decision-extensions/api/README.zh-CN.md', en: 'decision-extensions/api/README.md' },
  jev: { zh: 'decision-extensions/extensions/jev-decider/README.zh-CN.md', en: 'decision-extensions/extensions/jev-decider/README.md' },
};

// Website route, page name, and search-result title and snippet for each canonical document.
export const decisionPages = {
  mechanism: {
    slug: 'decision-extensions', zh: '决策模型与路由机制', en: 'Decision models and routing mechanism',
    title: { zh: '决策模型与路由机制：每轮判断难度，评价上一阶段', en: 'Decision models and routing: judge each turn, assess the last stage' },
    description: {
      zh: '决策模型在每条新用户消息时判断任务类别和难度，并评价上一阶段的执行表现；HiRoute 按你的路由计划选择经济模型或主力模型。',
      en: 'A decision model judges the category and difficulty of each new user message and assesses the previous stage; HiRoute applies your routing plan to pick an economical or primary model.',
    },
  },
  api: {
    slug: 'decision-api', zh: '自定义扩展 API', en: 'Custom extension API',
    title: { zh: '自定义扩展 API：接入你自己的决策服务', en: 'Custom extension API: plug in your own decision service' },
    description: {
      zh: '部署自己的 HTTP 决策服务：HiRoute 在每个新用户轮次发送已发布的类别、条件、可用模型组和安全历史，扩展返回任务类别和常规或主力组。附 OpenAPI 与完整示例。',
      en: 'Deploy your own HTTP decision service. Each new user turn, HiRoute sends published categories, conditions, groups and safe history; the extension returns a category and group. Includes OpenAPI.',
    },
  },
  jev: {
    slug: 'jev-decider', zh: 'Jev 参考扩展', en: 'Jev reference extension',
    title: { zh: '官方 Jev 决策扩展：可自行部署的参考实现', en: 'Official Jev decision extension: a self-hosted reference' },
    description: {
      zh: 'Jev 扩展是可自行部署的 Python HTTP 决策服务，每次决策调用一次兼容 System One API 的供应商；只使用内置决策连接时无需部署。',
      en: 'A self-hosted Python HTTP decision service that makes one System One provider call per decision. You only need it when you choose a custom extension over a built-in connection.',
    },
  },
};

const route = (language, page) => localePath(language, `/docs/${page}/`);
const sourceLink = file => `https://github.com/higress-group/HiRoute/blob/main/${file}`;
const maps = {
  mechanism: {
    'README.md': '/en/docs/decision-extensions/', 'README.zh-CN.md': '/zh/docs/decision-extensions/',
    'api/README.md': '/en/docs/decision-api/', 'api/README.zh-CN.md': '/zh/docs/decision-api/',
    'api/decision.openapi.json': '/api/decision.openapi.json',
    'api/decision-examples.json': '/api/decision-examples.json',
    'api/decision-design.md': sourceLink('decision-extensions/api/decision-design.md'),
    'api/system-one-design.md': sourceLink('decision-extensions/api/system-one-design.md'),
    'extensions/jev-decider/README.md': '/en/docs/jev-decider/',
    'extensions/jev-decider/README.zh-CN.md': '/zh/docs/jev-decider/',
  },
  api: {
    'README.md': '/en/docs/decision-api/', 'README.zh-CN.md': '/zh/docs/decision-api/',
    '../README.md': '/en/docs/decision-extensions/', '../README.zh-CN.md': '/zh/docs/decision-extensions/',
    '../extensions/jev-decider/README.md': '/en/docs/jev-decider/',
    '../extensions/jev-decider/README.zh-CN.md': '/zh/docs/jev-decider/',
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
    'README.md': '/en/docs/jev-decider/', 'README.zh-CN.md': '/zh/docs/jev-decider/',
    '../../README.md': '/en/docs/decision-extensions/', '../../README.zh-CN.md': '/zh/docs/decision-extensions/',
    '../../api/README.md': '/en/docs/decision-api/', '../../api/README.zh-CN.md': '/zh/docs/decision-api/',
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
  if (!(page in decisionSources) || !['zh', 'en'].includes(language)) throw new Error('unknown decision documentation page');
  const markdown = await fs.readFile(path.join(repository, decisionSources[page][language]), 'utf8');
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
