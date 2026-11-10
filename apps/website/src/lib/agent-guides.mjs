export const agentGuideHub = 'agent-integrations';

// WorkBuddy is a model source through Magpie; the other entries also have a
// native HiRoute connection. The homepage lists only native connections.
export const agentGuides = {
  'codex-model-routing': {
    zh: 'Codex', en: 'Codex', native: true,
    title: { zh: 'Codex 订阅模型路由与 Codex Desktop 接入教程', en: 'Codex subscription model routing and Desktop setup' },
    description: {
      zh: '登录 ChatGPT 订阅并导入 Codex 模型，配置按难度分流，再接入 Codex Desktop 独立窗口或 CLI。每一步都有图文说明。',
      en: 'Import Codex models from your ChatGPT subscription, configure difficulty routing, and connect Codex Desktop or CLI with step-by-step screenshots.',
    },
  },
  'claude-code-model-routing': {
    zh: 'Claude Code', en: 'Claude Code', native: true,
    title: { zh: 'Claude Code 模型路由：映射 Opus、Sonnet 和 Haiku', en: 'Claude Code model routing with Opus, Sonnet and Haiku presets' },
    description: {
      zh: '登录 Claude 订阅并选择账号模型，配置常规组与主力组，再映射到 Claude Code 的 Opus、Sonnet 或 Haiku 预设，配有完整步骤和截图。',
      en: 'Sign in to your Claude subscription, import models, configure difficulty routing and map it to Claude Code presets. Follow the screenshots for each step.',
    },
  },
  'qoder-model-routing': {
    zh: 'Qoder', en: 'Qoder', native: true,
    title: { zh: 'Qoder CLI 模型路由与订阅接入', en: 'Qoder CLI model routing and subscription setup' },
    description: {
      zh: '通过 Magpie headless 发现 Qoder 订阅模型，配置难度路由并接入 Agent。Qoder 不支持 BYOK 时，也可按截图接入 Codex Desktop。',
      en: 'Discover Qoder subscription models with headless Magpie, build a difficulty route and connect an Agent. Use Codex Desktop when Qoder BYOK is unavailable.',
    },
  },
  'pi-model-routing': {
    zh: 'Pi', en: 'Pi', native: true,
    title: { zh: 'Pi Coding Agent 多模型路由：导入 models.json 并按难度分流', en: 'Pi Coding Agent model routing by difficulty' },
    description: {
      zh: '复用 Pi 的 models.json 接入或其他订阅、API，把路由加入 Pi 的 /model，再将需要独立验证的工作委派给 Codex CLI。',
      en: 'Import eligible Pi model sources, combine them with other APIs or subscriptions, pick a HiRoute route in Pi, and delegate verification to Codex CLI.',
    },
  },
  'deepseek-harness-model-routing': {
    zh: 'DeepSeek Harness', en: 'DeepSeek Harness', native: true,
    title: { zh: 'DeepSeek Harness 多模型路由：接入已有模型并配置执行 Agent', en: 'DeepSeek Harness model routing and task delegation' },
    description: {
      zh: '复用 DeepSeek Harness 的静态模型配置，给 Web 或桌面版添加按难度分流路由，并用 DSH 或 Codex CLI 执行委派任务。',
      en: 'Connect existing DeepSeek Harness model sources, add a difficulty route to Web or Desktop, and configure DSH or Codex CLI for delegated work.',
    },
  },
  'opencode-model-routing': {
    zh: 'OpenCode', en: 'OpenCode', native: true,
    title: { zh: 'OpenCode 多模型路由：复用 API 配置并在 /models 选用', en: 'OpenCode model routing: add a difficulty route to /models' },
    description: {
      zh: '导入受支持的 OpenCode API 接入，在 /models 选用 HiRoute 路由，同时保留原有模型和默认值，并把独立任务委派给 Pi。',
      en: 'Reuse eligible OpenCode API settings, select a HiRoute difficulty route in /models alongside your existing providers, and delegate a task to Pi.',
    },
  },
  'hermes-model-routing': {
    zh: 'Hermes', en: 'Hermes', native: true,
    title: { zh: 'Hermes Agent 模型路由：复用 profile 并委派给 Codex', en: 'Hermes Agent model routing and Codex delegation' },
    description: {
      zh: '从 Hermes 实际使用的 profile 复用模型，给主对话配置难度路由，再用一条独立计划让 Codex CLI 执行编码任务。',
      en: 'Reuse model sources from the active Hermes profile, route its conversations by difficulty, and hand off coding through a separate Codex CLI plan.',
    },
  },
  'workbuddy-model-routing': {
    zh: 'WorkBuddy', en: 'WorkBuddy', native: false,
    title: { zh: 'WorkBuddy 模型接入 HiRoute：用 Magpie headless 连接 Pi 等 Agent', en: 'Use WorkBuddy models in Pi with headless Magpie and HiRoute' },
    description: {
      zh: '接入 WorkBuddy 国内版或国际版账号可用的模型，在 HiRoute 配置常规组与主力组，并用 Pi 或 Codex CLI 完成任务。',
      en: 'Connect models from a WorkBuddy account through headless Magpie, set Regular and Primary groups in HiRoute, and run a task with Pi or Codex CLI.',
    },
  },
};

export const magpieGuide = 'magpie-model-sources';
