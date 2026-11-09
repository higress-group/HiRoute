(() => {
  const picker = document.querySelector('[data-install-picker]');
  if (!picker) return;
  const cards = [...picker.querySelectorAll('[data-install-platform]')];
  const panels = [...picker.querySelectorAll('[data-install-panel]')];
  const prompt = picker.querySelector('[data-install-prompt]');
  const en = document.documentElement.lang.startsWith('en');

  function showSelection() {
    const id = location.hash.slice(1);
    const selected = panels.find(panel => panel.id === id
      || [...panel.querySelectorAll('[data-install-artifact]')].some(artifact => artifact.id === id));
    prompt.hidden = Boolean(selected);
    for (const card of cards) {
      if (card.dataset.installPlatform === selected?.id) card.setAttribute('aria-current', 'true');
      else card.removeAttribute('aria-current');
    }
    for (const panel of panels) panel.hidden = panel !== selected;
    if (!selected) return;
    const artifacts = [...selected.querySelectorAll('[data-install-artifact]')];
    const current = artifacts.find(artifact => artifact.id === id) ?? artifacts[0];
    for (const artifact of artifacts) artifact.hidden = artifact !== current;
    for (const option of selected.querySelectorAll('[data-install-architecture]')) {
      if (option.hash === `#${current?.id}`) option.setAttribute('aria-current', 'true');
      else option.removeAttribute('aria-current');
    }
    return selected;
  }

  picker.addEventListener('click', event => {
    const link = event.target.closest('a[data-install-platform], a[data-install-architecture]');
    if (!link || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    if (location.hash !== link.hash) history.pushState(null, '', link.hash);
    const panel = showSelection();
    if (link.dataset.installPlatform && panel && panel.querySelector('.install-panel-heading').getBoundingClientRect().bottom > window.innerHeight) {
      link.parentElement.scrollIntoView({ block: 'start' });
    }
  });
  picker.addEventListener('keydown', event => {
    const link = event.target.closest('[data-install-platform], [data-install-architecture]');
    if (!link || !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    const peers = [...link.parentElement.querySelectorAll('a')];
    let index = peers.indexOf(link);
    if (event.key === 'Home') index = 0;
    else if (event.key === 'End') index = peers.length - 1;
    else index = (index + (['ArrowLeft', 'ArrowUp'].includes(event.key) ? -1 : 1) + peers.length) % peers.length;
    event.preventDefault();
    peers[index].focus();
  });
  window.addEventListener('popstate', showSelection);
  window.addEventListener('hashchange', showSelection);
  showSelection();

  for (const button of picker.querySelectorAll('[data-copy-install]')) {
    button.hidden = false;
    const block = button.closest('.install-code');
    const code = block.querySelector('code');
    const status = block.querySelector('[role="status"]');
    button.addEventListener('click', async () => {
      if (button.disabled) return;
      button.disabled = true;
      status.textContent = en ? 'Copying…' : '正在复制…';
      try {
        await navigator.clipboard.writeText(code.textContent.trim());
        status.textContent = en ? 'Copied to clipboard.' : '已复制到剪贴板。';
      } catch {
        const selection = window.getSelection();
        if (selection) {
          const range = document.createRange();
          range.selectNodeContents(code);
          selection.removeAllRanges();
          selection.addRange(range);
        }
        code.parentElement.focus();
        status.textContent = en ? 'Copy unavailable. Select and copy the text manually.' : '自动复制不可用，请手动选择并复制文本。';
      } finally {
        button.disabled = false;
      }
    });
  }
})();
