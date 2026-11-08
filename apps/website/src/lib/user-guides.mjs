import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { marked } from 'marked';
import { localePath } from './locale.mjs';

const website = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const content = path.join(website, 'content/guides');

// `title` and `description` are the search-result title and snippet for each guide.
// Keep them specific to the guide; the build rejects a repeated title or description.
export const guidePages = {
  quickstart: {
    zh: '快速开始', en: 'Quickstart',
    title: { zh: '快速开始：模型路由与 Agent 路由', en: 'Quickstart: model routing and Agent routing' },
    description: {
      zh: '开启内置管理技能，让 Agent 通过 CLI 快速上手；按图配置模型路由与 Agent 路由。界面和 CLI 共用配置与执行记录，每页附操作对照。',
      en: 'Enable the management Skill for a faster start with an Agent and CLI. Follow illustrated model and Agent routing guides, with shared UI/CLI data and command maps.',
    },
  },
  'install-macos': {
    zh: 'macOS 安装与首次启动', en: 'Install and open HiRoute on macOS',
    title: { zh: '在 macOS 上安装桌面应用', en: 'Install HiRoute Desktop on macOS' },
    description: {
      zh: 'HiRoute 桌面应用支持 macOS 15.0 及以上版本：按 Apple 芯片或 Intel 选择安装包，核对 SHA-256，并完成首次打开。',
      en: 'HiRoute Desktop supports macOS 15.0 or later. Choose the Apple silicon or Intel installer, verify its SHA-256 and approve the first launch.',
    },
  },
  'install-linux': {
    zh: 'Linux Web UI 与 CLI 安装', en: 'Install HiRoute on Linux: Web UI and CLI',
    title: { zh: '在 Linux 上安装：Web UI 与 CLI', en: 'Install HiRoute on Linux: Web UI and CLI' },
    description: {
      zh: '在服务器和远程工作站上用一行命令安装 HiRoute，支持 x86_64 与 ARM64，无需 sudo；启用 Web UI 后可以在浏览器中管理模型、路由、Agent 和会话。',
      en: 'Install HiRoute on servers and remote workstations with one command, on x86_64 or ARM64 without sudo, then manage models, routes, Agents and sessions from a browser.',
    },
  },
  'install-windows': {
    zh: 'Windows 桌面版安装', en: 'Install HiRoute Desktop on Windows',
    title: { zh: '在 Windows 上安装桌面应用', en: 'Install HiRoute Desktop on Windows' },
    description: {
      zh: 'HiRoute Windows 桌面应用按当前用户安装，支持原生 Windows x64，自带命令行和后台服务，不需要 Python 或管理员权限。',
      en: 'HiRoute Desktop for Windows installs for the current user on native Windows x64, includes the command line and background service, and needs no Python or administrator rights.',
    },
  },
  models: {
    zh: '接入模型', en: 'Connect models',
    title: { zh: '接入模型：订阅、API 与本地模型', en: 'Connect models: subscriptions, APIs and local models' },
    description: {
      zh: '连接 Codex、Claude Code 订阅、API 和本地模型；把订阅和 API 里的同名模型组成一条路由，订阅暂时用不了时接着用 API，还可以限制 API Key 的花费。',
      en: 'Connect Codex and Claude Code subscriptions, APIs and local models. Combine a subscription and an API that offer the same model, so work continues on the API when the subscription is unavailable.',
    },
  },
  'model-trial': {
    zh: '试用新模型', en: 'Try a new model',
    title: { zh: '试用新模型：先接简单轮次，主力模型兜底', en: 'Try a new model with your primary model as backup' },
    description: {
      zh: '在自己的任务里试用新模型：新模型先承担简单轮次，熟悉的主力模型兜底，没做好时由主力接手；再根据记录逐步调整新模型的工作量。',
      en: 'Try a new model on your own tasks. It takes the simple turns while a familiar primary model backs it up and steps in when needed; then adjust its share from your records.',
    },
  },
  'model-routing': {
    zh: '使用智能模型路由', en: 'Use smart model routing',
    title: { zh: '智能模型路由：按难度分流、免费优先与自定义分支', en: 'Smart model routing: difficulty routing, free first and custom branches' },
    description: {
      zh: '为 Agent 配置一组模型：固定模型和免费优先按候选顺序尝试，按难度分流把常规工作交给经济模型、难点交给主力，自定义分支先区分任务类别再选模型。',
      en: 'Configure a set of models for your Agent. Fixed model and Free first follow candidate order, difficulty routing keeps hard parts on your primary model, and custom branches pick by task category.',
    },
  },
  'task-routing': {
    zh: 'Agent 智能路由', en: 'Smart Agent routing',
    title: { zh: 'Agent 智能路由：跨 Harness 委派任务', en: 'Smart Agent routing: delegate across harnesses' },
    description: {
      zh: '让 Codex 委派给 DeepSeek Harness，或让 Hermes 用 Codex 编码。按图配置执行 Agent、开启任务路由，再查看进度并取回结果。',
      en: 'Delegate from Codex to DeepSeek Harness, or use Codex for coding from Hermes. Configure an execution Agent, enable task routing and follow the results with illustrated steps.',
    },
  },
  cli: {
    zh: 'HiRoute CLI', en: 'HiRoute CLI',
    title: { zh: 'HiRoute CLI：在终端管理模型、路由、Agent 与任务', en: 'HiRoute CLI: manage models, routes, Agents and tasks' },
    description: {
      zh: '推荐开启内置管理与委派技能，让 Agent 操作 CLI。界面中的管理与任务操作都有 CLI 对应入口，底层配置和记录互通。',
      en: 'Enable the built-in management and delegation Skills to let an Agent use the CLI. UI management and task operations have CLI equivalents and share configuration and records.',
    },
  },
};

export function guideRoute(slug, language) {
  return localePath(language, slug === 'quickstart' ? '/docs/' : `/docs/${slug}/`);
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
