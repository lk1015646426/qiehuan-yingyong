import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { LoaderCircle } from 'lucide-react';
import workBuddyIcon from '../assets/icons/workbuddy.png';
import { CloudCheckinPanel } from '../components/checkin/CloudCheckinPanel';
import { GhSetupDialog } from '../components/work-cn/GhSetupDialog';
import { getWorkCnGitHubConfig } from '../services/workCnService';
import { WorkBuddyAddAccountDialog } from '../components/workbuddy/WorkBuddyAddAccountDialog';
import { WorkBuddySettingsDialog } from '../components/workbuddy/WorkBuddySettingsDialog';
import { WorkBuddyPoolPanel } from '../components/workbuddy/WorkBuddyPoolPanel';
import {
  AccountManagementLayout, AccountList, AccountCardFrame, AccountCardActions, AccountStatusBanner,
  LocalCheckinFeedback, type AccountCheckinResult,
} from '../components/AccountManagementLayout';
import { WorkBuddyActivitiesPanel } from '../components/workbuddy/WorkBuddyActivitiesPanel';
import { useWorkBuddyStore } from '../stores/useWorkBuddyStore';
import { WorkBuddyCreditsSummary } from '../components/workbuddy/WorkBuddyCreditsSummary';
import type { WorkBuddyAccountView, WorkBuddyInstallation } from '../types/workbuddy';
import { credentialInvalidated, githubSyncPresentation } from '../utils/accountCardPresentation';
// 切号各阶段文案（stage 与 Rust 事件 workbuddy-switch-progress 一致）。
const SWITCH_STAGE_LABELS: Record<string, string> = {
  validating: '正在校验账号快照',
  closing: '正在关闭客户端',
  refreshing: '正在刷新目标账号凭证',
  injecting: '正在注入账号凭证',
  launching: '正在启动客户端',
  activating: '正在将 WorkBuddy 窗口置前',
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
  const recoverCredentials = useWorkBuddyStore((state) => state.recoverCredentials);
  const [deleting, setDeleting] = useState(false);
  const [editing, setEditing] = useState(false);
  const [confirmingSwitch, setConfirmingSwitch] = useState(false);
  const [name, setName] = useState(account.displayName);
  const [openingCheckin, setOpeningCheckin] = useState(false);
  const [checkinHint, setCheckinHint] = useState('');
  const openClientCheckin = async () => {
    if (!active || openingCheckin) return;
    setOpeningCheckin(true);
    setCheckinHint('请在客户端核对账号，点击左下角头像 → Buddy 加油站／领取今日礼包；完成后返回刷新积分。');
    try {
      await invoke('open_workbuddy_checkin_client', { accountId: account.id });
    } catch (error) {
      setCheckinHint('未能打开窗口：' + String(error) + '。请手动打开客户端并核对登录。');
    } finally {
      setOpeningCheckin(false);
    }
  };
  const busy = switchingId !== null || updatingId !== null || deletingId !== null || checkingInId !== null;
  const githubSync = githubSyncPresentation(account.lastGithubSyncState, account.lastGithubSyncError);
  const saveName = async () => {
    const value = name.trim();
    if (!value) return;
    await updateAccount(account.id, { displayName: value });
    setEditing(false);
  };
  const statusSummary = `${account.checkinEnabled ? '云端自动签到已开启' : '云端自动签到未开启'} · ${githubSync.label}`;
  const statusDetail = githubSync.detail ?? '';
  const statusErrors = [status?.creditsError, status?.activityError]
    .filter((value): value is string => Boolean(value))
    .filter((value, index, values) => values.indexOf(value) === index);
  // 查询被拒绝不等于已确认异地登录或封号；认证恢复由显式操作触发。
  const credentialInvalid = credentialInvalidated([status?.creditsError, status?.activityError]);
  const requestSwitch = () => {
    if (credentialInvalid && !confirmingSwitch) {
      setConfirmingSwitch(true);
      return;
    }
    setConfirmingSwitch(false);
    void switchTo(account.id);
  };
  return <AccountCardFrame title={account.displayName} active={active}
    contact={account.maskedPhone} uid={account.uid} tokenExpiresAt={account.tokenExpiresAt}
    credits={statusLoading && !status
      ? '查询中…'
      : <>{status?.credits?.toLocaleString('zh-CN', { maximumFractionDigits: 2 }) ?? '未查询到'}
        {statusLoading && '（刷新中）'}</>}
    todayReward={status?.todayReward ?? null}
    streakDays={status?.streakDays ?? null}
    alert={credentialInvalid || switchingId === account.id || statusErrors.length ? <>
      {credentialInvalid ? <div className="wb-credential-invalid" role="alert">保存的登录凭证被拒绝，原因尚未确认。可手动恢复认证；当前账号由官方客户端管理时，请核对登录后重新导入。</div> : null}
      {switchingId === account.id ? <div className="wc-slot-sub wb-switch-progress" role="status">
        <LoaderCircle size={14} className="wb-progress-spinner" aria-hidden="true" />
        {SWITCH_STAGE_LABELS[switchStage ?? ''] ?? '正在准备切号'}…
      </div> : null}
      {statusErrors.length ? <div className="wb-status-error" role="alert">{statusErrors.join('；')}</div> : null}
    </> : undefined}
    details={<>
      {status?.credits != null ? <div className="wc-slot-sub">积分查询于 {new Date(status.updatedAt * 1000).toLocaleTimeString('zh-CN', { hour12: false })} · 页面打开时每分钟更新</div> : null}
      <div className={`wb-sync wb-sync--${githubSync.tone}`} title={githubSync.detail ?? githubSync.label}>{githubSync.label}</div>
      <div className="account-card__status account-card__status--toggle" title={statusDetail || statusSummary}>
        <label className="wb-checkin-toggle">
          <input type="checkbox" checked={account.checkinEnabled} disabled={busy} onChange={(event) => void updateAccount(account.id, { checkinEnabled: event.target.checked })} />
          <span>{statusSummary}</span>
        </label>
      </div>
    </>}
    recovery={{ onClick: () => void recoverCredentials(account.id), disabled: busy, label: updatingId === account.id ? '处理中…' : undefined }}
    cloudCheckin={{
      onClick: () => void triggerCheckin(account.id), disabled: busy || !account.checkinEnabled,
      label: checkingInId === account.id ? '触发中…' : undefined,
    }}>
    {active && checkinHint ? <div className="uac-checkin-hint" role="status">{checkinHint}</div> : null}
    {editing ? <div className="wb-edit-name">
      <input className="wc-input" aria-label="账号备注" value={name} maxLength={80} onChange={(event) => setName(event.target.value)} />
      <button type="button" className="wc-btn" onClick={() => void saveName()}>保存</button>
      <button type="button" className="wc-btn" onClick={() => { setName(account.displayName); setEditing(false); }}>取消</button>
    </div> : null}
    <AccountCardActions
      switchAccount={{ onClick: requestSwitch, disabled: busy, label: switchingId === account.id ? '切换中…' : undefined }}
      cancelSwitch={confirmingSwitch ? { onClick: () => setConfirmingSwitch(false), disabled: busy } : undefined}
      checkin={{
        onClick: () => void openClientCheckin(), disabled: busy || openingCheckin || !active,
        label: openingCheckin ? '打开中…' : undefined,
        title: active ? '打开已登录客户端，由你在官方界面签到' : '仅当前客户端账号可使用',
      }}
      refresh={{
        onClick: () => void refreshAccountStatus(account.id), disabled: busy || statusLoading,
        label: statusLoading ? '刷新中…' : undefined,
      }}
      rename={{ onClick: () => setEditing((value) => !value), disabled: busy }}
      remove={{ onClick: deleting ? () => void deleteAccount(account.id) : () => setDeleting(true), disabled: busy }}
      cancelRemove={deleting ? { onClick: () => setDeleting(false), disabled: busy } : undefined}
    />
  </AccountCardFrame>;
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
  const statusById = useWorkBuddyStore((state) => state.statusById);
  const clearFeedback = useWorkBuddyStore((state) => state.clearFeedback);
  const sessionWatchStatus = useWorkBuddyStore((state) => state.sessionWatchStatus);
  const loadSessionWatchStatus = useWorkBuddyStore((state) => state.loadSessionWatchStatus);
  useEffect(() => {
    void ensureInstallationLoaded();
    void ensureAccountsLoaded();
    void ensureMonitoringStarted();
    void loadSessionWatchStatus();
  }, [ensureAccountsLoaded, ensureInstallationLoaded, ensureMonitoringStarted, loadSessionWatchStatus]);
  useEffect(() => {
    const refreshIfStale = () => {
      if (document.visibilityState !== 'visible') return;
      const state = useWorkBuddyStore.getState();
      const now = Math.floor(Date.now() / 1000);
      if (state.accounts.some(account => !state.statusById[account.id]
        || now - state.statusById[account.id].updatedAt >= 60)) void refreshAllStatuses();
    };
    const timer = window.setInterval(refreshIfStale, 60_000);
    window.addEventListener('focus', refreshIfStale);
    document.addEventListener('visibilitychange', refreshIfStale);
    refreshIfStale();
    return () => {
      window.clearInterval(timer);
      window.removeEventListener('focus', refreshIfStale);
      document.removeEventListener('visibilitychange', refreshIfStale);
    };
  }, [refreshAllStatuses, accounts]);
  const [localCheckingIn, setLocalCheckingIn] = useState(false);
  const [localCheckinResults, setLocalCheckinResults] = useState<AccountCheckinResult[] | null>(null);
  const [localCheckinError, setLocalCheckinError] = useState('');
  const runLocalCheckin = async () => {
    setLocalCheckingIn(true);
    setLocalCheckinError('');
    setLocalCheckinResults(null);
    try {
      const results = await invoke<AccountCheckinResult[]>('local_checkin_all_workbuddy_accounts');
      setLocalCheckinResults(results);
      if (results.some(({ outcome }) => 'Ok' in outcome)) void refreshAllStatuses();
    } catch (error) {
      setLocalCheckinError(String(error));
    } finally {
      setLocalCheckingIn(false);
    }
  };
  const activeAccountId = installation?.currentAccountId ?? null;
  const updatingCredits = Object.values(statusLoadingById).some(Boolean);
  const cleanupPending = accounts.length === 0 && installation?.githubCleanupPending;
  return <AccountManagementLayout app="WorkBuddy" icon={workBuddyIcon} actions={{
    importAccount: { onClick: () => setDialogOpen(true), disabled: !installation?.installed },
    sync: {
      onClick: () => void handleSync(), disabled: syncing || (!accounts.length && !cleanupPending),
      label: syncing ? '同步中…' : cleanupPending ? '清理 WorkBuddy' : undefined,
    },
    refresh: { onClick: () => void refreshAllStatuses(), disabled: !accounts.length || updatingCredits },
    checkin: {
      onClick: () => void runLocalCheckin(), disabled: loading || localCheckingIn || !accounts.length,
      label: localCheckingIn ? '签到中…' : undefined, title: '为所有已保存的 WorkBuddy 账号发起本地签到',
    },
    settings: { onClick: () => setSettingsOpen(true) },
    detect: { onClick: () => void refreshInstallation() },
  }}>
    <AccountStatusBanner tone={installationLoading ? undefined : installation?.installed ? 'wc-status--ok' : 'wc-status--error'} detail={installation?.executablePath}>
      {installationText(installation, installationLoading)}
    </AccountStatusBanner>
    {sessionWatchStatus?.message ? <section className={`wc-status${sessionWatchStatus.outcome === 'FAILED' || sessionWatchStatus.githubError ? ' wc-status--error' : sessionWatchStatus.outcome === 'TOKEN_UPDATED' ? ' wc-status--ok' : ''}`}><span className="wc-status-dot" /><div>{sessionWatchStatus.message}</div></section> : null}
    {error || notice ? <section className={`wc-status ${error ? 'wc-status--error' : 'wc-status--ok'}`}><div style={{ flex: 1 }}>{error ?? notice}</div><button type="button" className="wc-btn" onClick={clearFeedback}>知道了</button></section> : null}
    <LocalCheckinFeedback running={localCheckingIn} results={localCheckinResults} error={localCheckinError} />
    <WorkBuddyPoolPanel />
    <AccountList count={accounts.length} loading={loading} emptyHint="先登录官方 WorkBuddy 客户端，再导入当前账号">
      {accounts.map((account) => <AccountCard key={account.id} account={account} active={account.id === activeAccountId} />)}
    </AccountList>
    <WorkBuddyCreditsSummary accountIds={accounts.map(account => account.id)} statuses={statusById} loading={updatingCredits} />
    {/* 云端签到任务（共享面板）：与 TRAE 同一签到仓库，可查看运行记录并手动触发验证。
        触发条件由后端校验（github.json 未配置时返回友好提示）。 */}
    <WorkBuddyActivitiesPanel />
    <CloudCheckinPanel />
    <WorkBuddyAddAccountDialog open={dialogOpen} onClose={() => setDialogOpen(false)} />
    <WorkBuddySettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} onSaved={refreshInstallation} />
    {/* GitHub 上传引导（repo-only 模式）：同步前置条件缺失时打开，
        只配置仓库，保存不影响 TRAE 页已绑定的槽位。 */}
    <GhSetupDialog open={setupOpen} onClose={() => void handleSetupClosed()} mode="repo-only" />
  </AccountManagementLayout>;
}
