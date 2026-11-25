import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import test from 'node:test';

import { shouldLoadWorkBuddyAccounts } from './workBuddyLifecycle.ts';

test('账号只在尚未加载且当前没有加载时请求', () => {
  assert.equal(shouldLoadWorkBuddyAccounts(false, false), true);
  assert.equal(shouldLoadWorkBuddyAccounts(true, false), false);
  assert.equal(shouldLoadWorkBuddyAccounts(false, true), false);
});

test('WorkBuddy 监测由页面显式启动，定时刷新只查询余额', () => {
  const lib = readFileSync(new URL('../../src-tauri/src/lib.rs', import.meta.url), 'utf8');
  const commands = readFileSync(new URL('../../src-tauri/src/commands/workbuddy.rs', import.meta.url), 'utf8');
  const page = readFileSync(new URL('../pages/WorkBuddyPage.tsx', import.meta.url), 'utf8');

  assert.equal(lib.includes('workbuddy_session_watcher::ensure_started();'), false);
  assert.equal(commands.includes('start_workbuddy_session_watcher'), true);
  const refreshEffect = page.slice(page.indexOf('const refreshIfStale ='), page.indexOf('}, [refreshAllStatuses, accounts]);'));
  assert.ok(refreshEffect.length > 0);
  assert.match(refreshEffect, /refreshAllStatuses\(\)/);
  assert.match(refreshEffect, /document\.visibilityState/);
  assert.doesNotMatch(refreshEffect, /triggerCheckin|switchAccount|importAccount|syncGitHub/);
});

test('WorkBuddy 用户操作不应同步阻塞 GitHub CLI', () => {
  const commands = readFileSync(new URL('../../src-tauri/src/commands/workbuddy.rs', import.meta.url), 'utf8');
  assert.match(commands, /fn queue_workbuddy_github_sync\(/);
  assert.match(commands, /queue_workbuddy_github_sync\("import"\)/);
  assert.match(commands, /queue_workbuddy_github_sync\("update"\)/);
  assert.match(commands, /queue_workbuddy_github_sync\("delete"\)/);
  assert.match(commands, /queue_workbuddy_github_sync\("switch"\)/);
});

test('WorkBuddy 真实积分、今日奖励和连续签到状态应支持只读刷新', () => {
  const lib = readFileSync(new URL('../../src-tauri/src/lib.rs', import.meta.url), 'utf8');
  const modules = readFileSync(new URL('../../src-tauri/src/modules/mod.rs', import.meta.url), 'utf8');
  const commands = readFileSync(new URL('../../src-tauri/src/commands/workbuddy.rs', import.meta.url), 'utf8');
  const models = readFileSync(new URL('../../src-tauri/src/models/workbuddy.rs', import.meta.url), 'utf8');
  const service = readFileSync(new URL('../services/workBuddyService.ts', import.meta.url), 'utf8');
  const store = readFileSync(new URL('../stores/useWorkBuddyStore.ts', import.meta.url), 'utf8');
  const page = readFileSync(new URL('../pages/WorkBuddyPage.tsx', import.meta.url), 'utf8');
  const statusModuleUrl = new URL('../../src-tauri/src/modules/workbuddy_status.rs', import.meta.url);
  const statusModule = existsSync(statusModuleUrl) ? readFileSync(statusModuleUrl, 'utf8') : '';

  assert.match(lib, /get_workbuddy_account_status/);
  assert.match(modules, /pub mod workbuddy_status/);
  assert.match(commands, /get_workbuddy_account_status/);
  assert.match(models, /WorkBuddyAccountStatus/);
  assert.match(service, /getWorkBuddyAccountStatus/);
  assert.match(store, /statusById|refreshAccountStatus|refreshAllStatuses/);
  assert.match(page, /credits=\{statusLoading[\s\S]*status\?\.credits/);
  assert.match(page, /todayReward=\{status\?\.todayReward/);
  assert.match(page, /streakDays=\{status\?\.streakDays/);
  assert.match(page, /refresh: \{ onClick: \(\) => void refreshAllStatuses\(\)/);
  assert.match(statusModule, /get-user-resource/);
  assert.match(statusModule, /checkin-activity-status/);
  assert.match(commands, /fn get_workbuddy_account_status\([^)]*\)[^{]*\{\s*workbuddy_status::query_account_status\(&account_id\)\.await\s*\}/);
  const query = statusModule.slice(statusModule.indexOf('async fn query_both('), statusModule.indexOf('fn assemble_status('));
  assert.match(query, /query_json\(client, RESOURCE_URL, token,/);
  assert.match(query, /query_json\(client, ACTIVITY_URL, token,/);
  assert.doesNotMatch(query, /daily-checkin|CHECKIN_URL|checkin_account\(/);
  const readOnly = statusModule.slice(statusModule.indexOf('pub async fn query_account_status('), statusModule.indexOf('#[cfg(test)]'));
  assert.match(readOnly, /query_both\(&client, &token\)\.await/);
  assert.doesNotMatch(readOnly, /daily-checkin|CHECKIN_URL|checkin_account\(/);
});

test('WorkBuddy 安装检测不得在 Tauri 命令线程同步扫描进程', () => {
  const commands = readFileSync(new URL('../../src-tauri/src/commands/workbuddy.rs', import.meta.url), 'utf8');
  assert.match(commands, /pub async fn get_workbuddy_installation\(/);
  assert.match(commands, /spawn_blocking\(\|\|/);
});

test('WorkBuddy 账号更新应立即乐观反馈并阻止重复提交', () => {
  const store = readFileSync(new URL('../stores/useWorkBuddyStore.ts', import.meta.url), 'utf8');
  const page = readFileSync(new URL('../pages/WorkBuddyPage.tsx', import.meta.url), 'utf8');
  assert.match(store, /updatingId: string \| null/);
  assert.match(store, /if \(get\(\)\.updatingId !== null\) return/);
  assert.match(store, /checkinEnabled: update\.checkinEnabled/);
  assert.match(page, /state\) => state\.updatingId/);
});
