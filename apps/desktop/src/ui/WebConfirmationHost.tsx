import { invoke } from '../host/index.ts';
import { listenForHostConfirmations } from '../host/index.ts';
import { useEffect, useState } from 'react';
import {
  enqueueWebConfirmation,
  isWebConfirmationRequest,
  removeWebConfirmation,
  type WebConfirmationRequest,
} from '../features/web-confirmation';
import { Dialog } from './Dialog';
import { UiIcon } from './UiIcon';

export function WebConfirmationHost() {
  const [queue, setQueue] = useState<WebConfirmationRequest[]>([]);
  const [resolving, setResolving] = useState(false);
  const [error, setError] = useState('');
  const current = queue[0] ?? null;

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let polling = false;
    void listenForHostConfirmations(request => {
      if (disposed) return;
      setError('');
      setQueue(items => enqueueWebConfirmation(items, request));
    }).then(stop => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch(() => {
      // Snapshot polling below remains a fail-closed delivery path when the
      // targeted WebView event subscription is temporarily unavailable.
    });
    const poll = async () => {
      if (disposed || polling) return;
      polling = true;
      try {
        const request = await invoke<unknown>('web_confirmation_snapshot');
        if (!disposed && isWebConfirmationRequest(request)) {
          setQueue(items => enqueueWebConfirmation(items, request));
        }
      } catch {
        // The targeted event remains the primary delivery path. A failed fallback read
        // never turns into approval and the backend confirmation still expires closed.
      } finally {
        polling = false;
      }
    };
    void poll();
    const poller = window.setInterval(() => void poll(), 750);
    return () => {
      disposed = true;
      window.clearInterval(poller);
      unlisten?.();
    };
  }, []);

  async function resolve(request: WebConfirmationRequest, accepted: boolean) {
    if (resolving) return;
    setResolving(true);
    setError('');
    try {
      await invoke('resolve_web_confirmation', {
        input: { confirmation_id: request.confirmation_id, accepted },
      });
      setQueue(items => removeWebConfirmation(items, request.confirmation_id));
    } catch {
      setError(document.documentElement.lang.startsWith('zh')
        ? '这项确认已失效，请返回原操作后重试。'
        : 'This confirmation expired. Return to the original action and try again.');
    } finally {
      setResolving(false);
    }
  }

  function dismissExpired(request: WebConfirmationRequest) {
    setQueue(items => removeWebConfirmation(items, request.confirmation_id));
    setError('');
  }

  return <Dialog
    open={Boolean(current)}
    title={current?.title ?? ''}
    closeLabel={current?.cancel_label ?? 'Cancel'}
    closeDisabled={resolving}
    onClose={() => {
      if (!current) return;
      if (error) dismissExpired(current);
      else void resolve(current, false);
    }}
    footer={current && <>
      {error
        ? <button className="btn" type="button" data-autofocus onClick={() => dismissExpired(current)}>{current.cancel_label}</button>
        : <>
          <button className="btn" type="button" data-autofocus disabled={resolving} onClick={() => void resolve(current, false)}>{current.cancel_label}</button>
          <button className="btn btn-primary" type="button" disabled={resolving} onClick={() => void resolve(current, true)}>{current.confirm_label}</button>
        </>}
    </>}
  >
    {current && <p className="web-confirmation-message">{current.message}</p>}
    {error && <div className="callout bad" role="alert"><UiIcon name="warning" /><span>{error}</span></div>}
  </Dialog>;
}
