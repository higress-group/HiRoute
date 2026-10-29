/** Match AgentPlanDisplayName's domain rules without sending user input to a service. */
export function planDisplayNameProblem(value: string): 'empty' | 'length' | 'spacing' | 'characters' | 'sensitive' | null {
  const scalars = Array.from(value);
  if (!scalars.length) return 'empty';
  if (/^\p{White_Space}|\p{White_Space}$/u.test(value)) return 'spacing';
  if (/\p{Cc}/u.test(value) || scalars.some(char => {
    const point = char.codePointAt(0)!;
    return point >= 0xd800 && point <= 0xdfff;
  })) return 'characters';
  // Rust's to_ascii_lowercase does not case-fold non-ASCII characters.
  const folded = value.replace(/[A-Z]/g, char => char.toLowerCase());
  if (folded.includes('://') || [
    'authorization: bearer ', 'api_key=', 'api-key=', 'credential_value=',
    '-----begin private key-----', '-----begin rsa private key-----',
  ].some(marker => folded.includes(marker)) || folded.split(/[\p{White_Space}=:]/u).some(part => /^sk-[a-z0-9]{20,}$/.test(part))) return 'sensitive';
  if (scalars.length > 128) return 'length';
  return null;
}

export function planDisplayNameError(value: string, language: 'zh' | 'en'): string | null {
  const problem = planDisplayNameProblem(value);
  const messages = {
    empty: ['请填写路由名称', 'Enter a route name'],
    length: ['路由名称最多 128 个字符', 'Use at most 128 characters for the route name'],
    spacing: ['去除名称首尾空格', 'Remove leading and trailing whitespace'],
    characters: ['名称不能包含控制字符或无效字符', 'Remove control or invalid characters from the name'],
    sensitive: ['名称不能包含网址或敏感信息', 'The name cannot contain a URL or sensitive information'],
  };
  return problem ? messages[problem][language === 'zh' ? 0 : 1] : null;
}
