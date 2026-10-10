import type { SubscriptionMode } from './types';

export function subscriptionAttentionCopy(
  reason: string | null | undefined,
  language: 'zh' | 'en',
  mode?: SubscriptionMode,
) {
  const zh = language === 'zh';
  switch (reason) {
    case 'subscription_updating':
      return { title: zh ? '正在更新订阅授权' : 'Updating subscription access', detail: zh ? '更新完成前不会使用旧授权发起新请求。' : 'New requests will not use the previous authorization while the update is pending.' };
    case 'authentication_required':
      return { title: zh ? '需要更新订阅登录' : 'Subscription sign-in required', detail: mode === 'cpa_managed'
        ? zh ? '请在 HiRoute 中重新独立登录，再检查并保存；现有模型和路由已保留。' : 'Sign in independently again in HiRoute, then check and save. Existing models and routes are retained.'
        : mode === 'native_borrowed'
          ? zh ? '请在原生客户端中更新登录后重新检查，或改用推荐的独立登录。' : 'Renew the native client sign-in and check again, or use the recommended independent sign-in.'
          : zh ? '请更新对应的订阅登录，再重新检查。复用本机登录时，请在原生客户端中登录。' : 'Renew the subscription sign-in, then check again. For a reused local sign-in, sign in through the native client.' };
    case 'model_not_allowed':
      return { title: zh ? '当前订阅不再允许此模型' : 'Model unavailable for this subscription', detail: zh ? '模型和路由配置已保留；当前授权不会执行它。' : 'The model and routing configuration are retained, but current access cannot execute it.' };
    case 'runtime_unavailable':
      return { title: zh ? '订阅服务暂不可用' : 'Subscription service unavailable', detail: zh ? '请确认本机服务和网络后重试。' : 'Confirm the local service and network, then try again.' };
    default:
      return { title: zh ? '订阅需要处理' : 'Subscription needs attention', detail: zh ? '其他已连接模型不受影响。' : 'Other connected models are unaffected.' };
  }
}

export function subscriptionFailureCopy(code: string, language: 'zh' | 'en'): string | null {
  const zh = language === 'zh';
  switch (code.toUpperCase()) {
    case 'SUBSCRIPTION_NATIVE_STORE_UNSUPPORTED':
      return zh ? '当前 Codex 凭据存储暂不支持复用。请使用推荐的独立登录。' : 'The selected Codex credential store cannot be reused. Use the recommended independent sign-in.';
    case 'SUBSCRIPTION_NATIVE_LOGIN_UNSUPPORTED':
      return zh ? '当前 Codex 登录不是可复用的 ChatGPT 订阅登录。请使用独立登录，API Key 可通过“连接 API”接入。' : 'This Codex sign-in is not a reusable ChatGPT subscription. Use independent sign-in, or Connect an API for an API key.';
    case 'SUBSCRIPTION_NATIVE_ACCOUNT_MISSING':
      return zh ? '无法识别当前 Codex 订阅账号。请在 Codex 中重新登录后重试，或使用独立登录。' : 'The Codex subscription account could not be identified. Sign in again in Codex and retry, or use independent sign-in.';
    case 'SUBSCRIPTION_NATIVE_READ_FAILED':
      return zh ? '无法读取本机登录。请确认凭据文件可访问，或解锁并允许读取钥匙串后重试；也可使用独立登录。' : 'The local sign-in could not be read. Check file access or unlock and allow Keychain access, then retry. Independent sign-in is also available.';
    case 'SUBSCRIPTION_NATIVE_LOGIN_MISSING':
    case 'SUBSCRIPTION_NATIVE_LOGIN_INVALID':
      return zh ? '本机订阅登录缺失、已过期或格式不支持。请在原生客户端中重新登录后检查，或使用独立登录。' : 'The local subscription sign-in is missing, expired or unsupported. Sign in again in the native client and check, or use independent sign-in.';
    case 'SUBSCRIPTION_NATIVE_CLIENT_UNAVAILABLE':
      return zh ? '无法读取所选 Codex 客户端信息。请确认客户端可运行后重试，或使用独立登录。' : 'The selected Codex client information is unavailable. Check that the client runs and retry, or use independent sign-in.';
    case 'SUBSCRIPTION_MANAGED_LOGIN_REQUIRED':
      return zh ? '独立登录已失效或凭据不可用。请在 HiRoute 中重新独立登录，再检查并保存。' : 'The independent sign-in has expired or its credentials are unavailable. Sign in again in HiRoute, then check and save.';
    case 'SUBSCRIPTION_RUNTIME_UNAVAILABLE':
      return zh ? '订阅服务暂不可用。请检查本机服务和网络后重试；现有模型和路由已保留。' : 'The subscription service is unavailable. Check the local service and network, then retry. Existing models and routes are retained.';
    default: return null;
  }
}
