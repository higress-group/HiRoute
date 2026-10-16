export function HermesRootFields({ value, onChange, language, disabled = false }: {
  value: string;
  onChange: (value: string) => void;
  language: 'zh' | 'en';
  disabled?: boolean;
}) {
  const zh = language === 'zh';
  return <>
    <label className="field"><span className="field-label">{zh ? '本机有效配置目录的绝对路径' : 'Absolute path to the effective local configuration directory'}</span>
      <input className="input" autoFocus disabled={disabled} value={value} onChange={event => onChange(event.target.value)} />
    </label>
    <p className="field-help">{zh ? '选择当前 Hermes 实际使用的目录，包括独立 profile。' : 'Select the directory Hermes actually uses, including a separate profile.'}</p>
  </>;
}
