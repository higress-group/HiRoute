import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from './host-mock';
import { TrustReplayPreview } from '../../../src/features/trust/TrustReplayPreview';
import { PresentationRoot, ProductPage } from '../../../src/ui';
import '../../../src/occami/styles.css';

const trace: { threshold: number; plan: string }[] = [];
let release: (() => void) | null = null;
mockIPC(async (command, payload: any) => {
  if (command !== 'observation_read') throw Error('Unexpected command');
  const query = payload.request.intent.query;
  const threshold = query.thresholds_millis[0] as number;
  trace.push({ threshold, plan: query.plan_id });
  if (threshold === 900) await new Promise<void>(resolve => { release = resolve; });
  return { included:10, excluded_by_reason:{}, recorded_share_at_current:.5, estimate_only:true, thresholds:[{threshold_millis:threshold,regular:5,primary:5,protected_as_primary:1,regular_share:.5,primary_share:.5,newly_regular:0}] };
});
function Fixture() {
  const [threshold, setThreshold] = useState(720);
  const [planId, setPlan] = useState('plan-a');
  const [active, setActive] = useState(true);
  Object.assign(window, { replayFixture:{ trace, active, setThreshold, setPlan, setActive, release: () => release?.() } });
  return <PresentationRoot theme="light" textScale={1} language="en"><ProductPage title="Replay"><TrustReplayPreview scope={{planId,revision:1,branchId:'branch',savedThreshold:720,active}} threshold={threshold} language="en" /></ProductPage></PresentationRoot>;
}
createRoot(document.getElementById('root')!).render(<Fixture />);
