import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { marked } from 'marked';

const website = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const content = path.join(website, 'content/guides');

export const guidePages = {
  quickstart: { zh: '快速开始', en: 'Quickstart' },
  'install-macos': { zh: 'macOS 安装与首次启动', en: 'Install and open HiRoute on macOS' },
  'install-linux': { zh: 'Linux 无界面版安装', en: 'Install HiRoute headless on Linux' },
  'model-routing': { zh: '使用智能模型路由', en: 'Use smart model routing' },
  'task-routing': { zh: '使用智能任务路由', en: 'Use smart task routing' },
  cli: { zh: 'HiRoute CLI', en: 'HiRoute CLI' },
};

export function guideRoute(slug, language) {
  const prefix = language === 'en' ? '/en' : '';
  return slug === 'quickstart' ? `${prefix}/docs/` : `${prefix}/docs/${slug}/`;
}

export async function renderUserGuide(slug, language) {
  if (!(slug in guidePages) || !['zh', 'en'].includes(language)) throw new Error('unknown user guide');
  const markdown = await fs.readFile(path.join(content, `${slug}.${language}.md`), 'utf8');
  const renderer = new marked.Renderer();
  renderer.link = ({ href, title, text }) => {
    const external = /^https?:/.test(href) ? ' target="_blank" rel="noreferrer"' : '';
    const download = href === '/api/decision.openapi.json' ? ' download' : '';
    const titleAttribute = title ? ` title="${title.replaceAll('"', '&quot;')}"` : '';
    return `<a href="${href}"${titleAttribute}${external}${download}>${text}</a>`;
  };
  return marked.parse(markdown, { gfm: true, renderer });
}
