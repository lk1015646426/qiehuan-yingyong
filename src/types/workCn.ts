// Detection result for the local TRAE Work CN install.
// Mirrors `WorkCnInstallation` in src-tauri/src/models/work_cn.rs.
export interface WorkCnInstallation {
  installed: boolean;
  executablePath: string | null;
  userDataDir: string | null;
  storagePath: string | null;
  displayName: string | null;
  version: string | null;
  legacyPath: boolean;
}

// Snapshot completeness validation returned by the backend.
// Mirrors `WorkCnSnapshotValidation` in src-tauri/src/models/work_cn.rs.
export interface WorkCnSnapshotValidation {
  validForSwitch: boolean;
  hasAccessToken: boolean;
  hasRefreshToken: boolean;
  hasUserId: boolean;
  hasAuthDeviceId: boolean;
  hasDevicePrivateKey: boolean;
  hasDevicePublicKey: boolean;
  hasCheckinDeviceId: boolean;
  warnings: string[];
}

// Desensitized view of a saved Work CN account. Tokens / private keys are
// never exposed to the frontend. Mirrors `WorkCnAccountView`.
export interface WorkCnAccountView {
  id: string;
  email: string | null;
  userId: string | null;
  nickname: string | null;
  tags: string[] | null;
  planType: string | null;
  createdAt: number;
  lastUsed: number;
  hasAccessToken: boolean;
  hasRefreshToken: boolean;
  hasUserId: boolean;
  hasAuthDeviceId: boolean;
  hasCheckinDeviceId: boolean;
  hasMachineId: boolean;
  hasDevicePrivateKey: boolean;
  hasDevicePublicKey: boolean;
  validForSwitch: boolean;
  warnings: string[];
}

// Result of a one-click Work CN account switch.
// Mirrors `WorkCnSwitchResult` in src-tauri/src/models/work_cn.rs.
export interface WorkCnSwitchResult {
  accountId: string;
  userId: string | null;
  launched: boolean;
  verified: boolean;
  githubSynced: boolean;
  warning: string | null;
}

// Structured error code returned by Work CN commands. The backend serializes
// `WorkCnCommandError` to a JSON string; the frontend parses it to branch on
// `code` instead of matching Chinese text. Mirrors `WorkCnErrorCode`.
export type WorkCnErrorCode =
  | 'ACCOUNT_NOT_FOUND'
  | 'SNAPSHOT_INCOMPLETE'
  | 'CLIENT_NOT_INSTALLED'
  | 'CLIENT_CLOSE_FAILED'
  | 'STORAGE_BACKUP_FAILED'
  | 'INJECT_FAILED'
  | 'LAUNCH_FAILED'
  | 'VERIFY_TIMEOUT'
  | 'VERIFY_ACCOUNT_MISMATCH'
  | 'ROLLBACK_FAILED'
  | 'BUSY';

export interface WorkCnCommandError {
  code: WorkCnErrorCode;
  message: string;
  detail: string | null;
}

// Credit balance summary for a Work CN account. Mirrors `WorkCnCreditsSummary`
// in src-tauri/src/models/work_cn.rs. `total`/`remaining` are `null` when there
// is no parsed entitlement data (UI shows "暂无积分数据"); `unlimited` means an
// infinite `-1` quota (UI shows "无限"). `remaining` is always >= 0.
export interface WorkCnCreditsSummary {
  total: number | null;
  used: number;
  remaining: number | null;
  unlimited: boolean;
  updatedAt: number;
}

// One GitHub Secrets slot binding (开发指南 §8.5 / 阶段 6).
export interface WorkCnGitHubSlot {
  slot: number;
  accountId: string;
  tokenSecret: string;
  deviceSecret: string;
}

// Persisted GitHub Secrets sync config (never holds a PAT).
export interface WorkCnGitHubConfig {
  enabled: boolean;
  repository: string;
  slots: WorkCnGitHubSlot[];
}

// Result of syncing one account's credentials to GitHub.
export interface WorkCnGitHubSyncResult {
  accountId: string;
  synced: boolean;
  skipped: boolean;
  skipReason: string | null;
  error: string | null;
  syncedAt: number;
}

// GitHub CLI availability / auth status for the settings dialog.
export interface WorkCnGitHubCliStatus {
  available: boolean;
  authed: boolean;
  detail: string | null;
}
