import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import type {
  WorkCnInstallation,
  WorkCnAccountView,
  WorkCnCreditsSummary,
  WorkCnSnapshotValidation,
  WorkCnSwitchResult,
  WorkCnCommandError,
  WorkCnGitHubConfig,
  WorkCnGitHubCliStatus,
  WorkCnGitHubSyncResult,
  WorkCnSessionWatchStatus,
} from '../types/workCn';

// 后台会话监测事件名（与 Rust `SESSION_WATCH_EVENT` 保持一致，避免字符串漂移）。
export const WORK_CN_SESSION_WATCH_EVENT = 'work-cn:session-watch';

// Probe the local TRAE Work CN install. Safe to call on every page load:
// the backend never touches login secrets here.
export function getWorkCnInstallation(): Promise<WorkCnInstallation> {
  return invoke<WorkCnInstallation>('get_work_cn_installation');
}

// Import the currently logged-in TRAE Work CN account as a full snapshot
// (tokens + device keys + ids). `label` is stored as a tag; never overwrites email.
export function importCurrentWorkCnAccount(label?: string | null): Promise<WorkCnAccountView> {
  return invoke<WorkCnAccountView>('import_current_work_cn_account', { label: label ?? null });
}

// List previously imported Work CN accounts (desensitized views).
export function listWorkCnAccounts(): Promise<WorkCnAccountView[]> {
  return invoke<WorkCnAccountView[]>('list_work_cn_accounts');
}

// Validate the completeness of a saved Work CN account snapshot.
export function validateWorkCnAccount(accountId: string): Promise<WorkCnSnapshotValidation> {
  return invoke<WorkCnSnapshotValidation>('validate_work_cn_account', { accountId });
}

// One-click switch to a saved Work CN account and open the official client.
// Orchestrates close → inject → bind → launch → verify → rollback on failure.
// On error the backend returns a serialized `WorkCnCommandError` JSON string.
export async function switchWorkCnAccount(accountId: string): Promise<WorkCnSwitchResult> {
  try {
    return await invoke<WorkCnSwitchResult>('switch_work_cn_account', { accountId });
  } catch (err) {
    throw parseWorkCnCommandError(err);
  }
}

// Query a saved Work CN account's credit balance. Query only — never performs a
// local check-in / claim. `forceRefresh` re-queries the upstream quota API using
// the cached access token; any error is re-thrown as a parsed `WorkCnCommandError`.
export async function getWorkCnCredits(
  accountId: string,
  forceRefresh = false,
): Promise<WorkCnCreditsSummary> {
  try {
    return await invoke<WorkCnCreditsSummary>('get_work_cn_credits', {
      accountId,
      forceRefresh,
    });
  } catch (err) {
    throw parseWorkCnCommandError(err);
  }
}

// The Tauri command rejects with a JSON-string `WorkCnCommandError`. Re-parse it
// and surface as a real `Error` so UI banners show the human-readable message
// (plus detail) instead of "[object Object]". `code` is kept as a property for
// any future branching.
export function parseWorkCnCommandError(err: unknown): Error & { code?: string } {
  const raw = err instanceof Error ? err.message : typeof err === 'string' ? err : JSON.stringify(err);
  try {
    const parsed = JSON.parse(raw) as Partial<WorkCnCommandError>;
    if (parsed && typeof parsed.code === 'string' && typeof parsed.message === 'string') {
      const message = parsed.detail ? `${parsed.message}（${parsed.detail}）` : parsed.message;
      const error = new Error(message) as Error & { code?: string };
      error.code = parsed.code;
      return error;
    }
  } catch {
    // fall through to generic error
  }
  return new Error(raw);
}

// ---- Stage 6: GitHub Secrets 同步 ----

// Read the persisted GitHub Secrets sync configuration (settings dialog prefill).
export function getWorkCnGitHubConfig(): Promise<WorkCnGitHubConfig> {
  return invoke<WorkCnGitHubConfig>('get_work_cn_github_config');
}

// Validate and persist the GitHub Secrets sync configuration.
export function saveWorkCnGitHubConfig(config: WorkCnGitHubConfig): Promise<void> {
  return invoke<void>('save_work_cn_github_config', { config });
}

// Report GitHub CLI availability / auth status (no secrets touched).
export function getWorkCnGitHubCliStatus(): Promise<WorkCnGitHubCliStatus> {
  return invoke<WorkCnGitHubCliStatus>('github_cli_status');
}

// gh 自动安装进度事件名（与 Rust `GH_SETUP_EVENT` 保持一致）。
export const GH_SETUP_EVENT = 'gh-setup:progress';

// gh 安装进度事件载荷。
export interface GhSetupProgress {
  phase: 'downloading' | 'installing' | 'done' | 'failed';
  received: number;
  total: number;
}

// 自动下载官方 gh MSI 并静默安装（进度经 GH_SETUP_EVENT 推送）。
export function setupGhCli(): Promise<void> {
  return invoke<void>('gh_cli_setup_download');
}

// 用 PAT 完成 gh 登录（Token 只走 stdin，绝不落盘）。
export function ghLoginWithToken(token: string): Promise<void> {
  return invoke<void>('gh_cli_login_with_token', { token });
}

// Sync one account's credentials to its bound GitHub slot. Never claims a
// check-in. On error the backend returns a plain string, which we surface as-is.
export async function syncWorkCnGitHubAccount(accountId: string): Promise<WorkCnGitHubSyncResult> {
  try {
    return await invoke<WorkCnGitHubSyncResult>('sync_work_cn_github_account', { accountId });
  } catch (err) {
    throw err instanceof Error ? err.message : String(err);
  }
}

// ---- Stage 7: 后台会话监测 ----

// Read the current background session-watch status (desensitized, no tokens).
export function getWorkCnSessionWatchStatus(): Promise<WorkCnSessionWatchStatus> {
  return invoke<WorkCnSessionWatchStatus>('get_work_cn_session_watch_status');
}

// ---- Stage 8: 清除本地凭证 ----

// 清除本地保存的全部 TRAE Work CN 凭证（账号库 + GitHub 同步配置）。
// 危险操作，设置弹窗必须二次确认后才可调用。
export function clearWorkCnCredentials(): Promise<void> {
  return invoke<void>('clear_work_cn_credentials');
}

// 删除单个已导入的 Work CN 账号槽位（解绑 GitHub 槽位；不触碰客户端与远端 Secrets）。
export function deleteWorkCnAccount(accountId: string): Promise<void> {
  return invoke<void>('delete_work_cn_account', { accountId });
}

// 在系统文件管理器中打开应用日志目录。
export function openWorkCnLogFolder(): Promise<void> {
  return invoke<void>('open_log_folder');
}

// 用系统默认浏览器打开外部链接。WebView 内 `<a target="_blank">` 不会唤起
// 浏览器，必须经 opener 插件（权限已含于 opener:default）。
export function openExternalUrl(url: string): Promise<void> {
  return openUrl(url);
}
