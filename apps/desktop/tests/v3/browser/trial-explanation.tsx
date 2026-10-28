import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { TrialProtectionExplanation } from '../../../src/features/trial/TrialProtectionExplanation';
import { emptyService, type Classifier } from '../../../src/features/decision-services/types';
import { PresentationRoot } from '../../../src/ui/PresentationRoot';
import '../../../src/occami/styles.css';
import '../../../src/features/trial/trial.css';

const params = new URLSearchParams(location.search);
function Harness() {
  const [language, setLanguage] = useState<'zh' | 'en'>(params.get('lang') === 'zh' ? 'zh' : 'en');
  const [mode, setMode] = useState<'native' | 'local' | 'custom'>('native');
  const service = emptyService();
  const classifier: Classifier = mode === 'local' ? { kind: 'local_rules' }
    : { kind: 'decision_service', service: mode === 'custom'
      ? { ...service, connection: { kind: 'custom', endpoint: 'https://example.test', timeout_ms: 1000 } }
      : service };
  Object.assign(window, { setTrialExplanationMode: setMode, setTrialExplanationLanguage: setLanguage });
  return <PresentationRoot language={language} theme={params.get('theme') === 'dark' ? 'dark' : 'light'} textScale={1}>
    <main className="trial-page"><section className="trial-card"><label>Fixture input <input defaultValue="retain me" /></label><TrialProtectionExplanation classifier={classifier} language={language} /></section></main>
  </PresentationRoot>;
}
createRoot(document.getElementById('root')!).render(<Harness />);
