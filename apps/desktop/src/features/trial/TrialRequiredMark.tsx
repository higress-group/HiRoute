/** Keep required status accessible without repeating it in visible headings. */
export function TrialRequiredMark({ language }: { language: 'zh' | 'en' }) {
  return <><span className="trial-required-mark" aria-hidden="true">*</span><span className="sr-only">{language === 'zh' ? '（必填）' : ' (required)'}</span></>;
}
