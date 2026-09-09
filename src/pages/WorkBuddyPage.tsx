import { useEffect, useState } from 'react';
import workBuddyIcon from '../assets/icons/workbuddy.png';
import { CloudCheckinPanel } from '../components/checkin/CloudCheckinPanel';
import { GhSetupDialog } from '../components/work-cn/GhSetupDialog';
import { getWorkCnGitHubConfig } from '../services/workCnService';
import { WorkBuddyAddAccountDialog } from '../components/workbuddy/WorkBuddyAddAccountDialog';
import { WorkBuddySettingsDialog } from '../components/workbuddy/WorkBuddySettingsDialog';
import { useWorkBuddyStore } from '../stores/useWorkBuddyStore';
import type { WorkBuddyAccountView, WorkBuddyInstallation } from '../types/workbuddy';
import { compactUid, credentialInvalidated, formatTokenExpiryDate, githubSyncPresentation, tokenDaysLabel, tokenDaysLeft } from '../utils/accountCardPresentation';

// 切号各阶段文案（stage 与 Rust 事件 workbuddy-switch-progress 一致）。
const SWITCH_STAGE_LABELS: Record<string, string> = {
  validating: '正在校验账号快照',
  closing: '正在关闭客户端',
  refreshing: '正在刷新目标账号凭证',
  injecting: '正在注入账号凭证',
  launching: '正在启动客户端',
  syncing: '正在同步 GitHub 签到',
};

function installationText(installation: WorkBuddyInstallation | null, loading: boolean): string {
  if (loading) return '正在检测 WorkBuddy 客户端…';
  if (!installation?.installed) return '未检测到 WorkBuddy，请安装客户端后重新检测';
  return installation.running ? '已检测到 WorkBuddy，客户端正在运行' : '已检测到 WorkBuddy，可以导入已登录账号';
}

function AccountCard({ account, active }: { account: WorkBuddyAccountView; active: boolean }) {
  const switchingId = useWorkBuddyStore((state) => state.switchingId);
  const switchStage = useWorkBuddyStore((state) => state.switchStage);
  const updatingId = useWorkBuddyStore((state) => state.updatingId);
  const deletingId = useWorkBuddyStore((state) => state.deletingId);
  const checkingInId = useWorkBuddyStore((state) => state.checkingInId);
  const updateAccount = useWorkBuddyStore((state) => state.updateAccount);
  const deleteAccount = useWorkBuddyStore((state) => state.deleteAccount);
  const switchTo = useWorkBuddyStore((state) => state.switchTo);
  const triggerCheckin = useWorkBuddyStore((state) => state.triggerCheckin);
  const status = useWorkBuddyStore((state) => state.statusById[account.id]);
  const statusLoading = useWorkBuddyStore((state) => Boolean(state.statusLoadingById[account.id]));
  const refreshAccountStatus = useWorkBuddyStore((state) => state.refreshAccountStatus);
  const [deleting, setDeleting] = useState(false);
  const [editing, setEditing] = useState(false);
  const [confirmingSwitch, setConfirmingSwitch] = useState(false);
  const [name, setName] = useState(account.displayName);
  const busy = switchingId !== null || updatingId !== null || deletingId !== null || checkingInId !== null;
  const githubSync = githubSyncPresentation(account.lastGithubSyncState, account.lastGithubSyncError);
  // Token 剩余天数预警（与 TRAE / 智谱 页同一实现）：≤5 天预警、≤1 天危险。
  const { tone: tokenTone, days: tokenDays } = tokenDaysLeft(account.tokenExpiresAt);
  const saveName = async () => {
    const value = name.trim();
    if (!value) return;
    await updateAccount(account.id, { displayName: value });
    setEditing(false);
  };
  const statusSummary = `${account.checkinEnabled ? '自动签到已开启' : '自动签到未开启'} · ${githubSync.label}`;
  const statusDetail = githubSync.detail ?? '';
  const statusErrors = [status?.creditsError, status?.activityError]
    .filter((value): value is string => Boolean(value))
    .filter((value, index, values) => values.indexOf(value) === index);
  // 服务端已作废的凭证（可能在别处登录导致 token 轮换）：切换会在联网刷新一步
  // 失败并回滚，提前用醒目标记 + 二次确认引导用户先重新登录导入。
  const credentialInvalid = credentialInvalidated([status?.creditsError, status?.activityError]);
  const requestSwitch = () => {
    if (credentialInvalid && !confirmingSwitch) {
      setConfirmingSwitch(true);
      return;
    }
    setConfirmingSwitch(false);
    void switchTo(account.id);
  };
  return <article className={active ? 'wc-slot account-card account-card--compact wb-slot wb-slot--active' : 'wc-slot account-card account-card--compact wb-slot'}>
    {active ? <span className="wc-slot-active-badge">当前账号</span> : null}
    <div className="wc-slot-head account-card__head">
      <div className="wb-account-title">
        <div className="wc-slot-title" title={account.displayName}>{account.displayName}</div>
      </div>
      <span className={`wb-sync wb-sync--${githubSync.tone}`} title={githubSync.detail ?? githubSync.label}>{githubSync.label}</span>
    </div>
    <div className="wc-slot-sub account-card__identity" title={account.uid}>{account.maskedPhone ?? '未提供手机号'} · UID {compactUid(account.uid)}</div>
    <div className="account-card__metrics">
      <div className="wc-slot-sub">令牌到期 {formatTokenExpiryDate(account.tokenExpiresAt)}</div>
    </div>
    <div className="wb-status-block">
      {credentialInvalid ? <div className="wb-credential-invalid" role="alert">凭证已失效（可能在其他设备登录过），切换无法完成。请先在 WorkBuddy 客户端重新登录该账号，再导入更新。</div> : null}
      <div className="wb-status-line" aria-label="WorkBuddy 状态">
        <span>真实积分 <strong>{statusLoading && !status ? '查询中…' : status?.credits != null ? status.credits.toLocaleString('zh-CN', { maximumFractionDigits: 2 }) : '暂无数据'}</strong></span>
        <span>今日奖励 <strong>{status?.todayReward != null ? status.todayReward.toLocaleString('zh-CN', { maximumFractionDigits: 2 }) : '暂无数据'}</strong></span>
        <span>连续签到 <strong>{status?.streakDays != null ? `${status.streakDays} 天` : '暂无数据'}</strong></span>
        <span title={tokenDays != null && tokenDays <= 0 ? 'Token 已过期，云端签到将失败，请在客户端重新登录后导入' : undefined}>
          Token 剩余 <strong className={`wc-token-strong wc-token-strong--${tokenTone}`}>{tokenDaysLabel(tokenDays)}</strong>
        </span>
      </div>
      {switchingId === account.id ? (
        <div className="wc-slot-sub">{SWITCH_STAGE_LABELS[switchStage ?? ''] ?? '正在准备'}…</div>
      ) : null}
      {statusErrors.length ? <div className="wb-status-error">{statusErrors.join('；')}</div> : null}
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
      {confirmingSwitch ? <>
        <span className="wb-switch-confirm-hint">凭证已失效，仍要切换？</span>
        <button type="button" className="wc-btn wc-btn-danger" disabled={busy} onClick={requestSwitch}>{switchingId === account.id ? '切换中…' : '仍要切换'}</button>
        <button type="button" className="wc-btn" disabled={busy} onClick={() => setConfirmingSwitch(false)}>取消</button>
      </> : <button type="button" className="wc-btn wc-btn-primary" disabled={busy} onClick={requestSwitch}>{switchingId === account.id ? '切换中…' : '切换并打开'}</button>}
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

export function WorkBuddyPage() {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [setupOpen, setSetupOpen] = useState(false);
  // 同步前置检查（与 TRAE 页 handleSyncAll 对齐）：github.json 未启用或未填
  // 仓库时打开引导弹窗（repo-only 模式，不碰 TRAE 槽位），而不是直接报错。
  const syncConfigReady = async (): Promise<boolean> => {
    try {
      const config = await getWorkCnGitHubConfig();
      return config.enabled && /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(config.repository);
    } catch {
      return false;
    }
  };
  const handleSync = async () => {
    if (!(await syncConfigReady())) {
      setSetupOpen(true);
      return;
    }
    void syncGitHub();
  };
  // 引导弹窗关闭后若配置已就绪，直接继续同步，用户不用多点一次。
  const handleSetupClosed = async () => {
    setSetupOpen(false);
    if (await syncConfigReady()) {
      void syncGitHub();
    }
  };
  const accounts = useWorkBuddyStore((state) => state.accounts);
  const installation = useWorkBuddyStore((state) => state.installation);
  const installationLoading = useWorkBuddyStore((state) => state.installationLoading);
  const loading = useWorkBuddyStore((state) => state.loading);
  const syncing = useWorkBuddyStore((state) => state.syncing);
  const error = useWorkBuddyStore((state) => state.error);
  const notice = useWorkBuddyStore((state) => state.notice);
  const ensureAccountsLoaded = useWorkBuddyStore((state) => state.ensureAccountsLoaded);
  const ensureInstallationLoaded = useWorkBuddyStore((state) => state.ensureInstallationLoaded);
  const ensureMonitoringStarted = useWorkBuddyStore((state) => state.ensureMonitoringStarted);
  const refreshInstallation = useWorkBuddyStore((state) => state.refreshInstallation);
  const syncGitHub = useWorkBuddyStore((state) => state.syncGitHub);
  const refreshAllStatuses = useWorkBuddyStore((state) => state.refreshAllStatuses);
  const statusLoadingById = useWorkBuddyStore((state) => state.statusLoadingById);
  const clearFeedback = useWorkBuddyStore((state) => state.clearFeedback);
  const sessionWatchStatus = useWorkBuddyStore((state) => state.sessionWatchStatus);
  const loadSessionWatchStatus = useWorkBuddyStore((state) => state.loadSessionWatchStatus);
  useEffect(() => {
    void ensureInstallationLoaded();
    void ensureAccountsLoaded();
    void ensureMonitoringStarted();
    void loadSessionWatchStatus();
  }, [ensureAccountsLoaded, ensureInstallationLoaded, ensureMonitoringStarted, loadSessionWatchStatus]);
  const activeAccountId = installation?.currentAccountId ?? null;
  return <div className="wc-page workbuddy-page">
    <header className="wc-header">
      <h1 className="wc-title"><img className="wc-title-icon" src={workBuddyIcon} alt="WorkBuddy" />WorkBuddy 账号管理</h1>
      <div className="wc-header-actions">
        <button type="button" className="wc-btn wc-btn-primary" disabled={!installation?.installed} onClick={() => setDialogOpen(true)}>导入当前账号</button>
        <button type="button" className="wc-btn" disabled={syncing || (accounts.length === 0 && !installation?.githubCleanupPending)} onClick={() => void handleSync()}>{syncing ? '同步 WorkBuddy 中…' : accounts.length === 0 && installation?.githubCleanupPending ? '清理 WorkBuddy' : '同步 WorkBuddy'}</button>
        <button type="button" className="wc-btn" disabled={!accounts.length || Object.values(statusLoadingById).some(Boolean)} onClick={() => void refreshAllStatuses()}>刷新全部积分</button>
        <button type="button" className="wc-btn" onClick={() => setSettingsOpen(true)}>路径设置</button>
        <button type="button" className="wc-btn" onClick={refreshInstallation}>重新检测</button>
      </div>
    </header>
    <section className={`wc-status${installation?.installed ? ' wc-status--ok' : ' wc-status--error'}`}>
      <span className={`wc-status-dot${installation?.installed ? ' wc-status-dot--ok' : ' wc-status-dot--error'}`} />
      <div><div>{installationText(installation, installationLoading)}</div>{installation?.executablePath ? <div className="wc-status-detail">{installation.executablePath}</div> : null}</div>
    </section>
    {sessionWatchStatus?.message ? <section className={`wc-status${sessionWatchStatus.outcome === 'FAILED' || sessionWatchStatus.githubError ? ' wc-status--error' : sessionWatchStatus.outcome === 'TOKEN_UPDATED' ? ' wc-status--ok' : ''}`}><span className="wc-status-dot" /><div>{sessionWatchStatus.message}</div></section> : null}
    {error || notice ? <section className={`wc-status ${error ? 'wc-status--error' : 'wc-status--ok'}`}><div style={{ flex: 1 }}>{error ?? notice}</div><button type="button" className="wc-btn" onClick={clearFeedback}>知道了</button></section> : null}
    <section>
      <h2 className="wc-section-title">已保存账号（{accounts.length}{loading ? ' · 加载中…' : ''}）</h2>
      <div className="wc-slots wb-slots">
        {accounts.map((account) => <AccountCard key={account.id} account={account} active={account.id === activeAccountId} />)}
        {!accounts.length && !loading ? <div className="wc-slot wc-slot--empty"><span className="wc-slot-empty-icon">＋</span><span className="wc-slot-empty-hint">先登录官方 WorkBuddy 客户端，再导入当前账号</span></div> : null}
      </div>
    </section>
    {/* 云端签到任务（共享面板）：与 TRAE / 智谱 同一签到仓库，可查看运行记录并手动触发验证。
        触发条件由后端校验（github.json 未配置时返回友好提示）。 */}
    <CloudCheckinPanel />
    <WorkBuddyAddAccountDialog open={dialogOpen} onClose={() => setDialogOpen(false)} />
    <WorkBuddySettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} onSaved={refreshInstallation} />
    {/* GitHub 上传引导（repo-only 模式）：同步前置条件缺失时打开，
        只配置仓库，保存不影响 TRAE 页已绑定的槽位。 */}
    <GhSetupDialog open={setupOpen} onClose={() => void handleSetupClosed()} mode="repo-only" />
  </div>;
}
