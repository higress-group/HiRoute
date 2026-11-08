(() => {
  const en = document.documentElement.lang.startsWith('en');
  for (const guide of document.querySelectorAll('.agent-guide')) {
    const heading = guide.querySelector('.agent-guide-heading');
    const preview = guide.querySelector('.agent-guide-preview');
    const code = preview?.querySelector('pre code');
    if (!heading || !code || heading.querySelector('button')) continue;

    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'button outline agent-guide-copy';
    button.textContent = en ? 'Copy for Agent' : '复制给 Agent';
    const status = document.createElement('p');
    status.className = 'agent-copy-status';
    status.setAttribute('role', 'status');
    status.setAttribute('aria-atomic', 'true');
    code.parentElement.tabIndex = 0;
    code.parentElement.setAttribute('aria-label', en ? 'Instructions for your Agent' : '给 Agent 的操作指引');
    heading.append(button);
    heading.after(status);

    button.addEventListener('click', async () => {
      if (button.disabled) return;
      button.disabled = true;
      status.textContent = en ? 'Copying…' : '正在复制…';
      try {
        await navigator.clipboard.writeText(code.textContent.trim());
        status.textContent = en ? 'Copied. Paste into your Agent.' : '已复制，可粘贴给 Agent。';
      } catch {
        preview.open = true;
        const selection = window.getSelection();
        if (selection) {
          const range = document.createRange();
          range.selectNodeContents(code);
          selection.removeAllRanges();
          selection.addRange(range);
        }
        status.textContent = selection
          ? (en ? 'Copy failed. All instructions are selected for manual copying.' : '复制失败，已选中全文，可手动复制。')
          : (en ? 'Copy failed. Copy the instructions below manually.' : '复制失败，请手动复制下方指引。');
        guide.scrollIntoView({ block: 'start', behavior: 'instant' });
      } finally {
        button.disabled = false;
      }
    });
  }
})();
