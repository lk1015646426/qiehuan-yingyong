import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { shouldLoadWorkCnAccounts } from './workCnLifecycle.ts';

test('TRAE 账号只在首次进入且没有请求进行时加载', () => {
  assert.equal(shouldLoadWorkCnAccounts(false, false), true);
  assert.equal(shouldLoadWorkCnAccounts(true, false), false);
  assert.equal(shouldLoadWorkCnAccounts(false, true), false);
});

test('页面挂载使用幂等加载，避免切页触发全账号积分请求风暴', () => {
  const switcher = readFileSync(new URL('../pages/WorkCnSwitcherPage.tsx', import.meta.url), 'utf8');
  const checkin = readFileSync(new URL('../pages/CheckinPanelPage.tsx', import.meta.url), 'utf8');
  assert.match(switcher, /ensureAccountsLoaded/);
  assert.match(checkin, /ensureAccountsLoaded/);
});
