// 云端签到类型（自独立面板合并至 TRAE 页）。Mirrors `CheckinWorkflowRun` in src-tauri/src/models/work_cn.rs.

// 一次签到 workflow 运行（gh run list --json）。
export interface CheckinWorkflowRun {
  databaseId: number;
  /** queued / in_progress / completed */
  status: string;
  /** success / failure / cancelled / null（未结束） */
  conclusion: string | null;
  createdAt: string;
  displayTitle: string;
  /** schedule / workflow_dispatch */
  event: string;
  url: string;
}

// 半自动刷新流的阶段（凭证健康卡片按钮文案随之变化）。
export type RefreshFlowPhase =
  | 'switching' // 切换到目标账号并打开客户端
  | 'waiting' // 等待客户端刷新 token（最长 3 分钟）
  | 'syncing' // 同步新凭证到 GitHub Secrets
  | 'done' // 完成，可切回原账号
  | 'failed'; // 任一步失败

export interface RefreshFlowState {
  accountId: string;
  phase: RefreshFlowPhase;
  message: string;
  startedAt: number;
  /** 流程开始时的活跃账号 id，用于完成后的「切回」按钮；可能为 null。 */
  previousAccountId: string | null;
}

// 本地单账号签到（诊断/补签）结果。Mirrors `WorkCnLocalCheckinResult` in src-tauri/src/models/work_cn.rs.
export interface LocalCheckinResult {
  accountId: string;
  ok: boolean;
  message: string;
  credits?: number | null;
  alreadyCheckedIn: boolean;
  outcome: 'already_checked_in' | 'claimed' | 'failed';
  stage: 'status' | 'claim' | 'usage';
  businessCode?: number | null;
  httpStatus?: number | null;
  claimAttempted: boolean;
  tokenExpiryState: 'valid' | 'expired' | 'unknown';
  devicePresent: boolean;
}
