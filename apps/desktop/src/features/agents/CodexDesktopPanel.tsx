import React from 'react';
import './codex-desktop.css';
import type { Agent } from './types';
import { codexDesktopLaunchCommand } from './codex-desktop-launch';
import { Disclosure } from '../../ui';

export function CodexDesktopPanel({ agent, language, configured, launching, disabled, nativeHost, feedback, onLaunch, onConfigure }: {
  agent: Agent; language: 'zh' | 'en'; configured: boolean; launching: boolean;
  disabled: boolean; nativeHost: boolean; onLaunch: () => void; onConfigure: (source: HTMLElement) => void;
  feedback: { code?: string; message: string } | null;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const command = configured && !nativeHost ? codexDesktopLaunchCommand(agent) : null;
  return <section className="codex-instance" data-codex-desktop-instance>
    <div className="agent-launch-row"><div><strong>{text('独立窗口', 'Separate window')}</strong><p>{configured ? text('配置更新后需重启独立实例，会话会保留。', 'Restart this instance after settings updates. Sessions are retained.') : text('选择路由后，即可启动独立窗口。', 'Choose routes to open a separate window.')}</p></div>
      <button type="button" className="btn btn-primary" data-agent-facet={!configured ? 'model' : undefined} data-codex-desktop-launch={configured || undefined} data-codex-desktop-configure={!configured || undefined} disabled={disabled || (configured && !nativeHost && !command)} onClick={event => configured ? onLaunch() : onConfigure(event.currentTarget)}>{launching ? nativeHost ? text('启动中…', 'Opening…') : text('复制中…', 'Copying…') : configured ? nativeHost ? text('启动独立窗口', 'Open separate window') : text('复制启动命令', 'Copy launch command') : text('选择路由并启用', 'Choose routes and enable')}</button>
    </div>
    {feedback && <div className={`callout${feedback.code ? ' bad' : ''}`} role={feedback.code ? 'alert' : 'status'} data-error-code={feedback.code}>{feedback.message}</div>}
    {!nativeHost && configured && <div className="codex-instance-command"><p className="field-help">{text('在 HiRoute 服务所在电脑的桌面终端运行，需已安装 ChatGPT / Codex 桌面应用。', 'Run in a desktop terminal on the HiRoute service computer, with the ChatGPT / Codex desktop app installed.')}</p>{command && <Disclosure label={text('查看启动命令', 'View launch command')} language={language}><pre><code>{command}</code></pre></Disclosure>}</div>}
  </section>;
}

export function CodexDesktopDetails({ agent, language }: { agent: Agent; language: 'zh' | 'en' }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const paths = agent.codex_desktop;
  if (!paths) return null;
  return <section data-codex-desktop-details><h4>{text('独立实例与共享技能', 'Separate instance and shared skills')}</h4>
    <p>{text('配置、登录和会话独立保存。更新路由或令牌后，请退出此独立实例再重新启动。停用接入会保留会话和共享链接。', 'Settings, sign-in and sessions are separate. After changing routes or tokens, quit this instance and start it again. Disabling retains sessions and shared links.')}</p>
    <p>{text('自动读取 ~/.agents/skills 和项目技能；其他 Codex 用户技能按目录链接，同名技能予以保留。系统技能由各实例自行管理。', 'Reads ~/.agents/skills and project skills. Other Codex user skills are linked by directory, keeping existing entries. Each instance manages its own system skills.')}</p>
    <dl className="codex-instance-paths">
      <div><dt>{text('配置与会话', 'Settings and sessions')}</dt><dd>{paths.codex_home}</dd></div>
      <div><dt>{text('桌面应用数据', 'Desktop app data')}</dt><dd>{paths.user_data}</dd></div>
      <div><dt>{text('用户技能来源', 'User skill source')}</dt><dd>{paths.source_codex_home}/skills</dd></div>
    </dl>
  </section>;
}
