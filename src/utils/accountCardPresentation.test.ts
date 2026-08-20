import assert from 'node:assert/strict';
import test from 'node:test';

import { compactUid, githubSyncPresentation } from './accountCardPresentation.ts';

test('UID 中间省略但短 UID 保持完整', () => {
  assert.equal(compactUid('541b69abcdef84a5'), '541b69…84a5');
  assert.equal(compactUid('uid-1'), 'uid-1');
  assert.equal(compactUid(null), '未知');
});

test('GitHub 只有真实失败才使用失败文案和详情', () => {
  assert.deepEqual(githubSyncPresentation('pending', '不应显示'), {
    label: 'GitHub 待同步',
    tone: 'pending',
    detail: null,
  });
  assert.deepEqual(githubSyncPresentation('syncing', null), {
    label: 'GitHub 同步中',
    tone: 'syncing',
    detail: null,
  });
  assert.deepEqual(githubSyncPresentation('synced', '旧错误'), {
    label: 'GitHub 已同步',
    tone: 'synced',
    detail: null,
  });
  assert.deepEqual(githubSyncPresentation('failed', '网络不可用'), {
    label: 'GitHub 失败',
    tone: 'failed',
    detail: '网络不可用',
  });
  assert.equal(githubSyncPresentation('legacy-value', null).tone, 'pending');
});
