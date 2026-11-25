import type { ReactNode } from 'react';
import { Bot, ChevronDown, CircleAlert, CircleCheck, LoaderCircle } from 'lucide-react';
import { compactUid, formatTokenExpiryDate, tokenDaysLabel, tokenDaysLeft } from '../utils/accountCardPresentation';

interface AccountAction {
  onClick?: () => void;
  disabled?: boolean;
  label?: string;
  title?: string;
}

function AccountButton({ action, label, className = '' }: {
  action: AccountAction; label: string; className?: string;
}) {
  return <button type="button" className={`wc-btn ${className}`.trim()}
    disabled={!action.onClick || action.disabled} onClick={action.onClick} title={action.title}
    aria-description={action.title} data-unavailable={action.onClick ? undefined : ''}>
    {action.label ?? label}
  </button>;
}

export function AccountManagementLayout({ app, icon, actions, children }: {
  app: string; icon?: string; children: ReactNode;
  actions: {
    importAccount: AccountAction;
    sync: AccountAction;
    refresh: AccountAction;
    checkin: AccountAction;
    settings: AccountAction;
    detect: AccountAction;
  };
}) {
  return <div className="wc-page workbuddy-page">
    <header className="wc-header">
      <h1 className="wc-title">{icon
        ? <img className="wc-title-icon" src={icon} alt="" />
        : <Bot className="wc-title-icon" aria-hidden="true" />}{app} 账号管理</h1>
      <div className="wc-header-actions" role="group" aria-label="账号管理操作">
        <AccountButton action={actions.importAccount} label="导入当前账号" className="wc-btn-primary" />
        <AccountButton action={actions.sync} label="同步 GitHub" />
        <AccountButton action={actions.refresh} label="刷新全部积分" />
        <AccountButton action={actions.checkin} label="本地一键签到" />
        <AccountButton action={actions.settings} label="路径设置" />
        <AccountButton action={actions.detect} label="重新检测" />
      </div>
    </header>
    {children}
  </div>;
}

export function AccountList({ count, loading, emptyHint, children }: {
  count: number; loading: boolean; emptyHint: string; children: ReactNode;
}) {
  return <section aria-label="已保存账号">
    <h2 className="wc-section-title">已保存账号（{count}{loading ? ' · 加载中…' : ''}）</h2>
    <div className="wc-slots wb-slots">
      {children}
      {!count && !loading ? <div className="wc-slot wc-slot--empty">
        <span className="wc-slot-empty-icon" aria-hidden="true">＋</span>
        <span className="wc-slot-empty-hint">{emptyHint}</span>
      </div> : null}
    </div>
  </section>;
}

export function AccountCardFrame({
  title, active = false, contact, uid, credits, creditsTitle, tokenExpiresAt,
  todayReward, streakDays, alert, details, recovery, cloudCheckin, children,
}: {
  title: string; active?: boolean; contact?: string | null; uid?: string | null;
  credits: ReactNode; creditsTitle?: string; tokenExpiresAt?: number | null;
  todayReward?: number | null; streakDays?: number | null;
  alert?: ReactNode; details?: ReactNode; recovery: AccountAction; cloudCheckin: AccountAction;
  children: ReactNode;
}) {
  const { tone, days } = tokenDaysLeft(tokenExpiresAt ?? null);
  const expiry = tokenExpiresAt === undefined ? '未提供' : formatTokenExpiryDate(tokenExpiresAt);
  const reward = todayReward === undefined ? '未提供' : todayReward?.toLocaleString('zh-CN', { maximumFractionDigits: 2 }) ?? '暂无数据';
  const streak = streakDays === undefined ? '未提供' : streakDays === null ? '暂无数据' : `${streakDays} 天`;
  return <article className={`wc-slot account-card account-card--compact wb-slot unified-account-card${active ? ' wc-slot--active wb-slot--active' : ''}`}>
    <div className="wc-slot-head account-card__head">
      <div className="wb-account-title"><div className="wc-slot-title" title={title}>{title}</div></div>
      <span className={`uac-state${active ? ' uac-state--active' : ''}`}>{active ? '当前账号' : '已保存'}</span>
    </div>
    <div className="wc-slot-sub account-card__identity" title={[contact, uid && `UID ${uid}`].filter(Boolean).join(' · ')}>
      {contact || '未提供联系方式'} · UID {uid ? compactUid(uid) : '未提供'}
    </div>
    <dl className="account-card__metrics uac-metrics">
      <div className="uac-metric" title={creditsTitle}>
        <dt>可用积分</dt><dd>{credits}</dd>
      </div>
      <div className="uac-metric" title={days === 0 ? '凭证已过期，请恢复认证或重新导入账号' : undefined}>
        <dt>凭证剩余</dt>
        <dd><span className={`wc-token-strong wc-token-strong--${tone}`}>{tokenExpiresAt === undefined ? expiry : tokenDaysLabel(days)}</span></dd>
      </div>
    </dl>
    <dl className="uac-quick-status">
      <div><dt>今日奖励</dt><dd>{reward}</dd></div>
      <div><dt>连续签到</dt><dd>{streak}</dd></div>
    </dl>
    {alert ? <div className="uac-alert">{alert}</div> : null}
    {children}
    <details className="uac-details">
      <summary>账号详情</summary>
      <div className="uac-details-body">
        <div className="wb-status-line"><span>令牌到期 <strong>{expiry}</strong></span></div>
        {details}
        <div className="wc-slot-actions uac-extra-actions">
          <AccountButton action={recovery} label="恢复认证" />
          <AccountButton action={cloudCheckin} label="云端签到" />
        </div>
      </div>
    </details>
  </article>;
}

export function AccountCardActions({
  switchAccount, checkin, refresh, rename, remove, cancelSwitch, cancelRemove,
}: {
  switchAccount: AccountAction; checkin: AccountAction; refresh: AccountAction; rename: AccountAction;
  remove: AccountAction; cancelSwitch?: AccountAction; cancelRemove?: AccountAction;
}) {
  return <div className="wc-slot-actions account-card__actions uac-actions">
    <div className="uac-primary">
      {cancelSwitch && <span className="wb-switch-confirm-hint">凭证需要核对，仍要尝试切换？</span>}
      <AccountButton action={switchAccount} label={cancelSwitch ? '仍要切换' : '切换并打开'}
        className={cancelSwitch ? 'wc-btn-danger' : 'wc-btn-primary'} />
      {cancelSwitch && <AccountButton action={cancelSwitch} label="取消" />}
      <AccountButton action={checkin} label="客户端签到" className="uac-checkin" />
    </div>
    <div className="uac-management">
      <AccountButton action={refresh} label="刷新积分" />
      <AccountButton action={rename} label="备注" />
      <AccountButton action={remove} label={cancelRemove ? '确认删除' : '删除'} className={cancelRemove ? 'wc-btn-danger' : ''} />
      {cancelRemove && <AccountButton action={cancelRemove} label="取消" />}
    </div>
  </div>;
}

export type AccountCheckinResult = {
  accountId: string;
  displayName: string;
  outcome: { Ok: 'signed' | 'alreadySigned' } | { Err: string };
};

export function LocalCheckinFeedback({ running, results, error }: {
  running: boolean; results: AccountCheckinResult[] | null; error?: string;
}) {
  if (!running && !results && !error) return null;

  const signed = results?.filter(({ outcome }) => 'Ok' in outcome && outcome.Ok === 'signed').length ?? 0;
  const failed = results?.filter(({ outcome }) => 'Err' in outcome).length ?? 0;
  const alreadySigned = (results?.length ?? 0) - signed - failed;
  const ResultIcon = error || failed ? CircleAlert : CircleCheck;

  return <div className="wb-checkin-feedback" aria-live="polite">
    {running || results?.length === 0 ? <div className="wb-checkin-result wb-checkin-row" role="status">
      {running && <LoaderCircle size={15} className="wb-progress-spinner" aria-hidden="true" />}
      {running ? '正在为已保存的账号签到…' : '没有可签到的已保存账号'}
    </div> : <details className="wb-checkin-result">
      <summary className="wb-checkin-row">
        <ResultIcon className={error || failed ? 'wb-checkin-failed' : 'wb-checkin-signed'} size={15} aria-hidden="true" />
        <strong>{error ? '签到失败' : '签到结果'}</strong>
        <span className="wb-checkin-counts">
          {error ? '查看原因' : <>
            {signed > 0 && <span className="wb-checkin-signed">成功 {signed}</span>}
            {alreadySigned > 0 && <span>已签到 {alreadySigned}</span>}
            {failed > 0 && <span className="wb-checkin-failed">失败 {failed}</span>}
          </>}
        </span>
        <ChevronDown size={14} className="wb-checkin-chevron" aria-hidden="true" />
      </summary>
      {error ? <p className="wb-checkin-error" role="alert">{error}</p> : (
        <ul className="wb-checkin-accounts" aria-label="逐账号签到结果" tabIndex={0}>
          {results?.map(({ accountId, displayName, outcome }) => <li key={accountId}>
            <span>{displayName}</span>
            {'Err' in outcome ? <>
              <strong className="wb-checkin-failed">失败</strong>
              <small>{outcome.Err}</small>
            </> : outcome.Ok === 'signed' ? <strong className="wb-checkin-signed">成功</strong> : <span>今日已签到</span>}
          </li>)}
        </ul>
      )}
    </details>}
  </div>;
}

export function AccountStatusBanner({ tone, children, detail }: {
  tone?: string; children: ReactNode; detail?: ReactNode;
}) {
  return <section className={`wc-status${tone ? ` ${tone}` : ''}`} role={tone === 'wc-status--error' ? 'alert' : 'status'}>
    <span aria-hidden="true" className={`wc-status-dot${tone === 'wc-status--ok' ? ' wc-status-dot--ok' : tone === 'wc-status--error' ? ' wc-status-dot--error' : ''}`} />
    <div><div>{children}</div>{detail ? <div className="wc-status-detail">{detail}</div> : null}</div>
  </section>;
}
