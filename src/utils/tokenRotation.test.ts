import assert from 'node:assert/strict';
import test from 'node:test';

import {
  didTokenRotate,
  hasCompleteOfficialDeviceSnapshot,
  isVerifiedRefreshTarget,
  shouldSyncWithoutWaiting,
  shouldWaitForTokenRotation,
} from './tokenRotation.ts';

test('iat 和 exp 都未变化时不判定为 token 轮换', () => {
  const metadata = { tokenIssuedAt: 100, tokenExpiresAt: 200 };
  assert.equal(didTokenRotate(metadata, metadata), false);
});

test('iat 或 exp 的已知值变化时判定为 token 轮换', () => {
  const before = { tokenIssuedAt: 100, tokenExpiresAt: 200 };
  assert.equal(
    didTokenRotate(before, { tokenIssuedAt: 101, tokenExpiresAt: 200 }),
    true,
  );
  assert.equal(
    didTokenRotate(before, { tokenIssuedAt: 100, tokenExpiresAt: 201 }),
    true,
  );
});

test('元数据缺失或仅从缺失变为可用时不产生误报', () => {
  assert.equal(
    didTokenRotate(
      { tokenIssuedAt: null, tokenExpiresAt: null },
      { tokenIssuedAt: 100, tokenExpiresAt: 200 },
    ),
    false,
  );
  assert.equal(
    didTokenRotate(
      { tokenIssuedAt: 100, tokenExpiresAt: null },
      { tokenIssuedAt: 100, tokenExpiresAt: 200 },
    ),
    false,
  );
});

test('仅在 token 已过期、即将过期或有效期未知时等待官方客户端轮换', () => {
  const now = 1_000_000;

  assert.equal(
    shouldWaitForTokenRotation({ tokenIssuedAt: 1, tokenExpiresAt: now + 86_401 }, now),
    false,
  );
  assert.equal(
    shouldWaitForTokenRotation({ tokenIssuedAt: 1, tokenExpiresAt: now + 86_400 }, now),
    true,
  );
  assert.equal(
    shouldWaitForTokenRotation({ tokenIssuedAt: 1, tokenExpiresAt: now - 1 }, now),
    true,
  );
  assert.equal(
    shouldWaitForTokenRotation({ tokenIssuedAt: 1, tokenExpiresAt: null }, now),
    true,
  );
});

test('同步现有有效 token 前必须具备完整官方设备快照', () => {
  const complete = {
    hasAccessToken: true,
    hasUserId: true,
    hasAuthDeviceId: true,
    hasDevicePrivateKey: true,
    hasDevicePublicKey: true,
    hasCheckinDeviceId: true,
  };

  assert.equal(hasCompleteOfficialDeviceSnapshot(complete), true);
  assert.equal(
    hasCompleteOfficialDeviceSnapshot({ ...complete, hasDevicePublicKey: false }),
    false,
  );
  assert.equal(
    hasCompleteOfficialDeviceSnapshot({ ...complete, hasAuthDeviceId: false }),
    false,
  );
  assert.equal(
    hasCompleteOfficialDeviceSnapshot({ ...complete, hasCheckinDeviceId: false }),
    false,
  );
});

test('切换期间已轮换 token 时不再进入等待阶段', () => {
  assert.equal(shouldSyncWithoutWaiting(false, false), true);
  assert.equal(shouldSyncWithoutWaiting(true, true), true);
  assert.equal(shouldSyncWithoutWaiting(true, false), false);
});

test('后端确认目标账号后不比较前端脱敏 UID', () => {
  assert.equal(
    isVerifiedRefreshTarget(
      { accountId: 'account-1780293', verified: true },
      'account-1780293',
    ),
    true,
  );
  assert.equal(
    isVerifiedRefreshTarget(
      { accountId: 'account-1780293', verified: false },
      'account-1780293',
    ),
    false,
  );
  assert.equal(
    isVerifiedRefreshTarget(
      { accountId: 'account-other', verified: true },
      'account-1780293',
    ),
    false,
  );
});
