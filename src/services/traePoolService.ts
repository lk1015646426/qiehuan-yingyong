import { invoke } from '@tauri-apps/api/core';
import type { TraePoolLaunchResult, TraePoolStatus } from '../types/traePool';
import type { TraeModelPoolModels, TraeModelPoolStatus } from '../types/traePool';

export function getTraeModelPoolStatus(): Promise<TraeModelPoolStatus> { return invoke('get_trae_model_pool_status'); }
export function getTraeModelPoolModels(): Promise<TraeModelPoolModels> { return invoke('get_trae_model_pool_models'); }
export function saveTraeModelPoolModels(modelIds: string[]): Promise<TraeModelPoolModels> { return invoke('save_trae_model_pool_models', { modelIds }); }
export function startTraeModelPool(): Promise<TraeModelPoolStatus> { return invoke('start_trae_model_pool'); }
export function stopTraeModelPool(): Promise<TraeModelPoolStatus> { return invoke('stop_trae_model_pool'); }
export function registerTraeModelPoolModels(accountId: string): Promise<{ registered: string[]; alreadyRegistered: string[]; endpoint: string }> {
  return invoke('register_trae_model_pool_models', { accountId });
}

// platformId 留空时后端固定按 TRAE Work CN（trae_solo_cn）处理，
// 与本 fork 只会操作的工作 CN 客户端保持一致。
export function getTraePoolStatus(platformId?: string | null): Promise<TraePoolStatus> {
  return invoke<TraePoolStatus>('get_trae_pool_status', { platformId: platformId ?? null });
}

export function ensureTraePoolInstance(
  accountId: string,
  platformId?: string | null,
): Promise<{ id: string; name: string; userDataDir: string; bindAccountId: string | null }> {
  return invoke('ensure_trae_pool_instance', { platformId: platformId ?? null, accountId });
}

export function startTraePoolAccount(
  accountId: string,
  platformId?: string | null,
): Promise<TraePoolLaunchResult> {
  return invoke<TraePoolLaunchResult>('start_trae_pool_account', {
    platformId: platformId ?? null,
    accountId,
  });
}
