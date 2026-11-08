const hasClass = (tag, name) => (tag.match(/\bclass="([^"]*)"/)?.[1] ?? '').split(/\s+/).includes(name);
const agentBlocks = html => [...html.matchAll(/<aside\b[^>]*>[\s\S]*?<\/aside>/g)]
  .filter(([block]) => hasClass(block.slice(0, block.indexOf('>') + 1), 'agent-guide'));

export function validateAgentGuide(markdown, slug, language) {
  const fail = message => { throw new Error(`${slug}.${language}: ${message}`); };
  const blocks = agentBlocks(markdown);
  if (blocks.length !== 1 || (markdown.match(/id="agent-cli"/g) ?? []).length !== 1
      || !blocks[0][0].includes('id="agent-cli"')) fail('provide one copyable Agent guide at #agent-cli');
  const block = blocks[0][0];
  if (!block.includes('class="agent-guide-heading"') || !block.includes('class="agent-guide-preview"')
      || !/<summary>\S[\s\S]*?<\/summary>/.test(block)) fail('keep the copy heading and native instruction preview');
  const messages = [...block.matchAll(/^```text\n([\s\S]*?)\n```/gm)];
  if (messages.length !== 1) fail('put the complete Agent message in one text code fence');
  const message = messages[0][1];
  const base = `https://hiroute.ai/${language}/docs/`;
  const links = [base + (slug === 'quickstart' ? '' : `${slug}/`), `${base}cli/`, `${base}#management-skill`];
  if (/\bworker\b/.test(message)) links.push(`${base}task-routing/#delegation-skill`);
  const urls = new Set(message.match(/https:\/\/[^\s]+/g) ?? []);
  for (const link of links) if (!urls.has(link)) fail(`copied instructions need ${link}`);
  if (!message.includes('hiroute') || !message.includes('--help')
      || !message.includes('schema show') || !message.includes('--include-schema')
      || !/^- .+(?:：|: ).+/m.test(message)) fail('include operation-to-CLI pointers and installed help/schema discovery');
  return message;
}

export function validateAgentGuidePage(html, route) {
  if (!/<script\b[^>]*src="\/scripts\/guide-agent-copy\.js"[^>]*>/.test(html)) {
    throw new Error(`${route}: load the shared Copy for Agent interaction`);
  }
  if (agentBlocks(html).length !== 1
      || !/<code\b[^>]*class="language-text"/.test(html)) {
    throw new Error(`${route}: rendered Agent instructions are missing`);
  }
}
