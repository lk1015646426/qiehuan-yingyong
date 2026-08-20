import assert from 'node:assert/strict';
import test from 'node:test';

import { parseWorkBuddyCommandError } from './workBuddyService.ts';

test('解析 WorkBuddy 结构化错误并保留错误码', () => {
  const error = parseWorkBuddyCommandError(JSON.stringify({
    code: 'CLIENT_CLOSE_FAILED',
    message: 'WorkBuddy 旧实例未能关闭',
    detail: '权限不足',
  }));
  assert.equal(error.message, 'WorkBuddy 旧实例未能关闭（权限不足）');
  assert.equal(error.code, 'CLIENT_CLOSE_FAILED');
});

test('非结构化 WorkBuddy 错误仍可读', () => {
  assert.equal(parseWorkBuddyCommandError('普通错误').message, '普通错误');
});
