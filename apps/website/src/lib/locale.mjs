// English is the default site language and every page lives under its language
// prefix. The root and the former unprefixed Chinese URLs are redirect pages.
export const localePath = (language, path = '/') => `/${language === 'en' ? 'en' : 'zh'}${path}`;

// Chinese pages published without a prefix before English became the default.
// They remain permanent redirects to /zh/ so existing links and indexes keep working.
export const LEGACY_CHINESE_PATHS = [
  '/docs/', '/docs/install-macos/', '/docs/install-linux/', '/docs/install-windows/',
  '/docs/models/', '/docs/model-trial/', '/docs/model-routing/', '/docs/task-routing/', '/docs/cli/',
  '/docs/decision-extensions/', '/docs/decision-api/', '/docs/jev-decider/',
  '/news/', '/news/new-model-trials-and-task-routing/', '/news/hermes-super-agent/',
  '/news/routing-cache-cost/', '/news/astra-qwen-smart-routing/', '/news/hiroute-open-source/',
  '/download/', '/changelog/',
];
