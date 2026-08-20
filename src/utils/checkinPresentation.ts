import type { LocalCheckinResult } from '../types/checkin';

export function localCheckinOutcomeLabel(result: LocalCheckinResult): string {
  if (result.outcome === 'already_checked_in') {
    return result.claimAttempted
      ? '领取接口确认今日已签到'
      : '查询到今日已签到，本次未发起领取';
  }
  if (result.outcome === 'claimed') {
    return '本次领取成功';
  }
  if (result.stage === 'status') {
    return '状态查询失败';
  }
  if (result.stage === 'usage') {
    return '余额查询失败';
  }
  return '领取失败';
}
