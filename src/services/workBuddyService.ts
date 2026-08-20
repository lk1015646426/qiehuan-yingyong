import { invoke } from '@tauri-apps/api/core';
import type {
  WorkBuddyAccountUpdate,
  WorkBuddyAccountView,
  WorkBuddyInstallation,
  WorkBuddySettings,
  WorkBuddySessionWatchStatus,
  WorkBuddySwitchResult,
  WorkBuddyCommandError,
  WorkBuddyAccountStatus,
} from '../types/workbuddy';

export const WORKBUDDY_CHANGED_EVENT = 'workbuddy:changed';

export function parseWorkBuddyCommandError(error: unknown): Error & { code?: string } {
  const raw = error instanceof Error
    ? error.message
    : typeof error === 'string'
      ? error
      : JSON.stringify(error);
  try {
    const parsed = JSON.parse(raw) as Partial<WorkBuddyCommandError>;
    if (typeof parsed.code === 'string' && typeof parsed.message === 'string') {
      const result = new Error(parsed.detail ? `${parsed.message}（${parsed.detail}）` : parsed.message) as Error & { code?: string };
      result.code = parsed.code;
      return result;
    }
  } catch {
    // 普通字符串错误直接透传。
  }
  return new Error(raw);
}

export function getWorkBuddyInstallation(): Promise<WorkBuddyInstallation> {
  return invoke('get_workbuddy_installation');
}

export function getWorkBuddySettings(): Promise<WorkBuddySettings> {
  return invoke('get_workbuddy_settings');
}

export function saveWorkBuddySettings(settings: WorkBuddySettings): Promise<WorkBuddySettings> {
  return invoke('save_workbuddy_settings', { settings });
}

export function importCurrentWorkBuddyAccount(displayName?: string | null): Promise<WorkBuddyAccountView> {
  return invoke('import_current_workbuddy_account', { displayName: displayName ?? null });
}

export function listWorkBuddyAccounts(): Promise<WorkBuddyAccountView[]> {
  return invoke('list_workbuddy_accounts');
}

export function getWorkBuddyAccountStatus(accountId: string): Promise<WorkBuddyAccountStatus> {
  return invoke('get_workbuddy_account_status', { accountId });
}

export function getWorkBuddySessionWatchStatus(): Promise<WorkBuddySessionWatchStatus> {
  return invoke('get_workbuddy_session_watch_status');
}

export function startWorkBuddySessionWatcher(): Promise<WorkBuddySessionWatchStatus> {
  return invoke('start_workbuddy_session_watcher');
}

export function updateWorkBuddyAccount(accountId: string, update: WorkBuddyAccountUpdate): Promise<WorkBuddyAccountView> {
  return invoke('update_workbuddy_account', { accountId, update });
}

export function deleteWorkBuddyAccount(accountId: string): Promise<boolean> {
  return invoke('delete_workbuddy_account', { accountId });
}

export async function switchWorkBuddyAccount(accountId: string): Promise<WorkBuddySwitchResult> {
  try {
    return await invoke('switch_workbuddy_account', { accountId });
  } catch (error) {
    throw parseWorkBuddyCommandError(error);
  }
}

export function syncWorkBuddyGitHub(): Promise<void> {
  return invoke('sync_workbuddy_github');
}

export function triggerWorkBuddyCheckin(accountId: string): Promise<void> {
  return invoke('trigger_workbuddy_checkin', { accountId });
}
