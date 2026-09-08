import { invoke } from '@tauri-apps/api/core';
import type {
  ZhipuAccountStatus,
  ZhipuAccountUpdate,
  ZhipuAccountView,
  ZhipuImportNotice,
} from '../types/zhipu';

export function listZhipuAccounts(): Promise<ZhipuAccountView[]> {
  return invoke('list_zhipu_accounts');
}

export function importCurrentZhipuAccount(
  displayName?: string | null,
): Promise<[ZhipuAccountView, ZhipuImportNotice]> {
  return invoke('import_current_zhipu_account', { displayName: displayName ?? null });
}

export function importZhipuAccount(
  accessToken: string,
  refreshToken?: string | null,
  displayName?: string | null,
): Promise<[ZhipuAccountView, ZhipuImportNotice]> {
  return invoke('import_zhipu_account', {
    accessToken,
    refreshToken: refreshToken ?? null,
    displayName: displayName ?? null,
  });
}

export function updateZhipuAccount(
  accountId: string,
  update: ZhipuAccountUpdate,
): Promise<ZhipuAccountView> {
  return invoke('update_zhipu_account', { accountId, update });
}

export function deleteZhipuAccount(accountId: string): Promise<boolean> {
  return invoke('delete_zhipu_account', { accountId });
}

export function getZhipuAccountStatus(accountId: string): Promise<ZhipuAccountStatus> {
  return invoke('get_zhipu_account_status', { accountId });
}

export function syncZhipuGitHub(): Promise<void> {
  return invoke('sync_zhipu_github');
}

export function triggerZhipuCheckin(accountId: string): Promise<void> {
  return invoke('trigger_zhipu_checkin', { accountId });
}
