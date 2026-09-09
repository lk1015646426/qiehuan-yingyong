// GhSetupDialog 配置保存的纯逻辑（两种模式，供组件与测试共用）。
//
// - full（TRAE 页）：按勾选的「账号 → 槽位号」重建槽位列表，并保留每个账号
//   已保存的自动签到开关（checkinEnabled，2026-09-09 新增）。
// - repo-only（WorkBuddy / 智谱页）：只改 启用/仓库/workflow 文件。
//   红线：槽位列表原样透传，绝不清空 TRAE 页已绑定的槽位——WB/智谱走聚合
//   Secret，没有槽位概念，不能因在这里保存仓库配置就把 TRAE 的绑定抹掉。
import type { WorkCnGitHubConfig, WorkCnGitHubSlot } from '../types/workCn';

export type GhSetupMode = 'full' | 'repo-only';

export function buildGhSetupConfig(params: {
  mode: GhSetupMode;
  enabled: boolean;
  repository: string;
  workflowFile: string;
  existingConfig: WorkCnGitHubConfig;
  /** full 模式专用：勾选账号 → 槽位号（按账号列表顺序分配）。 */
  slotNumbers?: Map<string, number>;
}): WorkCnGitHubConfig {
  const repository = params.repository.trim();
  const workflowFile =
    params.workflowFile || params.existingConfig.workflowFile || 'daily-checkin.yml';
  if (params.mode === 'repo-only') {
    return {
      enabled: params.enabled,
      repository,
      slots: params.existingConfig.slots,
      workflowFile,
    };
  }
  const slots: WorkCnGitHubSlot[] = [];
  for (const [accountId, slot] of params.slotNumbers ?? []) {
    slots.push({
      slot,
      accountId,
      tokenSecret: '',
      deviceSecret: '',
      checkinEnabled:
        params.existingConfig.slots.find((s) => s.accountId === accountId)?.checkinEnabled ?? true,
    });
  }
  return { enabled: params.enabled, repository, slots, workflowFile };
}

// 保存前置校验：两种模式都要求仓库合法；full 模式启用同步时必须至少绑定一个
// 槽位（槽位制同步），repo-only 不依赖槽位（聚合 Secret 制）。
export function validateGhSetupConfig(mode: GhSetupMode, config: WorkCnGitHubConfig): string | null {
  if (!config.enabled) return null;
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(config.repository)) {
    return '仓库需为 owner/repo 形式';
  }
  if (mode === 'full' && config.slots.length === 0) {
    return '启用同步时至少要绑定一个账号到槽位';
  }
  return null;
}

// 「③ 同步配置」行的就绪判定（驱动弹窗里的 已配置/未配置 徽标）。
export function ghSetupConfigReady(mode: GhSetupMode, config: WorkCnGitHubConfig): boolean {
  if (!config.enabled) return false;
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(config.repository)) return false;
  return mode === 'repo-only' || config.slots.length > 0;
}
