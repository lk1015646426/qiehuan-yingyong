export type GitHubSyncTone = 'pending' | 'syncing' | 'synced' | 'failed';

export interface GitHubSyncPresentation {
  label: string;
  tone: GitHubSyncTone;
  detail: string | null;
}

export function compactUid(uid: string | null | undefined): string {
  if (!uid) return '未知';
  return uid.length <= 12 ? uid : `${uid.slice(0, 6)}…${uid.slice(-4)}`;
}

export function githubSyncPresentation(
  state: string | null | undefined,
  error: string | null | undefined,
): GitHubSyncPresentation {
  if (state === 'syncing') return { label: 'GitHub 同步中', tone: 'syncing', detail: null };
  if (state === 'synced') return { label: 'GitHub 已同步', tone: 'synced', detail: null };
  if (state === 'failed') return { label: 'GitHub 失败', tone: 'failed', detail: error?.trim() || null };
  return { label: 'GitHub 待同步', tone: 'pending', detail: null };
}

// 凭证失效关键字需与后端保持一致：
// - workbuddy_status.rs query_json 401 => "认证已失效，请重新登录 WorkBuddy 后更新账号"
// - workbuddy_account.rs refresh_error_requires_login => "refresh token 已失效" / "缺少 refresh token" / "refresh token 无效"
// 网络/超时类刷新失败不在此列，不应标记凭证失效。
const CREDENTIAL_INVALID_MARKERS = [
  '认证已失效',
  'refresh token 已失效',
  '缺少 refresh token',
  'refresh token 无效',
];

export function credentialInvalidated(
  errors: ReadonlyArray<string | null | undefined>,
): boolean {
  const haystack = errors
    .filter((error): error is string => Boolean(error))
    .join('\n')
    .toLowerCase();
  return CREDENTIAL_INVALID_MARKERS.some((marker) => haystack.includes(marker));
}
