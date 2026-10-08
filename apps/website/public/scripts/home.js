// Shared static-site interactions; the content itself is rendered in each language.
const boundaries = document.documentElement.lang === 'en' ? {
  continuation: {
    kicker: 'Tool continuation within one turn',
    title: 'After a tool returns, continue this turn’s work.',
    copy: 'Tool results still belong to the current task. HiRoute inherits this turn’s frozen decision and prefers the eligible current model within its group, helping reuse the prefix. Candidate failures may use bounded failover; a tool result itself does not trigger another judgment.',
    assessment: 'Inherit this turn’s frozen task and model-group decision',
    choice: 'Model A continues with the tool results in this example'
  },
  human: {
    kicker: 'A decision point with human input',
    title: 'A new user turn gets a fresh decision.',
    copy: 'A new question or feedback prompts a fresh task and model-group choice, with an optional assessment of the prior stage. Old low scores do not lock the group, and a new turn does not automatically return to economy. The same model can continue if it fits the newly selected group.',
    assessment: 'Judge the new task and, when possible, assess model A’s completed stage',
    choice: 'Select primary for the deeper work; model B takes over in this example'
  }
} : {
  continuation: {
    kicker: '同一轮中的工具续接',
    title: '工具返回后，继续本轮工作。',
    copy: '工具结果仍属于当前任务，HiRoute 沿用本轮冻结的决策，尽量复用同组内合格的当前模型与前缀。候选故障可以按计划接力，工具返回本身不会重新判断任务。',
    assessment: '沿用本轮已冻结的任务与模型组判断',
    choice: '示例中模型 A 继续处理工具结果'
  },
  human: {
    kicker: '用户参与后的决策机会',
    title: '新用户轮次，重新判断当前工作。',
    copy: '新的问题或反馈触发本轮任务与模型组判断，并可评价上一阶段。旧低分不会跨轮锁定，新一轮也不会自动回到省钱组。原模型符合本轮新选中的模型组时，可以继续使用。',
    assessment: '判断新任务，并在证据可用时评价模型 A 已完成的阶段',
    choice: '示例中深入工作使用主力组，由模型 B 接续'
  }
};

document.querySelectorAll('[data-boundary]').forEach(button => {
  button.addEventListener('click', () => {
    const boundary = boundaries[button.dataset.boundary];
    document.querySelectorAll('[data-boundary]').forEach(item => {
      item.setAttribute('aria-pressed', String(item === button));
    });
    for (const field of ['kicker', 'title', 'copy', 'assessment', 'choice']) {
      document.getElementById(`boundary-${field}`).textContent = boundary[field];
    }
  });
});

// FAQ links open their answer. Native links/details also work without JavaScript.
function revealLinkedAnswer() {
  const target = document.getElementById(location.hash.slice(1));
  if (target instanceof HTMLDetailsElement) target.open = true;
}
window.addEventListener('hashchange', revealLinkedAnswer);
document.querySelectorAll('a[href^="#"]').forEach(link => {
  link.addEventListener('click', () => {
    const target = document.getElementById(link.getAttribute('href').slice(1));
    if (target instanceof HTMLDetailsElement) target.open = true;
  });
});
document.querySelectorAll('[data-language]').forEach(link => {
  link.addEventListener('click', () => {
    const target = new URL(link.href);
    target.hash = location.hash;
    link.href = target.href;
  });
});
revealLinkedAnswer();
