// 云端签到面板服务层：触发 workflow / 查询运行状态。
// 签到本身永远在云端 Actions 执行，这里只做触发（验证/补签）与状态查询；
// 例外：localCheckin 是本地直接签到入口（诊断/补签，用户 2026-08-16 决策）。

import { invoke } from '@tauri-apps/api/core';
import type { CheckinWorkflowRun, LocalCheckinResult } from '../types/checkin';

/** 手动触发云端签到 workflow（凭证修复后的即时验证/补签）。 */
export function triggerCheckinWorkflow(): Promise<void> {
  return invoke<void>('trigger_checkin_workflow');
}

/** 查询签到 workflow 最近运行状态（默认 5 条）。 */
export function listCheckinWorkflowRuns(limit?: number): Promise<CheckinWorkflowRun[]> {
  return invoke<CheckinWorkflowRun[]>('list_checkin_workflow_runs', { limit: limit ?? 5 });
}

/**
 * 本地单账号签到（诊断/补签）：从本机直接调用 TRAE 签到 API。
 * 注意：有意打破「本地绝不签到/claim」原则的诊断入口（用户 2026-08-16 决策），
 * 与云端 Actions 的运行环境和请求标识形成对照，用于定位『操作太过频繁』的来源。
 */
export function localCheckin(accountId: string): Promise<LocalCheckinResult> {
  return invoke<LocalCheckinResult>('local_checkin_work_cn', { accountId });
}
