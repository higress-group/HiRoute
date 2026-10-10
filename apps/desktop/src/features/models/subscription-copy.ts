export function subscriptionAttentionCopy(
  reason: string | null | undefined,
  language: 'zh' | 'en',
) {
  const zh = language === 'zh';
  switch (reason) {
    case 'subscription_updating':
      return { title: zh ? '正在更新订阅授权' : 'Updating subscription access', detail: zh ? '更新完成前不会使用旧授权发起新请求。' : 'New requests will not use the previous authorization while the update is pending.' };
    case 'authentication_required':
      return { title: zh ? '需要更新订阅登录' : 'Subscription sign-in required', detail: zh ? '请更新对应的订阅登录，再重新检查。复用本机登录时，请在原生客户端中登录。' : 'Renew the subscription sign-in, then check again. For a reused local sign-in, sign in through the native client.' };
    case 'model_not_allowed':
      return { title: zh ? '当前订阅不再允许此模型' : 'Model unavailable for this subscription', detail: zh ? '模型和路由配置已保留；当前授权不会执行它。' : 'The model and routing configuration are retained, but current access cannot execute it.' };
    case 'runtime_unavailable':
      return { title: zh ? '订阅服务暂不可用' : 'Subscription service unavailable', detail: zh ? '请确认本机服务和网络后重试。' : 'Confirm the local service and network, then try again.' };
    default:
      return { title: zh ? '订阅需要处理' : 'Subscription needs attention', detail: zh ? '其他已连接模型不受影响。' : 'Other connected models are unaffected.' };
  }
}
