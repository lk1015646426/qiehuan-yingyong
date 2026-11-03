// TRAE 账号池视图模型，镜像 src-tauri/src/modules/trae_pool.rs。
// 注意：池按「账号 → 独立实例」路由会话，不做对话内跨账号续接。

export interface TraePoolAccountView {
  accountId: string;
  label: string;
  email: string | null;
  planType: string | null;
  /** 剩余积分；未知时为 null，不要当成 0。 */
  creditsRemaining: number | null;
  creditsUnlimited: boolean;
  creditsUpdatedAt: number | null;
  tokenExpiresAt: number | null;
  tokenDaysLeft: number | null;
  /** 快照是否具备切换条件（令牌 + 设备标识 + 设备密钥）。 */
  switchable: boolean;
  warnings: string[];
  instanceId: string | null;
  instanceName: string | null;
  instanceRunning: boolean;
  rank: number;
}

export interface TraePoolStatus {
  accounts: TraePoolAccountView[];
  recommendedAccountId: string | null;
  totalAccounts: number;
  readyAccounts: number;
  runningInstances: number;
  instanceRoot: string | null;
}

export interface TraePoolLaunchResult {
  accountId: string;
  instanceId: string;
  instanceName: string;
  launched: boolean;
}

export interface TraeModelPoolModel {
  id: string; name: string; multimodal: boolean; tools: boolean | null;
  inputLimit: number | null; outputLimit: number | null;
}
export interface TraeModelPoolModels {
  models: TraeModelPoolModel[]; selectedModelIds: string[];
}
export interface TraeModelPoolStatus {
  running: boolean; stopping: boolean; endpoint: string;
  requests: number; rotations: number; activeRequests: number;
  coolingAccounts: number; selectedModelIds: string[]; lastError: string | null;
}
