(() => {
  const key = 'hiroute-language';
  const url = new URL(location.href);
  const explicit = url.searchParams.get('lang');
  let preference;
  try { preference = localStorage.getItem(key); } catch {}
  if (explicit === 'zh' || explicit === 'en') {
    preference = explicit;
    try { localStorage.setItem(key, explicit); } catch {}
  }
  // Redirect pages name their destination, and the root also names the Chinese home
  // page for a saved or explicit Chinese choice. This runs before the page's meta
  // refresh fallback so the query and fragment carry over. Browser language never
  // redirects, so search engines and shared links receive the page they requested.
  const { redirect, redirectZh } = document.documentElement.dataset;
  if (redirect) {
    const target = new URL(preference === 'zh' && redirectZh || redirect, url);
    target.search = url.search;
    target.hash = url.hash;
    location.replace(target.href);
    return;
  }
  document.addEventListener('click', event => {
    const link = event.target.closest?.('a[data-language]');
    if (!link) return;
    const language = link.lang === 'en' ? 'en' : 'zh';
    try { localStorage.setItem(key, language); } catch {}
    const target = new URL(link.href);
    target.searchParams.set('lang', language);
    target.hash = location.hash;
    link.href = target.href;
  }, true);
  if (preference === 'zh' || preference === 'en') return;
  const page = document.documentElement.lang === 'en' ? 'en' : 'zh';
  const browserLanguage = navigator.languages?.[0] || navigator.language || 'en';
  const reader = /^zh(?:-|$)/i.test(browserLanguage) ? 'zh' : 'en';
  if (reader === page) return;
  document.addEventListener('DOMContentLoaded', () => {
    const english = reader === 'en';
    const counterpart = document.querySelector(`a[data-language][lang="${english ? 'en' : 'zh-CN'}"]`);
    if (!counterpart) return;
    const element = (tag, properties) => Object.assign(document.createElement(tag), properties);
    const banner = element('div', { className: 'language-suggestion', lang: counterpart.lang });
    const content = element('div', { className: 'wrap' });
    const link = element('a', { href: counterpart.getAttribute('href'), lang: counterpart.lang,
      textContent: english ? 'View in English →' : '查看中文版 →' });
    link.setAttribute('data-language', '');
    const dismiss = element('button', { type: 'button', textContent: '×' });
    dismiss.setAttribute('aria-label', english ? 'Dismiss' : '关闭');
    // Dismissing keeps the current language, like choosing it in the header.
    dismiss.addEventListener('click', () => {
      try { localStorage.setItem(key, page); } catch {}
      banner.remove();
    });
    content.append(element('span', { textContent: english ? 'This page is also available in English.' : '本页也有中文版。' }), link, dismiss);
    banner.append(content);
    document.body.prepend(banner);
  });
})();
