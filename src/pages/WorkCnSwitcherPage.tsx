// TRAE Work CN 账号切换器 - 主页面
//
// 阶段 1：渲染静态应用壳框架。
// 阶段 2：调用 get_work_cn_installation 在状态栏显示客户端检测结果。
// 阶段 3：接入"导入当前账号"（完整快照），并展示已导入账号及其快照完整度。
//         一键切换、积分查询等能力将在后续阶段接入。

import { useEffect, useState } from 'react';
import type { CSSProperties, ReactElement } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getWorkCnInstallation, WORK_CN_SESSION_WATCH_EVENT } from '../services/workCnService';
import { useWorkCnStore } from '../stores/useWorkCnStore';
import { WorkCnAddAccountDialog } from '../components/work-cn/WorkCnAddAccountDialog';
import { WorkCnSettingsDialog } from '../components/work-cn/WorkCnSettingsDialog';
import { WorkCnStatusBanner } from '../components/work-cn/WorkCnStatusBanner';
import type { WorkCnInstallation, WorkCnAccountView, WorkCnCreditsSummary, WorkCnGitHubSyncResult, WorkCnSessionWatchStatus } from '../types/workCn';

const ACCOUNT_SLOT_COUNT = 4;

const pageStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  minWidth: 0,
  minHeight: '100vh',
  padding: '20px 24px',
  boxSizing: 'border-box',
  background: '#f5f6f8',
  color: '#1f2430',
  fontFamily: "'Segoe UI', 'Microsoft YaHei', 'PingFang SC', system-ui, sans-serif",
  gap: 16,
};

const headerStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  gap: 12,
};

const titleStyle: CSSProperties = {
  margin: 0,
  fontSize: 20,
  fontWeight: 600,
  letterSpacing: 0.2,
};

const headerActionsStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
};

const buttonStyle: CSSProperties = {
  height: 32,
  padding: '0 14px',
  fontSize: 13,
  color: '#1f2430',
  background: '#ffffff',
  border: '1px solid #d7dae0',
  borderRadius: 6,
  cursor: 'pointer',
};

const primaryButtonStyle: CSSProperties = {
  ...buttonStyle,
  color: '#ffffff',
  background: '#2563eb',
  border: '1px solid #2563eb',
};

const statusStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  padding: '10px 14px',
  background: '#ffffff',
  border: '1px solid #e4e7ec',
  borderRadius: 8,
  fontSize: 13,
  color: '#525866',
};

const statusDotStyle: CSSProperties = {
  width: 8,
  height: 8,
  borderRadius: '50%',
  background: '#c4c8d0',
  flexShrink: 0,
};

const sectionTitleStyle: CSSProperties = {
  margin: '4px 0 0',
  fontSize: 14,
  fontWeight: 600,
  color: '#1f2430',
};

const slotsStyle: CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
  gap: 12,
};

const slotStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 8,
  padding: '16px 18px',
  background: '#ffffff',
  border: '1px solid #e4e7ec',
  borderRadius: 10,
  minHeight: 132,
};

const slotTitleStyle: CSSProperties = {
  fontSize: 14,
  fontWeight: 600,
  color: '#1f2430',
};

const slotStatusStyle: CSSProperties = {
  fontSize: 12,
  color: '#8a909c',
};

const badgeRowStyle: CSSProperties = {
  display: 'flex',
  flexWrap: 'wrap',
  gap: 6,
};

const badgeStyle: CSSProperties = {
  fontSize: 11,
  padding: '2px 7px',
  borderRadius: 999,
  border: '1px solid #e4e7ec',
  color: '#525866',
};

const badgeOkStyle: CSSProperties = {
  ...badgeStyle,
  color: '#15803d',
  borderColor: '#bbf7d0',
  background: '#f0fdf4',
};

const badgeWarnStyle: CSSProperties = {
  ...badgeStyle,
  color: '#b45309',
  borderColor: '#fde68a',
  background: '#fffbeb',
};

const warningStyle: CSSProperties = {
  fontSize: 11,
  color: '#b45309',
  lineHeight: 1.5,
};

const statusDotOkStyle: CSSProperties = {
  ...statusDotStyle,
  background: '#22c55e',
};

const statusDotErrorStyle: CSSProperties = {
  ...statusDotStyle,
  background: '#ef4444',
};

function renderInstallationStatus(
  installation: WorkCnInstallation | null,
  loading: boolean,
  error: string | null,
): { dot: CSSProperties; line: string; detail?: string } {
  if (loading) {
    return { dot: statusDotStyle, line: '客户端检测：正在检测 TRAE Work CN 安装…' };
  }
  if (error) {
    return {
      dot: statusDotErrorStyle,
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
      dot: statusDotErrorStyle,
      line: '未检测到 TRAE Work CN，请在设置中选择 EXE 路径',
      detail,
    };
  }
  const versionText = installation.version ? ` ${installation.version}` : '';
  const dataDirText = installation.userDataDir ?? '';
  const legacyNote = installation.legacyPath ? '（兼容旧数据目录）' : '';
  return {
    dot: statusDotOkStyle,
    line: `已检测到 TRAE Work CN${versionText}`,
    detail: [installation.executablePath, dataDirText ? `数据目录：${dataDirText}${legacyNote}` : '']
      .filter(Boolean)
      .join('\n'),
  };
}

function SnapshotBadges({ account }: { account: WorkCnAccountView }) {
  const items: Array<[boolean, string]> = [
    [account.hasAccessToken, '令牌'],
    [account.hasRefreshToken, '刷新令牌'],
    [account.hasUserId, '用户ID'],
    [account.hasAuthDeviceId, 'Auth设备ID'],
    [account.hasCheckinDeviceId, 'Checkin设备ID'],
    [account.hasMachineId, '机器ID'],
    [account.hasDevicePrivateKey, '私钥'],
    [account.hasDevicePublicKey, '公钥'],
  ];
  return (
    <div style={badgeRowStyle}>
      {items.map(([ok, label]) => (
        <span key={label} style={ok ? badgeOkStyle : badgeWarnStyle}>
          {ok ? '✓ ' : '✗ '}
          {label}
        </span>
      ))}
    </div>
  );
}

function renderCredits(
  credits: WorkCnCreditsSummary | null | undefined,
  error: string | null | undefined,
): ReactElement | null {
  if (error) {
    return <div style={warningStyle}>积分查询失败：{error}</div>;
  }
  if (!credits) {
    return null;
  }
  if (credits.unlimited) {
    return (
      <div style={{ fontSize: 20, fontWeight: 700, color: '#1f2430' }}>剩余积分：无限</div>
    );
  }
  if (credits.total == null) {
    return <div style={slotStatusStyle}>剩余积分：暂无积分数据</div>;
  }
  return (
    <>
      <div style={{ fontSize: 20, fontWeight: 700, color: '#1f2430' }}>
        剩余 {credits.remaining} 积分
      </div>
      <div style={slotStatusStyle}>
        已用 {credits.used} / 总 {credits.total}
      </div>
    </>
  );
}

function AccountCard({
  account,
  credits,
  creditsError,
  switching,
  refreshingCredits,
  githubEnabled,
  githubSync,
  githubSyncing,
  onSwitch,
  onRefreshCredits,
  onSyncGitHub,
}: {
  account: WorkCnAccountView;
  credits: WorkCnCreditsSummary | null;
  creditsError: string | null;
  switching: boolean;
  refreshingCredits: boolean;
  githubEnabled: boolean;
  githubSync: WorkCnGitHubSyncResult | null;
  githubSyncing: boolean;
  onSwitch: () => void;
  onRefreshCredits: () => void;
  onSyncGitHub: () => void;
}) {
  const title = account.tags?.length
    ? account.tags[0]
    : account.nickname ?? account.email ?? account.userId ?? account.id;
  const canSwitch = account.validForSwitch && !switching;

  let githubLine = 'GitHub：未同步';
  let githubColor = '#8a909c';
  if (!githubEnabled) {
    githubLine = 'GitHub：未启用';
  } else if (githubSync?.synced) {
    githubLine = 'GitHub：已同步';
    githubColor = '#16a34a';
  } else if (githubSync?.error) {
    githubLine = `GitHub 同步失败：${githubSync.error}`;
    githubColor = '#ef4444';
  } else if (githubSync?.skipped) {
    githubLine = `GitHub 待同步：${githubSync.skipReason ?? '未绑定槽位'}`;
    githubColor = '#f59e0b';
  }

  return (
    <div style={slotStyle}>
      <div style={slotTitleStyle}>{title}</div>
      <div style={slotStatusStyle}>
        {account.validForSwitch ? '快照完整 · 可切换' : '快照不完整 · 不可切换'}
        {account.userId ? ` · ${account.userId}` : ''}
      </div>
      {renderCredits(credits, creditsError)}
      <div style={{ ...slotStatusStyle, color: githubColor, fontSize: 12 }}>{githubLine}</div>
      <SnapshotBadges account={account} />
      {account.warnings.length ? (
        <div style={warningStyle}>{account.warnings.join('；')}</div>
      ) : null}
      <div style={{ marginTop: 'auto', display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        <button
          type="button"
          style={account.validForSwitch ? primaryButtonStyle : buttonStyle}
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
          style={buttonStyle}
          disabled={refreshingCredits || switching}
          onClick={onRefreshCredits}
          title="仅查询积分，绝不签到"
        >
          {refreshingCredits ? '查询中…' : '刷新积分'}
        </button>
        <button
          type="button"
          style={buttonStyle}
          disabled={!githubEnabled || githubSyncing || switching}
          onClick={onSyncGitHub}
          title="把最新凭证同步到 GitHub Secrets（绝不签到）"
        >
          {githubSyncing ? '同步中…' : '同步 GitHub'}
        </button>
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
    <section style={{ ...statusStyle, borderColor: '#fecaca', color: '#b91c1c' }}>
      <span style={statusDotErrorStyle} />
      <div style={{ flex: 1 }}>{error}</div>
      <button type="button" style={buttonStyle} onClick={clearError}>
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
  const loadAccounts = useWorkCnStore((s) => s.loadAccounts);
  const lastImportWarning = useWorkCnStore((s) => s.lastImportWarning);
  const switchingId = useWorkCnStore((s) => s.switchingId);
  const switchTo = useWorkCnStore((s) => s.switchTo);
  const creditsById = useWorkCnStore((s) => s.creditsById);
  const creditsErrorById = useWorkCnStore((s) => s.creditsErrorById);
  const refreshingCreditsId = useWorkCnStore((s) => s.refreshingCreditsId);
  const refreshCredits = useWorkCnStore((s) => s.refreshCredits);
  const githubConfig = useWorkCnStore((s) => s.githubConfig);
  const githubSyncResultById = useWorkCnStore((s) => s.githubSyncResultById);
  const githubSyncingById = useWorkCnStore((s) => s.githubSyncingById);
  const loadGitHubConfig = useWorkCnStore((s) => s.loadGitHubConfig);
  const syncGitHub = useWorkCnStore((s) => s.syncGitHub);
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
    loadAccounts();
    void loadGitHubConfig();
    return () => {
      cancelled = true;
    };
  }, [loadAccounts, loadGitHubConfig]);

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
  const emptySlots = Math.max(0, ACCOUNT_SLOT_COUNT - accounts.length);

  return (
    <div style={pageStyle}>
      <header style={headerStyle}>
        <h1 style={titleStyle}>TRAE Work CN 账号切换器</h1>
        <div style={headerActionsStyle}>
          <button
            type="button"
            style={primaryButtonStyle}
            onClick={() => setDialogOpen(true)}
            disabled={!installation?.installed}
            title={installation?.installed ? '' : '请先安装并登录 TRAE Work CN'}
          >
            导入当前账号
          </button>
          <button
            type="button"
            style={buttonStyle}
            onClick={() => setSettingsOpen(true)}
          >
            设置
          </button>
          <button type="button" style={buttonStyle}>
            日志
          </button>
        </div>
      </header>

      <section style={statusStyle}>
        <span style={status.dot} />
        <div>
          <div>{status.line}</div>
          {status.detail ? (
            <div style={{ fontSize: 12, color: '#8a909c', marginTop: 4, wordBreak: 'break-all' }}>
              {status.detail.split('\n').map((line, index) => (
                <div key={index}>{line}</div>
              ))}
            </div>
          ) : null}
        </div>
      </section>

      {lastImportWarning ? (
        <section style={{ ...statusStyle, borderColor: '#fde68a', color: '#b45309' }}>
          {lastImportWarning}
        </section>
      ) : null}

      <WorkCnStatusBanner />

      <StoreErrorBanner />

      <section>
        <h2 style={sectionTitleStyle}>
          账号槽位（{accounts.length}/{ACCOUNT_SLOT_COUNT}）
          {storeLoading ? ' · 加载中…' : ''}
        </h2>
        <div style={slotsStyle}>
          {accounts.map((account) => (
            <AccountCard
              key={account.id}
              account={account}
              credits={creditsById[account.id] ?? null}
              creditsError={creditsErrorById[account.id] ?? null}
              switching={switchingId === account.id}
              refreshingCredits={refreshingCreditsId === account.id}
              githubEnabled={githubConfig.enabled}
              githubSync={githubSyncResultById[account.id] ?? null}
              githubSyncing={githubSyncingById[account.id] ?? false}
              onSwitch={() => void switchTo(account.id)}
              onRefreshCredits={() => void refreshCredits(account.id)}
              onSyncGitHub={() => void syncGitHub(account.id)}
            />
          ))}
          {Array.from({ length: emptySlots }, (_, index) => (
            <div key={`empty-${index}`} style={slotStyle}>
              <div style={slotTitleStyle}>空槽位</div>
              <div style={slotStatusStyle}>可导入</div>
            </div>
          ))}
        </div>
      </section>

      <WorkCnAddAccountDialog open={dialogOpen} onClose={() => setDialogOpen(false)} />
      <WorkCnSettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
}
