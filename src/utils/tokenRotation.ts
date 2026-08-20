export interface TokenMetadata {
  tokenIssuedAt: number | null;
  tokenExpiresAt: number | null;
}

export interface OfficialDeviceSnapshotStatus {
  hasAccessToken: boolean;
  hasUserId: boolean;
  hasAuthDeviceId: boolean;
  hasDevicePrivateKey: boolean;
  hasDevicePublicKey: boolean;
  hasCheckinDeviceId: boolean;
}

const TOKEN_ROTATION_WINDOW_SECONDS = 24 * 60 * 60;

function knownTimestampChanged(before: number | null, after: number | null): boolean {
  return before !== null && after !== null && before !== after;
}

export function didTokenRotate(before: TokenMetadata, after: TokenMetadata): boolean {
  return (
    knownTimestampChanged(before.tokenIssuedAt, after.tokenIssuedAt) ||
    knownTimestampChanged(before.tokenExpiresAt, after.tokenExpiresAt)
  );
}

/**
 * 有效期未知、已过期或不足一天时，必须等官方客户端轮换 Token。
 * 仍有充足有效期的 Token 可以用于修复并同步设备身份，无须强制重登。
 */
export function shouldWaitForTokenRotation(
  metadata: TokenMetadata,
  nowSeconds: number,
): boolean {
  return (
    metadata.tokenExpiresAt === null ||
    metadata.tokenExpiresAt <= nowSeconds + TOKEN_ROTATION_WINDOW_SECONDS
  );
}

/**
 * 切换命令已确认发生凭证轮换时，直接同步，避免错过早于事件订阅发生的更新。
 */
export function shouldSyncWithoutWaiting(
  needsTokenRotation: boolean,
  tokenChangedDuringSwitch: boolean,
): boolean {
  return !needsTokenRotation || tokenChangedDuringSwitch;
}

/**
 * UID 在前端账号视图中必须脱敏，不能再与后端验证阶段使用的真实 UID 比较。
 * 后端已在切换命令内确认官方客户端身份；前端只核对该结果仍属于本次目标账号。
 */
export function isVerifiedRefreshTarget(
  result: { accountId: string; verified: boolean },
  targetAccountId: string,
): boolean {
  return result.verified && result.accountId === targetAccountId;
}

/**
 * 前端只消费脱敏布尔值；实际数字设备 ID 仍由 Rust 同步层验证。
 */
export function hasCompleteOfficialDeviceSnapshot(
  snapshot: OfficialDeviceSnapshotStatus,
): boolean {
  return (
    snapshot.hasAccessToken &&
    snapshot.hasUserId &&
    snapshot.hasAuthDeviceId &&
    snapshot.hasDevicePrivateKey &&
    snapshot.hasDevicePublicKey &&
    snapshot.hasCheckinDeviceId
  );
}
