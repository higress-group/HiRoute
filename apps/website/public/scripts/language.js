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
  if (url.pathname === '/') {
    const browserLanguage = navigator.languages?.[0] || navigator.language || 'en';
    const language = preference === 'zh' || preference === 'en'
      ? preference : /^zh(?:-|$)/i.test(browserLanguage) ? 'zh' : 'en';
    if (language === 'en') {
      url.pathname = '/en/';
      location.replace(url.href);
    }
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
})();
