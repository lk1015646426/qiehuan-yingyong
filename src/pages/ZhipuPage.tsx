import { useEffect, useState } from 'react';
import zhipuIcon from '../assets/icons/zhipu.svg';
import { ZhipuAddAccountDialog } from '../components/zhipu/ZhipuAddAccountDialog';
import { useZhipuStore } from '../stores/useZhipuStore';
import type { ZhipuAccountView } from '../types/zhipu';
import { githubSyncPresentation } from '../utils/accountCardPresentation';

function timeText(timestamp: number | null): string {
  if (!timestamp) return '未知';
  return new Date(timestamp * 1000).toLocaleString('zh-CN', { hour12: false });
}

function scoreText(value: number | null): string {
  return value != null ? value.toLocaleString('zh-CN', { maximumFractionDigits: 0 }) : '暂无数据';
}

function AccountCard({ account }: { account: ZhipuAccountView }) {
  const updatingId = useZhipuStore((state) => state.updatingId);
  const deletingId = useZhipuStore((state) => state.deletingId);
  const checkingInId = useZhipuStore((state) => state.checkingInId);
  const updateAccount = useZhipuStore((state) => state.updateAccount);
  const deleteAccount = useZhipuStore((state) => state.deleteAccount);
  const triggerCheckin = useZhipuStore((state) => state.triggerCheckin);
  const status = useZhipuStore((state) => state.statusById[account.id]);
  const statusLoading = useZhipuStore((state) => Boolean(state.statusLoadingById[account.id]));
  const refreshAccountStatus = useZhipuStore((state) => state.refreshAccountStatus);
  const [deleting, setDeleting] = useState(false);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(account.displayName);
  const busy = updatingId !== null || deletingId !== null || checkingInId !== null;
  const githubSync = githubSyncPresentation(account.lastGithubSyncState, account.lastGithubSyncError);
  const saveName = async () => {
    const value = name.trim();
    if (!value) return;
    await updateAccount(account.id, { displayName: value });
    setEditing(false);
  };
  const statusSummary = `${account.checkinEnabled ? '自动签到已开启' : '自动签到未开启'} · ${githubSync.label}`;
  const statusDetail = githubSync.detail ?? '';
  return <article className="wc-slot account-card account-card--compact wb-slot">
    <div className="wc-slot-head account-card__head">
      <div className="wb-account-title">
        <div className="wc-slot-title" title={account.displayName}>{account.displayName}</div>
      </div>
      <span className={`wb-sync wb-sync--${githubSync.tone}`} title={githubSync.detail ?? githubSync.label}>{githubSync.label}</span>
    </div>
    <div className="wc-slot-sub account-card__identity" title={account.userLabel}>{account.userLabel || `ID ${account.id.slice(3, 9)}`}</div>
    <div className="account-card__metrics">
      <div className="wc-slot-sub">令牌到期 {timeText(account.tokenExpiresAt)}</div>
    </div>
    <div className="wb-status-block">
      <div className="wb-status-line" aria-label="智谱清言积分">
        <span>当前积分 <strong>{statusLoading && !status ? '查询中…' : scoreText(status?.currentScore ?? null)}</strong></span>
        <span>活动状态 <strong>{status?.activityStatus != null ? (status.activityStatus > 0 ? '进行中' : '无活动') : '暂无数据'}</strong></span>
      </div>
      {status?.scoreError ? <div className="wb-status-error">{status.scoreError}</div> : null}
    </div>
    <div className="account-card__status account-card__status--toggle" title={statusDetail || statusSummary}>
      <label className="wb-checkin-toggle">
        <input type="checkbox" checked={account.checkinEnabled} disabled={busy} onChange={(event) => void updateAccount(account.id, { checkinEnabled: event.target.checked })} />
        <span>{statusSummary}</span>
      </label>
    </div>
    {editing ? <div className="wb-edit-name">
      <input className="wc-input" value={name} maxLength={80} onChange={(event) => setName(event.target.value)} />
      <button type="button" className="wc-btn" onClick={() => void saveName()}>保存</button>
      <button type="button" className="wc-btn" onClick={() => { setName(account.displayName); setEditing(false); }}>取消</button>
    </div> : null}
    <div className="wc-slot-actions account-card__actions">
      <button type="button" className="wc-btn" disabled={busy || statusLoading} onClick={() => void refreshAccountStatus(account.id)}>{statusLoading ? '刷新中…' : '刷新积分'}</button>
      <button type="button" className="wc-btn" disabled={busy || !account.checkinEnabled} onClick={() => void triggerCheckin(account.id)}>{checkingInId === account.id ? '触发中…' : '立即签到'}</button>
      <button type="button" className="wc-btn" disabled={busy} onClick={() => setEditing((value) => !value)}>备注</button>
      {deleting ? <>
        <button type="button" className="wc-btn wc-btn-danger" disabled={busy} onClick={() => void deleteAccount(account.id)}>确认删除</button>
        <button type="button" className="wc-btn" disabled={busy} onClick={() => setDeleting(false)}>取消</button>
      </> : <button type="button" className="wc-btn" disabled={busy} onClick={() => setDeleting(true)}>删除</button>}
    </div>
  </article>;
}

export function ZhipuPage() {
  const [dialogOpen, setDialogOpen] = useState(false);
  const accounts = useZhipuStore((state) => state.accounts);
  const loading = useZhipuStore((state) => state.loading);
  const syncing = useZhipuStore((state) => state.syncing);
  const error = useZhipuStore((state) => state.error);
  const notice = useZhipuStore((state) => state.notice);
  const ensureAccountsLoaded = useZhipuStore((state) => state.ensureAccountsLoaded);
  const syncGitHub = useZhipuStore((state) => state.syncGitHub);
  const refreshAllStatuses = useZhipuStore((state) => state.refreshAllStatuses);
  const statusLoadingById = useZhipuStore((state) => state.statusLoadingById);
  const clearFeedback = useZhipuStore((state) => state.clearFeedback);
  useEffect(() => {
    void ensureAccountsLoaded();
  }, [ensureAccountsLoaded]);
  return <div className="wc-page zhipu-page">
    <header className="wc-header">
      <h1 className="wc-title"><img className="wc-title-icon" src={zhipuIcon} alt="智谱" />智谱清言账号管理</h1>
      <div className="wc-header-actions">
        <button type="button" className="wc-btn wc-btn-primary" onClick={() => setDialogOpen(true)}>导入当前账号</button>
        <button type="button" className="wc-btn" disabled={syncing || !accounts.length} onClick={() => void syncGitHub()}>{syncing ? '同步智谱中…' : '同步智谱'}</button>
        <button type="button" className="wc-btn" disabled={!accounts.length || Object.values(statusLoadingById).some(Boolean)} onClick={() => void refreshAllStatuses()}>刷新全部积分</button>
      </div>
    </header>
    <section className="wc-status">
      <span className="wc-status-dot" />
      <div>智谱清言（chatglm.cn）每日登录积分：本地从客户端加密导入登录态并同步到 GitHub，签到由云端 Actions 完成（与 TRAE / WorkBuddy 同一签到仓库）。云端每次签到前自动续期 access token（refresh token 约 180 天有效），导入一次即可长期免维护，到期前重新导入即可。</div>
    </section>
    {error || notice ? <section className={`wc-status ${error ? 'wc-status--error' : 'wc-status--ok'}`}><div style={{ flex: 1 }}>{error ?? notice}</div><button type="button" className="wc-btn" onClick={clearFeedback}>知道了</button></section> : null}
    <section>
      <h2 className="wc-section-title">已保存账号（{accounts.length}{loading ? ' · 加载中…' : ''}）</h2>
      <div className="wc-slots wb-slots zhipu-slots">
        {accounts.map((account) => <AccountCard key={account.id} account={account} />)}
        {!accounts.length && !loading ? <div className="wc-slot wc-slot--empty"><span className="wc-slot-empty-icon">＋</span><span className="wc-slot-empty-hint">在智谱清言客户端完成登录后，导入当前账号</span></div> : null}
      </div>
    </section>
    <ZhipuAddAccountDialog open={dialogOpen} onClose={() => setDialogOpen(false)} />
  </div>;
}
