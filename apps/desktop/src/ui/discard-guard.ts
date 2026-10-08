import { useEffect } from 'react';
type Scope = 'decisions' | 'models' | 'routing' | 'agents';
const eventName = 'hiroute-before-editor-replace';
let pending: Promise<boolean> | null = null;
type Decision = () => Promise<boolean>;

function button(label: string, className: string, onClick: () => void): HTMLButtonElement {
  const element = document.createElement('button');
  element.type = 'button';
  element.className = className;
  element.textContent = label;
  element.onclick = onClick;
  return element;
}

export function confirmDiscard(language: 'zh' | 'en'): Promise<boolean> {
  if (pending) return pending;
  const zh = language === 'zh';
  pending = new Promise<boolean>(resolve => {
    const previous = document.activeElement as HTMLElement | null;
    const dialog = document.createElement('dialog');
    dialog.className = 'discard-dialog';
    const title = document.createElement('h2');
    title.id = 'discard-dialog-title';
    title.textContent = zh ? '放弃未保存的修改？' : 'Discard unsaved changes?';
    dialog.setAttribute('aria-labelledby', title.id);
    const actions = document.createElement('div');
    actions.className = 'discard-dialog-actions';
    const description = document.createElement('p');
    description.id = 'discard-dialog-description';
    description.textContent = zh ? '离开后，这次未保存的修改将丢失。' : 'Leaving will lose your unsaved changes.';
    dialog.setAttribute('aria-describedby', description.id);
    function finish(accepted: boolean) {
      dialog.close(); dialog.remove(); pending = null;
      if (previous?.isConnected) previous.focus();
      resolve(accepted);
    }
    const keep = button(zh ? '继续编辑' : 'Keep editing', 'btn btn-primary', () => finish(false));
    const discard = button(zh ? '放弃修改' : 'Discard changes', 'btn btn-danger', () => finish(true));
    dialog.oncancel = event => { event.preventDefault(); finish(false); };
    actions.append(keep, discard); dialog.append(title, description, actions);
    (document.querySelector('.app-window') ?? document.body).append(dialog);
    dialog.showModal(); keep.focus();
  });
  return pending;
}

export function confirmSaveDraftOrDiscard(language: 'zh' | 'en', saveDraft: () => Promise<boolean>): Promise<boolean> {
  if (pending) return pending;
  const zh = language === 'zh';
  pending = new Promise<boolean>(resolve => {
    const previous = document.activeElement as HTMLElement | null;
    const dialog = document.createElement('dialog');
    dialog.className = 'discard-dialog modal route-leave-dialog';
    const title = document.createElement('h2');
    title.id = 'route-leave-dialog-title';
    title.textContent = zh ? '保存这次修改？' : 'Save your changes?';
    dialog.setAttribute('aria-labelledby', title.id);
    const head = document.createElement('header');
    head.className = 'modal-head';
    const heading = document.createElement('div');
    heading.append(title);
    const description = document.createElement('p');
    description.id = 'route-leave-dialog-description';
    description.textContent = zh
      ? '离开前可以保存为草稿，稍后继续。已发布的路由不会改变。'
      : 'Save a draft to continue later. Published routing will remain unchanged.';
    dialog.setAttribute('aria-describedby', description.id);
    const body = document.createElement('div');
    body.className = 'modal-body';
    body.append(description);
    const actions = document.createElement('div');
    actions.className = 'modal-foot discard-dialog-actions';
    let finished = false;
    function finish(accepted: boolean) {
      if (finished) return;
      finished = true;
      dialog.close();
      dialog.remove();
      pending = null;
      if (!accepted && previous?.isConnected) previous.focus();
      resolve(accepted);
    }
    const keep = button(zh ? '继续编辑' : 'Keep editing', 'btn', () => finish(false));
    const discard = button(zh ? '不保存' : 'Discard', 'btn', () => finish(true));
    const save = button(zh ? '保存草稿并离开' : 'Save draft and leave', 'btn btn-primary', () => {
      keep.disabled = true;
      discard.disabled = true;
      save.disabled = true;
      save.textContent = zh ? '正在保存…' : 'Saving…';
      void saveDraft().then(finish).catch(() => finish(false));
    });
    const close = button('', 'icon-btn', () => finish(false));
    close.setAttribute('aria-label', zh ? '关闭' : 'Close');
    close.title = zh ? '关闭' : 'Close';
    close.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"></path></svg>';
    head.append(heading, close);
    dialog.oncancel = event => { event.preventDefault(); finish(false); };
    actions.append(keep, discard, save);
    dialog.append(head, body, actions);
    (document.querySelector('.app-window') ?? document.body).append(dialog);
    dialog.showModal();
    keep.focus();
  });
  return pending;
}
export async function requestEditorReplacement(scope: Scope): Promise<boolean> {
  const decisions: Promise<boolean>[] = [];
  window.dispatchEvent(new CustomEvent(eventName, { detail: { scope, decisions } }));
  return (await Promise.all(decisions)).every(Boolean);
}
export function useDiscardGuard(scope: Scope, dirty: boolean | (() => boolean), language: 'zh' | 'en', decide?: Decision): void {
  useEffect(() => {
    const guard = (event: Event) => {
      const detail = (event as CustomEvent<{ scope: Scope; decisions: Promise<boolean>[] }>).detail;
      const shouldConfirm = typeof dirty === 'function' ? dirty() : dirty;
      if (detail.scope === scope && shouldConfirm) detail.decisions.push(decide ? decide() : confirmDiscard(language));
    };
    window.addEventListener(eventName, guard);
    return () => window.removeEventListener(eventName, guard);
  }, [scope, dirty, language, decide]);
}
