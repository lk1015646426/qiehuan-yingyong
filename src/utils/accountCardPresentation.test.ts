import assert from 'node:assert/strict';
import test from 'node:test';

import { compactUid, credentialInvalidated, githubSyncPresentation, tokenDaysLabel, tokenDaysLeft } from './accountCardPresentation.ts';

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

test('后端凭证失效文案被识别为凭证失效', () => {
  // workbuddy_status.rs 401 文案
  assert.equal(credentialInvalidated(['积分查询认证已失效，请重新登录 WorkBuddy 后更新账号']), true);
  // workbuddy_account.rs 刷新失败文案（status 层包装后）
  assert.equal(credentialInvalidated(['积分查询认证刷新失败：refresh token 已失效，请重新登录 WorkBuddy']), true);
  assert.equal(credentialInvalidated(['签到状态查询认证刷新失败：WorkBuddy 快照缺少 refresh token，请重新登录 WorkBuddy']), true);
  assert.equal(credentialInvalidated(['refresh token 无效']), true);
  // 错误出现在任一字段即可识别
  assert.equal(credentialInvalidated([null, '积分查询认证已失效，请重新登录 WorkBuddy 后更新账号']), true);
});

test('网络类刷新失败不误判为凭证失效', () => {
  assert.equal(credentialInvalidated(['积分查询认证刷新失败：WorkBuddy 刷新请求失败: 连接超时']), false);
  assert.equal(credentialInvalidated(['签到状态查询返回 HTTP 502']), false);
  assert.equal(credentialInvalidated([null, undefined, '']), false);
  assert.equal(credentialInvalidated([]), false);
});

test('Token 剩余天数按阈值分级（三页共用预警逻辑）', () => {
  const DAY = 86_400;
  const now = Math.floor(Date.now() / 1000);
  // 无到期时间 → unknown
  assert.deepEqual(tokenDaysLeft(null), { tone: 'unknown', days: null });
  // 已过期 → danger / 0
  assert.deepEqual(tokenDaysLeft(now - 10), { tone: 'danger', days: 0 });
  // 剩 1 天内 → danger
  const danger = tokenDaysLeft(now + DAY / 2);
  assert.equal(danger.tone, 'danger');
  assert.equal(danger.days, 1);
  // 剩 2~5 天 → warn
  const warn = tokenDaysLeft(now + 3 * DAY);
  assert.equal(warn.tone, 'warn');
  assert.equal(warn.days, 3);
  // 剩 6 天以上 → ok
  const ok = tokenDaysLeft(now + 30 * DAY);
  assert.equal(ok.tone, 'ok');
  assert.equal(ok.days, 30);
});

test('Token 剩余天数文案', () => {
  assert.equal(tokenDaysLabel(null), '未知');
  assert.equal(tokenDaysLabel(0), '已过期');
  assert.equal(tokenDaysLabel(3), '3 天');
});
