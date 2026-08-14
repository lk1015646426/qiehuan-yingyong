import { invoke } from '@tauri-apps/api/core';
import type {
  WorkCnInstallation,
  WorkCnAccountView,
  WorkCnSnapshotValidation,
  WorkCnSwitchResult,
  WorkCnCommandError,
} from '../types/workCn';

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

// The Tauri command rejects with a JSON-string `WorkCnCommandError`. Re-parse it
// so the UI can branch on `code` instead of scanning Chinese text.
export function parseWorkCnCommandError(err: unknown): WorkCnCommandError {
  const raw = err instanceof Error ? err.message : typeof err === 'string' ? err : JSON.stringify(err);
  try {
    const parsed = JSON.parse(raw) as Partial<WorkCnCommandError>;
    if (parsed && typeof parsed.code === 'string' && typeof parsed.message === 'string') {
      return {
        code: parsed.code as WorkCnCommandError['code'],
        message: parsed.message,
        detail: parsed.detail ?? null,
      };
    }
  } catch {
    // fall through to generic error
  }
  return { code: 'LAUNCH_FAILED', message: raw, detail: null };
}
