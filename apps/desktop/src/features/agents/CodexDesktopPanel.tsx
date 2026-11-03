import React from 'react';
import './codex-desktop.css';
import { Disclosure } from '../../ui';
import type { Agent } from './types';

export function CodexDesktopPanel({ agent, language, configured, launching, disabled, nativeHost, feedback, onLaunch, onConfigure }: {
  agent: Agent; language: 'zh' | 'en'; configured: boolean; launching: boolean;
  disabled: boolean; nativeHost: boolean; onLaunch: () => void; onConfigure: (source: HTMLElement) => void;
  feedback: { code?: string; message: string } | null;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const paths = agent.codex_desktop;
  return <section className="detail-section codex-instance" data-codex-desktop-instance>
    <div className="codex-instance-heading"><h3>{text('HiRoute 独立窗口', 'A separate HiRoute window')}</h3>
      <button type="button" className="btn btn-primary" data-codex-desktop-launch={configured || undefined} data-codex-desktop-configure={!configured || undefined} disabled={disabled || (configured && !nativeHost)} onClick={event => configured ? onLaunch() : onConfigure(event.currentTarget)}>{launching ? text('启动中…', 'Opening…') : configured ? text('启动独立窗口', 'Open separate window') : text('选择路由并启用', 'Choose routes and enable')}</button>
    </div>
    {feedback && <div className={`callout${feedback.code ? ' bad' : ''}`} role={feedback.code ? 'alert' : 'status'} data-error-code={feedback.code}>{feedback.message}</div>}
    <p>{text('使用专属配置和会话目录，可与 CLI 独立 profile 同时启用。主用 Codex Desktop 的订阅与会话保持独立。', 'Uses its own settings and sessions alongside the separate CLI profile. Your regular Codex Desktop subscription and sessions stay separate.')}</p>
    {!nativeHost ? <p className="field-help">{text('请在 macOS 版 HiRoute Desktop 中启动。', 'Open this instance from HiRoute Desktop on macOS.')}</p>
      : !configured ? <p className="field-help">{text('启用模型路由后，即可从这里启动独立窗口。', 'Enable model routing, then open the separate window here.')}</p>
        : <p className="field-help">{text('调整路由后，请关闭这个独立窗口再从此处启动；已有会话会保留。', 'After editing routes, close this separate window and reopen it here. Its sessions are retained.')}</p>}
    <div className="codex-instance-skills"><strong>{text('共享用户技能', 'Shared user skills')}</strong><p>{text('复用现有用户技能，保留本实例的同名技能。', 'Reuse your user skills while keeping existing entries in this instance.')}</p></div>
    {paths && <Disclosure label={text('共享方式与实例目录', 'Sharing and instance directories')} language={language}>
      <p className="field-help">{text('自动读取 ~/.agents/skills 和项目技能；其余 Codex 用户技能按目录链接。系统技能由各实例自行管理。', 'Reads ~/.agents/skills and project skills. Other Codex user skills are linked by directory. Each instance manages its own system skills.')}</p><dl className="codex-instance-paths">
      <div><dt>{text('配置与会话', 'Settings and sessions')}</dt><dd>{paths.codex_home}</dd></div>
      <div><dt>{text('桌面应用数据', 'Desktop app data')}</dt><dd>{paths.user_data}</dd></div>
      <div><dt>{text('用户技能来源', 'User skill source')}</dt><dd>{paths.source_codex_home}/skills</dd></div>
    </dl><p className="field-help">{text('停用只撤销本实例的 HiRoute 接入，保留会话、登录和共享链接。', 'Disabling removes this instance’s HiRoute access while retaining sessions, sign-in and shared links.')}</p></Disclosure>}
  </section>;
}
