export interface ZhipuAccountView {
  id: string;
  displayName: string;
  /** JWT sub 声明里的用户标识（如「用户名_T9PBW7」）。 */
  userLabel: string;
  checkinEnabled: boolean;
  /** access token 过期时间（秒级时间戳，null = 未知）。 */
  tokenExpiresAt: number | null;
  createdAt: number;
  updatedAt: number;
  lastGithubSyncAt: number | null;
  lastGithubSyncState: 'pending' | 'synced' | 'failed' | string;
  lastGithubSyncError: string | null;
}

export interface ZhipuAccountUpdate {
  displayName?: string;
  checkinEnabled?: boolean;
}

export interface ZhipuAccountStatus {
  /** 当前总积分（user/info 的 left_score，客户端显示口径）。 */
  leftScore: number | null;
  updatedAt: number;
  scoreError: string | null;
}

export interface ZhipuImportNotice {
  /** 网络原因未能完成在线验证（账号已保存，可稍后刷新积分确认）。 */
  verificationSkipped: boolean;
  message: string;
}
