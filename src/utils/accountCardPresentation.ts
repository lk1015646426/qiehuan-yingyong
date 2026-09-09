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

// Token 剩余天数预警（TRAE / WorkBuddy / 智谱 三页共用）。
// 阈值与云端 auto 刷新逻辑对齐：客户端剩 1/3 寿命时刷新；
// 剩余 ≤5 天预警、≤1 天（含已过期）危险。expiresAt 为秒级时间戳。
export type TokenTone = 'ok' | 'warn' | 'danger' | 'unknown';

const TOKEN_WARN_DAYS = 5;
const TOKEN_DANGER_DAYS = 1;

export function tokenDaysLeft(expiresAt: number | null): { tone: TokenTone; days: number | null } {
  if (!expiresAt) {
    return { tone: 'unknown', days: null };
  }
  const msLeft = expiresAt * 1000 - Date.now();
  if (msLeft <= 0) {
    return { tone: 'danger', days: 0 };
  }
  const days = Math.ceil(msLeft / 86_400_000);
  if (days <= TOKEN_DANGER_DAYS) {
    return { tone: 'danger', days };
  }
  if (days <= TOKEN_WARN_DAYS) {
    return { tone: 'warn', days };
  }
  return { tone: 'ok', days };
}

export function tokenDaysLabel(days: number | null): string {
  if (days == null) return '未知';
  return days <= 0 ? '已过期' : `${days} 天`;
}

// 「令牌到期」展示格式（三页共用）：只显示日期。完整时间戳太占空间且会被
// 截断，日期精度对凭证预警已足够（秒级时间戳入参）。
export function formatTokenExpiryDate(ts: number | null): string {
  if (!ts) {
    return '未知';
  }
  return new Date(ts * 1000).toLocaleDateString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  });
}
