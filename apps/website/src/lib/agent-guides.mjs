export const agentGuideHub = 'agent-integrations';

// WorkBuddy is a model source through Magpie; the other entries also have a
// native HiRoute connection. The homepage lists only native connections.
export const agentGuides = {
  'codex-model-routing': {
    zh: 'Codex', en: 'Codex', native: true,
    title: { zh: 'Codex API 赠金与订阅路由', en: 'Codex API credits and subscription routing' },
    description: {
      zh: '在 Codex Desktop 或 CLI 使用 ChatGPT 订阅和 OpenAI API 赠金。把同名模型加入候选，选择订阅优先或赠金优先。',
      en: 'Use OpenAI API credits and your ChatGPT subscription in Codex Desktop or CLI. Combine the same model from both sources and choose which comes first.',
    },
  },
  'claude-code-model-routing': {
    zh: 'Claude Code', en: 'Claude Code', native: true,
    title: { zh: 'Claude Code API 赠金与订阅路由', en: 'Claude Code API credits and subscription routing' },
    description: {
      zh: '在 Claude Code 使用订阅和 Claude API 赠金。把同名模型放入候选并设置备用顺序，再映射到 Opus、Sonnet 或 Haiku。',
      en: 'Use Claude API credits with Claude Code. Pair subscription and API candidates for the same model, order the fallback and map an Opus, Sonnet or Haiku preset.',
    },
  },
  'qoder-model-routing': {
    zh: 'Qoder', en: 'Qoder', native: true,
    title: { zh: 'Qoder CLI 模型路由与订阅接入', en: 'Qoder CLI model routing and subscription setup' },
    description: {
      zh: '通过 Magpie headless 导入 Qoder 订阅模型，配置按难度分流，并接入 Qoder CLI 或 Codex Desktop 独立窗口。',
      en: 'Import Qoder subscription models through headless Magpie, route by task difficulty, and connect Qoder CLI or a separate Codex Desktop window.',
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
    title: { zh: 'DeepSeek Harness Web 与 Desktop 模型路由', en: 'DeepSeek Harness model routing for Web and Desktop' },
    description: {
      zh: '扫描 DeepSeek Harness 的现有模型，配置按难度分流，再在 Web 或 Desktop 的模型列表中选用 HiRoute 路由。',
      en: 'Scan existing DeepSeek Harness model sources, configure difficulty routing and select your HiRoute route in the Web or Desktop model picker.',
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
