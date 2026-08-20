export interface WorkBuddyInstallation {
  installed: boolean;
  executablePath: string | null;
  authFilePath: string;
  running: boolean;
  currentAccountId: string | null;
  githubCleanupPending: boolean;
}

export interface WorkBuddySettings {
  executablePath: string | null;
  authFilePath: string | null;
}

export interface WorkBuddySessionWatchStatus {
  running: boolean;
  lastCheckAt: number;
  outcome: 'IDLE' | 'NO_AUTH_FILE' | 'UNCHANGED' | 'NO_MATCH' | 'SNAPSHOT_UPDATED' | 'TOKEN_UPDATED' | 'FAILED';
  accountId: string | null;
  tokenChanged: boolean;
  githubSynced: boolean;
  githubError: string | null;
  message: string;
}

export interface WorkBuddyAccountView {
  id: string;
  uid: string;
  uin: string | null;
  displayName: string;
  maskedPhone: string | null;
  checkinEnabled: boolean;
  tokenExpiresAt: number | null;
  createdAt: number;
  updatedAt: number;
  lastUsedAt: number | null;
  lastGithubSyncAt: number | null;
  lastGithubSyncState: 'pending' | 'synced' | 'failed' | string;
  lastGithubSyncError: string | null;
}

export interface WorkBuddyAccountUpdate {
  displayName?: string;
  checkinEnabled?: boolean;
}

export interface WorkBuddyAccountStatus {
  credits: number | null;
  todayReward: number | null;
  streakDays: number | null;
  updatedAt: number;
  creditsError: string | null;
  activityError: string | null;
}

export interface WorkBuddySwitchResult {
  transactionId: string;
  accountId: string;
  verifiedUid: string;
  forcedClose: boolean;
  githubSyncPending: boolean;
}

export type WorkBuddyErrorCode =
  | 'ACCOUNT_NOT_FOUND'
  | 'AUTH_FILE_NOT_FOUND'
  | 'AUTH_FILE_INVALID'
  | 'SNAPSHOT_INCOMPLETE'
  | 'CLIENT_NOT_INSTALLED'
  | 'CLIENT_CLOSE_FAILED'
  | 'SWITCH_CANCELLED'
  | 'BACKUP_FAILED'
  | 'INJECT_FAILED'
  | 'LAUNCH_FAILED'
  | 'WINDOW_ACTIVATION_FAILED'
  | 'VERIFY_TIMEOUT'
  | 'VERIFY_ACCOUNT_MISMATCH'
  | 'ROLLBACK_FAILED'
  | 'GITHUB_NOT_READY'
  | 'GITHUB_SYNC_FAILED'
  | 'WORKFLOW_TRIGGER_FAILED'
  | 'BUSY';

export interface WorkBuddyCommandError {
  code: WorkBuddyErrorCode;
  message: string;
  detail?: string | null;
  transactionId?: string | null;
}
