import assert from 'node:assert/strict';
import test from 'node:test';

import { localCheckinOutcomeLabel } from './checkinPresentation.ts';
import type { LocalCheckinResult } from '../types/checkin.ts';

function failedResult(stage: LocalCheckinResult['stage']): LocalCheckinResult {
  return {
    accountId: 'account-1',
    ok: false,
    message: '请求失败',
    alreadyCheckedIn: false,
    outcome: 'failed',
    stage,
    businessCode: null,
    httpStatus: 503,
    claimAttempted: stage === 'claim',
    tokenExpiryState: 'valid',
    devicePresent: true,
  };
}

test('失败结果按实际阶段显示标题', () => {
  assert.equal(localCheckinOutcomeLabel(failedResult('status')), '状态查询失败');
  assert.equal(localCheckinOutcomeLabel(failedResult('claim')), '领取失败');
  assert.equal(localCheckinOutcomeLabel(failedResult('usage')), '余额查询失败');
});

test('已签到结果区分是否实际发起领取', () => {
  const result = failedResult('status');
  result.ok = true;
  result.outcome = 'already_checked_in';
  result.claimAttempted = false;
  assert.equal(localCheckinOutcomeLabel(result), '查询到今日已签到，本次未发起领取');

  result.stage = 'claim';
  result.claimAttempted = true;
  assert.equal(localCheckinOutcomeLabel(result), '领取接口确认今日已签到');
});
