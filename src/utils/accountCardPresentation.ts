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
