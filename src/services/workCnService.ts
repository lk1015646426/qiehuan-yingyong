import { invoke } from '@tauri-apps/api/core';
import type { WorkCnInstallation, WorkCnAccountView, WorkCnSnapshotValidation } from '../types/workCn';

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
