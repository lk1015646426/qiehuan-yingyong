import assert from 'node:assert/strict';
import test from 'node:test';

import { buildGhSetupConfig, ghSetupConfigReady, validateGhSetupConfig } from './ghSetupConfig.ts';
import type { WorkCnGitHubConfig } from '../types/workCn.ts';

const existingConfig: WorkCnGitHubConfig = {
  enabled: true,
  repository: 'o/r',
  slots: [
    { slot: 1, accountId: 'a', tokenSecret: '', deviceSecret: '', checkinEnabled: false },
    { slot: 2, accountId: 'b', tokenSecret: 'CUSTOM_TOKEN', deviceSecret: 'CUSTOM_DEVICE_ID', checkinEnabled: true },
  ],
  workflowFile: 'daily-checkin.yml',
};

test('红线：repo-only 保存原样透传已有槽位，绝不清空', () => {
  const config = buildGhSetupConfig({
    mode: 'repo-only',
    enabled: true,
    repository: ' new-owner/new-repo ',
    workflowFile: 'daily-checkin.yml',
    existingConfig,
  });
  assert.equal(config.repository, 'new-owner/new-repo');
  assert.deepEqual(config.slots, existingConfig.slots, 'repo-only 保存不得改动 TRAE 已绑定的槽位');
});

test('full 模式重建槽位但保留自动签到开关', () => {
  const slotNumbers = new Map([
    ['a', 1],
    ['c', 2],
  ]);
  const config = buildGhSetupConfig({
    mode: 'full',
    enabled: true,
    repository: 'o/r',
    workflowFile: 'daily-checkin.yml',
    existingConfig,
    slotNumbers,
  });
  assert.deepEqual(
    config.slots.map((s) => [s.slot, s.accountId, s.checkinEnabled]),
    [
      [1, 'a', false],
      [2, 'c', true],
    ],
    '重建时按勾选列表生成，开关从旧配置继承（未保存过的默认开启）',
  );
});

test('校验：repo-only 允许零槽位，full 必须至少绑定一个', () => {
  const repoOnly = buildGhSetupConfig({
    mode: 'repo-only',
    enabled: true,
    repository: 'o/r',
    workflowFile: 'daily-checkin.yml',
    existingConfig: { ...existingConfig, slots: [] },
  });
  assert.equal(validateGhSetupConfig('repo-only', repoOnly), null);

  const fullEmpty = buildGhSetupConfig({
    mode: 'full',
    enabled: true,
    repository: 'o/r',
    workflowFile: 'daily-checkin.yml',
    existingConfig,
    slotNumbers: new Map(),
  });
  assert.equal(validateGhSetupConfig('full', fullEmpty), '启用同步时至少要绑定一个账号到槽位');

  const badRepo = { ...repoOnly, repository: 'not-a-repo' };
  assert.equal(validateGhSetupConfig('repo-only', badRepo), '仓库需为 owner/repo 形式');
});

test('就绪判定：repo-only 不看槽位数，full 必须有槽位', () => {
  const emptySlots = { ...existingConfig, slots: [] };
  assert.equal(ghSetupConfigReady('repo-only', emptySlots), true);
  assert.equal(ghSetupConfigReady('full', emptySlots), false);
  assert.equal(ghSetupConfigReady('repo-only', { ...emptySlots, enabled: false }), false);
  assert.equal(ghSetupConfigReady('full', { ...existingConfig, repository: '' }), false);
});
