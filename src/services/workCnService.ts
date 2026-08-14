import { invoke } from '@tauri-apps/api/core';
import type { WorkCnInstallation } from '../types/workCn';

// Probe the local TRAE Work CN install. Safe to call on every page load:
// the backend never touches login secrets here.
export function getWorkCnInstallation(): Promise<WorkCnInstallation> {
  return invoke<WorkCnInstallation>('get_work_cn_installation');
}
