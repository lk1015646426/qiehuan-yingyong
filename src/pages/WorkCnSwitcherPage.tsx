// TRAE Work CN 账号切换器 - 主页面
//
// 样式全部走 work-cn.css（基于 base.css 设计系统 token，自动适配暗色主题）：
// - 快照完整度折叠为一行摘要（悬浮 title 展示缺失项明细）
// - 积分区带 已用/总量 进度条
// - 当前活跃账号（后台会话监测 accountId）高亮描边 + 角标

import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import traeCnIcon from '../assets/icons/trae-cn.png';
import { getWorkCnInstallation, WORK_CN_SESSION_WATCH_EVENT } from '../services/workCnService';
import { useWorkCnStore } from '../stores/useWorkCnStore';
import { WorkCnAddAccountDialog } from '../components/work-cn/WorkCnAddAccountDialog';
import { WorkCnSettingsDialog } from '../components/work-cn/WorkCnSettingsDialog';
import { WorkCnStatusBanner } from '../components/work-cn/WorkCnStatusBanner';
import type { WorkCnInstallation, WorkCnAccountView, WorkCnCreditsSummary, WorkCnGitHubSyncResult, WorkCnSessionWatchStatus } from '../types/workCn';
import { compactUid } from '../utils/accountCardPresentation';

// 快照字段中文名（与后端 WorkCnSnapshotValidation 对应），用于摘要明细。
const SNAPSHOT_FIELDS: Array<[keyof WorkCnAccountView, string]> = [
  ['hasAccessToken', '访问令牌'],
  ['hasRefreshToken', '刷新令牌'],
  ['hasUserId', '用户ID'],
  ['hasAuthDeviceId', 'Auth设备ID'],
  ['hasCheckinDeviceId', 'Checkin设备ID'],
  ['hasMachineId', '机器ID'],
  ['hasDevicePrivateKey', '设备私钥'],
  ['hasDevicePublicKey', '设备公钥'],
];

function renderInstallationStatus(
  installation: WorkCnInstallation | null,
  loading: boolean,
  error: string | null,
): { tone: string; line: string; detail?: string } {
  if (loading) {
    return { tone: '', line: '客户端检测：正在检测 TRAE Work CN 安装…' };
  }
  if (error) {
    return {
      tone: 'wc-status--error',
      line: '客户端检测失败，请稍后重试或在设置中手动指定路径',
      detail: error,
    };
  }
  if (!installation || !installation.installed) {
    const legacyNote = installation?.legacyPath ? '（兼容旧数据目录）' : '';
    const detail = installation?.userDataDir
      ? `数据目录：${installation.userDataDir}${legacyNote}`
      : undefined;
    return {
      tone: 'wc-status--error',
      line: '未检测到 TRAE Work CN，请在设置中选择 EXE 路径',
      detail,
    };
  }
  const versionText = installation.version ? ` ${installation.version}` : '';
  const dataDirText = installation.userDataDir ?? '';
  const legacyNote = installation.legacyPath ? '（兼容旧数据目录）' : '';
  return {
    tone: 'wc-status--ok',
    line: `已检测到 TRAE Work CN${versionText}`,
    detail: [installation.executablePath, dataDirText ? `数据目录：${dataDirText}${legacyNote}` : '']
      .filter(Boolean)
      .join('\n'),
  };
}

// 8 个技术徽章折叠为一行「快照 N/8」摘要；缺失时悬浮 title 列出缺失项。
function SnapshotSummary({ account }: { account: WorkCnAccountView }) {
  const missing = SNAPSHOT_FIELDS.filter(([key]) => !account[key]).map(([, label]) => label);
  const total = SNAPSHOT_FIELDS.length;
  const ok = missing.length === 0;
  return (
    <div
      className={ok ? 'wc-snapshot-summary wc-snapshot-summary--ok' : 'wc-snapshot-summary wc-snapshot-summary--missing'}
      title={ok ? '登录快照字段完整，可安全切换' : `快照缺失 ${missing.length} 项：${missing.join('、')}\n请重新登录该账号并导入`}
    >
      {ok ? `✓ 快照完整 ${total}/${total}` : `快照 ${total - missing.length}/${total} · 缺 ${missing.length} 项`}
    </div>
  );
}

function formatCreditsValue(value: number): string {
  // 接口用量带小数（如 864.63）；浮点累加会产生尾差，四舍五入到 2 位并去掉多余尾零。
  const rounded = Math.round(value * 100) / 100;
  return Number.isInteger(rounded) ? String(rounded) : rounded.toFixed(2);
}

function CreditsBlock({
  credits,
  error,
}: {
  credits: WorkCnCreditsSummary | null;
  error: string | null;
}) {
  if (error) {
    return <div className="wc-credits-error">积分查询失败：{error}</div>;
  }
  if (!credits) {
    return null;
  }
  if (credits.unlimited) {
    return <div className="wc-credits-value">剩余积分：无限</div>;
  }
  if (credits.total == null) {
    return <div className="wc-credits-sub">剩余积分：暂无积分数据</div>;
  }
  return (
    <div className="wc-credits-value">剩余 {formatCreditsValue(credits.remaining ?? 0)} 积分</div>
  );
}

function AccountCard({
  account,
  credits,
  creditsError,
  switching,
  deleting,
  refreshingCredits,
  githubEnabled,
  githubSync,
  active,
  onSwitch,
  onRefreshCredits,
  onDelete,
}: {
  account: WorkCnAccountView;
  credits: WorkCnCreditsSummary | null;
  creditsError: string | null;
  switching: boolean;
  deleting: boolean;
  refreshingCredits: boolean;
  githubEnabled: boolean;
  githubSync: WorkCnGitHubSyncResult | null;
  active: boolean;
  onSwitch: () => void;
  onRefreshCredits: () => void;
  onDelete: () => void;
}) {
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const title = account.tags?.length
    ? account.tags[0]
    : account.nickname ?? account.email ?? account.userId ?? account.id;
  const canSwitch = account.validForSwitch && !switching;

  let githubLine = 'GitHub：未同步';
  let githubTone = '';
  if (!githubEnabled) {
    githubLine = 'GitHub：未启用';
  } else if (githubSync?.synced) {
    githubLine = 'GitHub：已同步';
    githubTone = ' wc-github-line--ok';
  } else if (githubSync?.error) {
    githubLine = `GitHub 同步失败：${githubSync.error}`;
    githubTone = ' wc-github-line--error';
  } else if (githubSync?.skipped) {
    githubLine = `GitHub 待同步：${githubSync.skipReason ?? '未绑定槽位'}`;
    githubTone = ' wc-github-line--warn';
  }

  return (
    <div className={active ? 'wc-slot account-card account-card--compact wc-slot--active' : 'wc-slot account-card account-card--compact'}>
      {active ? <span className="wc-slot-active-badge">使用中</span> : null}
      <div className="wc-slot-head account-card__head">
        <div className="wc-slot-title" title={title}>{title}</div>
        <SnapshotSummary account={account} />
      </div>
      <div className="wc-slot-sub account-card__identity" title={account.userId ?? undefined}>
        {account.email ?? '未提供邮箱'} · UID {compactUid(account.userId)}
      </div>
      <div className="account-card__metrics">
        <CreditsBlock credits={credits} error={creditsError} />
        <div className="wc-credits-sub">{account.validForSwitch ? '快照可切换' : '快照需重新导入'}</div>
      </div>
      <div className="account-card__status">
        <div
          className={`wc-github-line${githubTone}`}
          title={[githubLine, ...account.warnings].join('；')}
        >
          {githubLine}{account.warnings.length ? ` · ${account.warnings.join('；')}` : ''}
        </div>
      </div>
      <div className="wc-slot-actions account-card__actions">
        <button
          type="button"
          className={account.validForSwitch ? 'wc-btn wc-btn-primary' : 'wc-btn'}
          disabled={!canSwitch}
          onClick={onSwitch}
          title={
            account.validForSwitch
              ? '关闭当前客户端并注入该账号后打开'
              : '设备密钥不完整，请重新登录并导入'
          }
        >
          {switching ? '切换中…' : account.validForSwitch ? '切换并打开' : '快照不完整'}
        </button>
        <button
          type="button"
          className="wc-btn"
          disabled={refreshingCredits || switching}
          onClick={onRefreshCredits}
          title="仅查询积分，绝不签到"
        >
          {refreshingCredits ? '查询中…' : '刷新积分'}
        </button>
        {confirmingDelete ? (
          <>
            <button
              type="button"
              className="wc-btn wc-btn-danger"
              disabled={deleting}
              onClick={onDelete}
              title="删除该账号槽位（不影响官方客户端登录态）"
            >
              {deleting ? '删除中…' : '确认删除'}
            </button>
            <button
              type="button"
              className="wc-btn"
              disabled={deleting}
              onClick={() => setConfirmingDelete(false)}
            >
              取消
            </button>
          </>
        ) : (
          <button
            type="button"
            className="wc-btn"
            disabled={switching || deleting}
            onClick={() => setConfirmingDelete(true)}
            title="从账号库删除该账号槽位"
          >
            删除
          </button>
        )}
      </div>
    </div>
  );
}

function StoreErrorBanner() {
  const error = useWorkCnStore((s) => s.error);
  const clearError = useWorkCnStore((s) => s.clearError);
  if (!error) {
    return null;
  }
  return (
    <section className="wc-status wc-status--error">
      <span className="wc-status-dot wc-status-dot--error" />
      <div style={{ flex: 1 }}>{error}</div>
      <button type="button" className="wc-btn" onClick={clearError}>
        知道了
      </button>
    </section>
  );
}

export function WorkCnSwitcherPage() {
  const [installation, setInstallation] = useState<WorkCnInstallation | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

  const accounts = useWorkCnStore((s) => s.accounts);
  const storeLoading = useWorkCnStore((s) => s.loading);
  const ensureAccountsLoaded = useWorkCnStore((s) => s.ensureAccountsLoaded);
  const lastImportWarning = useWorkCnStore((s) => s.lastImportWarning);
  const switchingId = useWorkCnStore((s) => s.switchingId);
  const switchTo = useWorkCnStore((s) => s.switchTo);
  const deletingId = useWorkCnStore((s) => s.deletingId);
  const deleteAccount = useWorkCnStore((s) => s.deleteAccount);
  const openLogs = useWorkCnStore((s) => s.openLogs);
  const creditsById = useWorkCnStore((s) => s.creditsById);
  const creditsErrorById = useWorkCnStore((s) => s.creditsErrorById);
  const refreshingCreditsIds = useWorkCnStore((s) => s.refreshingCreditsIds);
  const refreshCredits = useWorkCnStore((s) => s.refreshCredits);
  const githubConfig = useWorkCnStore((s) => s.githubConfig);
  const githubSyncResultById = useWorkCnStore((s) => s.githubSyncResultById);
  const loadGitHubConfig = useWorkCnStore((s) => s.loadGitHubConfig);
  const syncGitHubAll = useWorkCnStore((s) => s.syncGitHubAll);
  const syncingAllGithub = useWorkCnStore((s) => s.syncingAllGithub);
  const syncAllProgress = useWorkCnStore((s) => s.syncAllProgress);
  const sessionWatchStatus = useWorkCnStore((s) => s.sessionWatchStatus);
  const loadSessionWatchStatus = useWorkCnStore((s) => s.loadSessionWatchStatus);
  const applySessionWatchStatus = useWorkCnStore((s) => s.applySessionWatchStatus);
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    getWorkCnInstallation()
      .then((result) => {
        if (!cancelled) {
          setInstallation(result);
          setError(null);
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    void ensureAccountsLoaded();
    void loadGitHubConfig();
    return () => {
      cancelled = true;
    };
  }, [ensureAccountsLoaded, loadGitHubConfig]);

  useEffect(() => {
    void loadSessionWatchStatus();
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<WorkCnSessionWatchStatus>(
      WORK_CN_SESSION_WATCH_EVENT,
      (event) => {
        if (!disposed) {
          applySessionWatchStatus(event.payload);
        }
      },
    ).then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [loadSessionWatchStatus, applySessionWatchStatus]);

  const status = renderInstallationStatus(installation, loading, error);

  // 当前活跃账号：优先取后台会话监测到的 accountId；未监测时回退到最近使用。
  const activeAccountId =
    sessionWatchStatus?.accountId ??
    (accounts.some((a) => a.lastUsed > 0)
      ? accounts.reduce((latest, a) => (a.lastUsed > latest.lastUsed ? a : latest)).id
      : null);

  return (
    <div className="wc-page">
      <header className="wc-header">
        <h1 className="wc-title">
          <img className="wc-title-icon" src={traeCnIcon} alt="TRAE" />
          TRAE Work CN 账号切换器
        </h1>
        <div className="wc-header-actions">
          <button
            type="button"
            className="wc-btn wc-btn-primary"
            onClick={() => setDialogOpen(true)}
            disabled={!installation?.installed}
            title={installation?.installed ? '' : '请先安装并登录 TRAE Work CN'}
          >
            导入当前账号
          </button>
          <button
            type="button"
            className="wc-btn"
            onClick={() => void syncGitHubAll()}
            disabled={syncingAllGithub || !githubConfig.enabled || accounts.length === 0}
            title="把所有已绑定槽位账号的最新凭证（按账号槽位名命名的 *_TOKEN / *_DEVICE_ID）一键同步到 GitHub Secrets，供 daily-checkin 工作流签到使用"
          >
            {syncingAllGithub && syncAllProgress
              ? `同步中 ${syncAllProgress.done}/${syncAllProgress.total}…`
              : '同步全部 GitHub'}
          </button>
          <button
            type="button"
            className="wc-btn"
            onClick={() => setSettingsOpen(true)}
          >
            设置
          </button>
          <button
            type="button"
            className="wc-btn"
            onClick={() => void openLogs()}
            title="在系统文件管理器中打开日志目录"
          >
            日志
          </button>
        </div>
      </header>

      <section className={`wc-status${status.tone ? ` ${status.tone}` : ''}`}>
        <span className={`wc-status-dot${status.tone === 'wc-status--ok' ? ' wc-status-dot--ok' : status.tone === 'wc-status--error' ? ' wc-status-dot--error' : ''}`} />
        <div>
          <div>{status.line}</div>
          {status.detail ? (
            <div className="wc-status-detail">
              {status.detail.split('\n').map((line, index) => (
                <div key={index}>{line}</div>
              ))}
            </div>
          ) : null}
        </div>
      </section>

      {lastImportWarning ? (
        <section className="wc-status wc-status--warn">{lastImportWarning}</section>
      ) : null}

      <WorkCnStatusBanner />

      <StoreErrorBanner />

      <section>
        <h2 className="wc-section-title">
          账号槽位（{accounts.length}，数量不限）
          {storeLoading ? ' · 加载中…' : ''}
        </h2>
        <div className="wc-slots">
          {accounts.map((account) => (
            <AccountCard
              key={account.id}
              account={account}
              credits={creditsById[account.id] ?? null}
              creditsError={creditsErrorById[account.id] ?? null}
              switching={switchingId === account.id}
              deleting={deletingId === account.id}
              refreshingCredits={!!refreshingCreditsIds[account.id]}
              githubEnabled={githubConfig.enabled}
              githubSync={githubSyncResultById[account.id] ?? null}
              active={account.id === activeAccountId}
              onSwitch={() => void switchTo(account.id)}
              onRefreshCredits={() => void refreshCredits(account.id, true)}
              onDelete={() => void deleteAccount(account.id)}
            />
          ))}
          <div className="wc-slot wc-slot--empty">
            <span className="wc-slot-empty-icon">＋</span>
            <span className="wc-slot-empty-hint">新增槽位 · 点击右上「导入当前账号」按顺序追加</span>
          </div>
        </div>
      </section>

      <WorkCnAddAccountDialog open={dialogOpen} onClose={() => setDialogOpen(false)} />
      <WorkCnSettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
}
