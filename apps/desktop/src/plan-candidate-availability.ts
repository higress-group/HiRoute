export type UnavailableCandidate = { binding_id: string; display_name: string; reason: string };

/** Closed backend reasons only; never display a provider error or credential-bearing payload. */
export function candidateUnavailableMessage(reason: string, language: 'zh' | 'en'): string {
  const messages: Record<string, [string, string]> = {
    invalid_model_id: ['模型 ID 格式不受支持，请在模型页面核对上游实际 ID。', 'Unsupported model ID. Check the upstream model ID on the Models page.'],
    source_not_ready: ['接入已停用或尚未就绪，请在模型页面启用或完成接入。', 'The connection is disabled or not ready. Enable or complete it on the Models page.'],
    model_not_eligible: ['当前来源尚未授权此模型执行，请重新检查接入。', 'This source does not currently authorize this model. Recheck the connection.'],
    catalog_mismatch: ['已保存接入与当前供应商目录不兼容，请重新检查接入。', 'The saved connection is incompatible with the current provider catalog. Recheck it.'],
    credential_unavailable: ['接入缺少可用凭据或授权，请在模型页面处理。', 'The connection needs credentials or authorization. Update it on the Models page.'],
    runtime_unavailable: ['订阅执行服务暂不可用，请检查订阅接入后重试。', 'The subscription runtime is unavailable. Check the subscription connection and retry.'],
    capability_unavailable: ['缺少路由所需的能力资料，请检查模型配置。', 'Required routing capability data is missing. Check the model configuration.'],
    invalid_configuration: ['当前接入配置无法用于路由，请在模型页面检查模型和协议设置。', 'This connection cannot be prepared for routing. Check its model and protocol settings on the Models page.'],
  };
  return (messages[reason] ?? messages.invalid_configuration)[language === 'zh' ? 0 : 1];
}
