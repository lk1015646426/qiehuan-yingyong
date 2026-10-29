import type { WorkBuddyAccountStatus } from '../../types/workbuddy';
import { aggregateWorkBuddyCredits } from '../../utils/workBuddyCredits';

interface Props {
  accountIds: readonly string[];
  statuses: Record<string, WorkBuddyAccountStatus>;
  loading: boolean;
}

export function WorkBuddyCreditsSummary({ accountIds, statuses, loading }: Props) {
  const total = aggregateWorkBuddyCredits(accountIds, statuses);
  return <section className="wc-slot wb-credits-total" aria-label="全部账号积分汇总">
    <div><span className="wb-pool-muted">{total.complete ? '全部账号可用积分合计' : '已查询账号可用积分合计'}</span>
      <strong className="wb-credits-total-value">{total.credits == null ? '暂未查询到' : total.credits.toLocaleString('zh-CN', { maximumFractionDigits: 2 })}{total.credits == null ? '' : ' 积分'}</strong></div>
    <div className="wb-pool-muted">已查询 {total.knownCount}/{total.accountCount} 个账号{loading ? ' · 正在更新' : ''}
      {total.oldestUpdatedAt != null ? ` · 最早查询 ${new Date(total.oldestUpdatedAt * 1000).toLocaleTimeString('zh-CN', { hour12: false })}` : ''}
      <div>{total.complete ? '复用各账号的只读余额数据，每分钟随账号卡片更新。' : total.accountCount ? '部分余额尚未查询到，当前数值不是完整总额。' : '导入账号后显示积分汇总。'}</div>
    </div>
  </section>;
}
