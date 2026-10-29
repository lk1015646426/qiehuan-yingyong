import type { WorkBuddyAccountStatus } from '../types/workbuddy';

export function aggregateWorkBuddyCredits(
  accountIds: readonly string[],
  statuses: Record<string, WorkBuddyAccountStatus>,
) {
  const ids = [...new Set(accountIds)];
  const known = ids.map(id => statuses[id]).filter(status => status
    && !status.creditsError && typeof status.credits === 'number'
    && Number.isFinite(status.credits) && status.credits >= 0);
  return {
    credits: known.length ? Math.round(known.reduce((sum, status) => sum + status.credits!, 0) * 100) / 100 : null,
    knownCount: known.length,
    accountCount: ids.length,
    complete: ids.length > 0 && known.length === ids.length,
    oldestUpdatedAt: known.length ? Math.min(...known.map(status => status.updatedAt)) : null,
  };
}
