import { scenario } from './scenario-selection.mjs';

export function nativeConnectionScenarios({ fresh, click, until, text, calls, control }) {
  const c = control;
  const assert = (value, message) => { if (!value) throw new Error(message); };
  async function lose() {
    c().desktop.native_connection = 'recovery_required';
    c().desktop.trusted_authority = false;
    c().handlers.observation_read = () => { throw { source: 'native', code: 'TRUSTED_AUTHORITY_UNAVAILABLE' }; };
    await c().callHost('observation_read', {}).catch(() => {});
    await until(() => document.querySelector('[data-connection-state="recovery_required"]'), 'lost native connection');
  }
  return [
    scenario('desktop.connection.loss-retry-recovery', ['native-recovery'], 'Live service with lost authority shows accurate read errors and only explicit recovery restores it', async () => {
      await fresh(); await lose(); await click('会话');
      await until(() => text().includes('暂时无法读取记录'), 'accurate observation error');
      const health = document.querySelector('.sidebar-health')?.textContent ?? '';
      assert(health.includes('需要恢复') && !health.includes('只读'), 'Lost authority masquerades as a healthy or intentionally read-only service');
      const retry = [...document.querySelectorAll('button')].find(button => button.textContent.trim() === '重试');
      assert(retry, 'Read retry missing'); retry.click();
      await until(() => text().includes('暂时无法读取记录'), 'retry stays refused');
      assert(calls('recover_native_connection').length === 0, 'Read retry silently restarted the service');
      c().handlers.recover_native_connection = async payload => {
        assert(Object.keys(payload).length === 0, 'WebView supplied native authority or filesystem parameters');
        c().desktop.native_connection = 'ready'; c().desktop.trusted_authority = true;
        delete c().handlers.observation_read;
      };
      await click('恢复本机连接');
      await until(() => !document.querySelector('.native-connection-recovery'), 'recovered status');
      assert(calls('recover_native_connection').length === 1, 'Recovery was repeated');
    }),
    scenario('desktop.connection.busy-and-failed-recovery', ['native-recovery'], 'Busy, pending and failed recovery preserve an explicit action and prevent duplicate submission', async () => {
      await fresh(); await lose();
      let release;
      c().handlers.recover_native_connection = () => new Promise((_, reject) => { release = reject; });
      await click('恢复本机连接');
      const button = document.querySelector('.native-connection-recovery button');
      assert(button.disabled, 'Concurrent recovery remains enabled'); button.click();
      assert(calls('recover_native_connection').length === 1, 'Concurrent recovery submitted twice');
      release({ source: 'native', code: 'AUTHORITY_RECOVERY_WORK_ACTIVE' });
      await until(() => text().includes('仍有请求或任务运行'), 'active work guidance');
      assert(c().desktop.native_connection === 'recovery_required', 'Busy work became a successful connection');
      c().handlers.recover_native_connection = () => { throw { source: 'native', code: 'UPGRADE_STOP_PENDING' }; };
      await click('恢复本机连接'); await until(() => text().includes('正在等待原服务退出'), 'pending stop guidance');
      c().desktop.native_connection = 'restart_required';
      await c().callHost('observation_read', {}).catch(() => {});
      await until(() => document.querySelector('[data-connection-state="restart_required"]'), 'failed maintenance state');
      assert(!document.querySelector('.native-connection-recovery button'), 'Failed maintenance offered unsafe channel reuse');
    }),
  ];
}
