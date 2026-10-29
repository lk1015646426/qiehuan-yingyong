import assert from 'node:assert/strict';
import test from 'node:test';
import { aggregateWorkBuddyCredits } from './workBuddyCredits.ts';
import type { WorkBuddyAccountStatus } from '../types/workbuddy.ts';

const status = (credits: number | null, updatedAt = 100): WorkBuddyAccountStatus => ({
  credits, updatedAt, creditsError: null, activityError: null, todayReward: null, streakDays: null,
});

test('全部账号按当前可用余额合计，时间采用最早一次查询', () => {
  assert.deepEqual(aggregateWorkBuddyCredits(['a', 'b'], { a: status(100.12, 20), b: status(200.38, 30) }), {
    credits: 300.5, knownCount: 2, accountCount: 2, complete: true, oldestUpdatedAt: 20,
  });
});
test('真实零余额参与合计，未查询账号不能当作零', () => {
  const result = aggregateWorkBuddyCredits(['a', 'b'], { a: status(0), b: status(null) });
  assert.equal(result.credits, 0);
  assert.equal(result.knownCount, 1);
  assert.equal(result.complete, false);
});
test('没有可用数据时不伪造零积分', () => {
  assert.equal(aggregateWorkBuddyCredits(['a'], {}).credits, null);
  assert.equal(aggregateWorkBuddyCredits([], {}).complete, false);
});
test('已删除账号的缓存不参与统计，相同账号不会重复累计', () => {
  const result = aggregateWorkBuddyCredits(['a', 'a'], { a: status(9.99), removed: status(9000) });
  assert.equal(result.credits, 9.99);
  assert.equal(result.accountCount, 1);
});
test('失败和非有限余额不进入完整总额', () => {
  const result = aggregateWorkBuddyCredits(['a', 'b', 'c'], {
    a: { ...status(20), creditsError: '查询失败' }, b: status(Infinity), c: status(-1),
  });
  assert.equal(result.credits, null);
  assert.equal(result.knownCount, 0);
});
