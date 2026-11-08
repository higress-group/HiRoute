(() => {
  const en = document.documentElement.lang === 'en';
  for (const prompt of document.querySelectorAll('.agent-prompt')) {
    const code = prompt.querySelector('pre code');
    if (!code) continue;
    const actions = document.createElement('div');
    actions.className = 'agent-prompt-actions';
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = en ? 'Copy prompt' : '复制提示词';
    const status = document.createElement('span');
    status.setAttribute('role', 'status');
    button.addEventListener('click', async () => {
      button.disabled = true;
      status.textContent = '';
      try {
        await navigator.clipboard.writeText(code.textContent);
        status.textContent = en ? 'Copied' : '已复制';
      } catch {
        const range = document.createRange();
        range.selectNodeContents(code);
        const selection = window.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(range);
        status.textContent = en ? 'Text selected. Copy it manually.' : '已选中文本，请手动复制。';
      } finally {
        button.disabled = false;
      }
    });
    actions.append(button, status);
    code.parentElement.before(actions);
  }
})();
