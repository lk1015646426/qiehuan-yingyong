// 切换应用的 TRAE Work CN 账号页面
//
// 样式主要走 work-cn.css（基于 base.css 设计系统 token，自动适配暗色主题），
// 云端签到相关复用 checkin.css 的 ck-* 体系：
// - 快照完整度折叠为一行摘要（悬浮 title 展示缺失项明细）
// - 积分区带 已用/总量 进度条
// - 当前活跃账号（后台会话监测 accountId）高亮描边 + 角标
// - 云端签到：可折叠任务区（运行记录 + 触发验证）已抽为共享组件
//   CloudCheckinPanel，WorkBuddy 页复用同一签到仓库的面板

import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import traeCnIcon from '../assets/icons/trae-cn.png';
import { getWorkCnInstallation, WORK_CN_SESSION_WATCH_EVENT } from '../services/workCnService';
import { useWorkCnStore } from '../stores/useWorkCnStore';
import { useCheckinStore } from '../stores/useCheckinStore';
import { localCheckinOutcomeLabel } from '../utils/checkinPresentation';
import { CloudCheckinPanel } from '../components/checkin/CloudCheckinPanel';
import { WorkCnAddAccountDialog } from '../components/work-cn/WorkCnAddAccountDialog';
import { WorkCnSettingsDialog } from '../components/work-cn/WorkCnSettingsDialog';
import { WorkCnStatusBanner } from '../components/work-cn/WorkCnStatusBanner';
import { TraePoolPanel } from '../components/work-cn/TraePoolPanel';
import {
  AccountManagementLayout, AccountList, AccountCardFrame, AccountCardActions, AccountStatusBanner,
  LocalCheckinFeedback, type AccountCheckinResult,
} from '../components/AccountManagementLayout';
import { GhSetupDialog } from '../components/work-cn/GhSetupDialog';
import type { WorkCnInstallation, WorkCnAccountView, WorkCnCreditsSummary, WorkCnGitHubSyncResult, WorkCnSessionWatchStatus } from '../types/workCn';
import type { LocalCheckinResult, RefreshFlowState } from '../types/checkin';
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
    return { tone: '', line: '客户端检测：正在检测 TRAE 安装…' };
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
      line: '未检测到 TRAE，请先打开官方客户端后重新检测，或在设置中选择 EXE 路径',
      detail,
    };
  }
  const versionText = installation.version ? ` ${installation.version}` : '';
  const dataDirText = installation.userDataDir ?? '';
  const legacyNote = installation.legacyPath ? '（兼容旧数据目录）' : '';
  return {
    tone: 'wc-status--ok',
    line: `已检测到 ${installation.displayName ?? 'TRAE'}${versionText}`,
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

const SWITCH_STAGE_LABELS: Record<string, string> = {
  validating: '正在校验账号快照',
  closing: '正在关闭客户端（最长 20 秒）',
  injecting: '正在注入账号凭证',
  binding: '正在绑定默认实例',
  launching: '正在启动客户端',
  verifying: '正在验证切换结果（最长 30 秒）',
  syncing: '正在同步会话与 GitHub',
};

// 刷新凭证流状态行：完成后提供「切回原账号」（不自动切回）。
function FlowLine({ flow, accountLabel }: { flow: RefreshFlowState; accountLabel: string }) {
  const switchTo = useWorkCnStore((s) => s.switchTo);
  const resetRefreshFlow = useCheckinStore((s) => s.resetRefreshFlow);
  const running = flow.phase !== 'done' && flow.phase !== 'failed';
  const tone =
    flow.phase === 'failed'
      ? 'ck-flow-line ck-flow-line--failed'
      : flow.phase === 'done'
        ? 'ck-flow-line ck-flow-line--done'
        : 'ck-flow-line';

  return (
    <div className={tone}>
      <span>{flow.message}</span>
      {flow.phase === 'done' && flow.previousAccountId ? (
        <span className="ck-flow-actions">
          <button
            type="button"
            className="wc-btn"
            onClick={() => {
              void switchTo(flow.previousAccountId!);
              resetRefreshFlow();
            }}
          >
            切回原账号
          </button>
        </span>
      ) : null}
      {!running ? (
        <button
          type="button"
          className="wc-btn"
          onClick={resetRefreshFlow}
          title={`关闭「${accountLabel}」的刷新提示`}
        >
          知道了
        </button>
      ) : null}
    </div>
  );
}

function AccountCard({
  account,
  credits,
  creditsError,
  switching,
  switchInProgress,
  switchStage,
  deleting,
  refreshingCredits,
  githubEnabled,
  githubSync,
  active,
  slotCheckinEnabled,
  localCheckinBusy,
  onSwitch,
  onRefreshCredits,
  onDelete,
  onToggleCheckin,
}: {
  account: WorkCnAccountView;
  credits: WorkCnCreditsSummary | null;
  creditsError: string | null;
  switching: boolean;
  switchInProgress: boolean;
  switchStage: string | null;
  deleting: boolean;
  refreshingCredits: boolean;
  githubEnabled: boolean;
  githubSync: WorkCnGitHubSyncResult | null;
  active: boolean;
  /** 绑定槽位的自动签到开关；null 表示该账号未绑定槽位（无云端签到）。 */
  slotCheckinEnabled: boolean | null;
  localCheckinBusy: boolean;
  onSwitch: () => void;
  onRefreshCredits: () => void;
  onDelete: () => void;
  onToggleCheckin: (enabled: boolean) => void;
}) {
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  // 云端签到相关状态（自独立面板合并）：
  const refreshFlow = useCheckinStore((s) => s.refreshFlow);
  const startRefreshFlow = useCheckinStore((s) => s.startRefreshFlow);
  const localCheckin = useCheckinStore((s) => s.localCheckin);
  const localCheckinState = useCheckinStore((s) => s.localCheckinById[account.id]);
  const title = account.tags?.length
    ? account.tags[0]
    : account.nickname ?? account.email ?? account.userId ?? account.id;
  // 备注（显示名）编辑：保存走 store.renameAccount，成功后 tags[0] 即新标题。
  const renameAccount = useWorkCnStore((s) => s.renameAccount);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(title);
  const saveName = async () => {
    const value = name.trim();
    if (!value) return;
    await renameAccount(account.id, value);
    setEditing(false);
  };
  const canSwitch = account.validForSwitch && !switchInProgress && !localCheckinBusy;
  const flow = refreshFlow?.accountId === account.id ? refreshFlow : null;
  const restoring = !!refreshFlow && refreshFlow.phase !== 'done' && refreshFlow.phase !== 'failed';
  const localChecking = localCheckinState?.loading ?? false;
  const localResult = localCheckinState?.result ?? null;
  const [manualResult, setManualResult] = useState<LocalCheckinResult | null>(null);
  const runAccountCheckin = async () => {
    await localCheckin(account.id);
    setManualResult(useCheckinStore.getState().localCheckinById[account.id]?.result ?? null);
  };
  const creditsLabel = refreshingCredits && !credits ? '查询中…' : credits?.unlimited
    ? '无限'
    : credits?.remaining?.toLocaleString('zh-CN', { maximumFractionDigits: 2 }) ?? '未查询到';
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

  const checkinFeedback = localResult && (
    <div className={localResult.ok ? 'uac-checkin-hint' : 'wb-status-error'} role={localResult.ok ? 'status' : 'alert'}
      title={`诊断详情：stage=${localResult.stage}${localResult.businessCode == null ? '' : ` code=${localResult.businessCode}`}${localResult.httpStatus == null ? '' : ` HTTP=${localResult.httpStatus}`}`}>
      本地签到：{localCheckinOutcomeLabel(localResult)} · {localResult.message}
    </div>
  );
  const inlineCheckinFeedback = localResult === manualResult ? checkinFeedback : null;

  return (
    <AccountCardFrame title={title} active={active}
      contact={account.email} uid={account.userId} tokenExpiresAt={account.tokenExpiresAt}
      credits={<>{creditsLabel}{refreshingCredits && credits && '（刷新中）'}</>}
      creditsTitle="查询 TRAE 账户剩余积分"
      alert={switching || creditsError || flow || account.warnings.length || githubSync?.error || !account.validForSwitch ? <>
        {!account.validForSwitch ? <div className="wc-warning">登录快照不完整，请重新导入账号。</div> : null}
        {switching ? <div className="wc-slot-sub">{SWITCH_STAGE_LABELS[switchStage ?? ''] ?? '正在准备'}…</div> : null}
        {creditsError ? <div className="wb-status-error" role="alert">{creditsError}</div> : null}
        {account.warnings.length ? <div className="wc-warning">{account.warnings.join('；')}</div> : null}
        {githubSync?.error ? <div className="wb-status-error" role="alert">{githubLine}</div> : null}
        {flow ? <FlowLine flow={flow} accountLabel={title} /> : null}
      </> : undefined}
      details={<>
        <SnapshotSummary account={account} />
        <div className="account-card__status account-card__status--toggle">
          {slotCheckinEnabled != null ? <label className="wb-checkin-toggle"
            title="关闭后，移除该槽位的 GitHub Secrets 并停止自动签到">
            <input type="checkbox" checked={slotCheckinEnabled} disabled={switching || deleting}
              onChange={(event) => onToggleCheckin(event.target.checked)} />
            <span>{slotCheckinEnabled ? '云端自动签到已开启' : '云端自动签到已关闭'}</span>
          </label> : null}
          <div className={`wc-github-line${githubTone}`} title={[githubLine, ...account.warnings].join('；')}>
            {githubLine}{account.warnings.length ? ` · ${account.warnings.join('；')}` : ''}
          </div>
        </div>
        {inlineCheckinFeedback ? null : checkinFeedback}
      </>}
      recovery={{
        onClick: () => void startRefreshFlow(account.id), disabled: restoring || switchInProgress || localCheckinBusy,
        label: flow && restoring ? '刷新中…' : undefined,
        title: '打开该账号的 TRAE 客户端并恢复凭证，完成后同步 GitHub',
      }}
      cloudCheckin={{ title: '请在下方的云端签到任务中运行' }}>
      {inlineCheckinFeedback}
      {editing ? <div className="wb-edit-name">
        <input className="wc-input" aria-label="账号备注" value={name} maxLength={80} onChange={(event) => setName(event.target.value)} />
        <button type="button" className="wc-btn" onClick={() => void saveName()}>保存</button>
        <button type="button" className="wc-btn" onClick={() => { setName(title); setEditing(false); }}>取消</button>
      </div> : null}
      <AccountCardActions
        switchAccount={{
          onClick: onSwitch, disabled: !canSwitch, label: switching ? '切换中…' : undefined,
          title: account.validForSwitch ? '切换到该账号并打开 TRAE' : '快照不完整，请重新登录并导入',
        }}
        checkin={{
          onClick: () => void runAccountCheckin(),
          disabled: localCheckinBusy || restoring || switchInProgress || deleting,
          label: localChecking ? '签到中…' : undefined,
          title: '使用 TRAE 已保存的登录状态发起本地签到',
        }}
        refresh={{ onClick: onRefreshCredits, disabled: refreshingCredits || switching, label: refreshingCredits ? '刷新中…' : undefined }}
        rename={{ onClick: () => setEditing((value) => !value), disabled: switching }}
        remove={{
          onClick: confirmingDelete ? onDelete : () => setConfirmingDelete(true),
          disabled: switchInProgress || deleting || localCheckinBusy, label: deleting ? '删除中…' : undefined,
        }}
        cancelRemove={confirmingDelete ? { onClick: () => setConfirmingDelete(false), disabled: deleting } : undefined}
      />
    </AccountCardFrame>
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

const TRAE_PATH_FIELDS = [
  { key: 'trae_cn_app_path', label: 'TRAE 国行 EXE 路径' },
  { key: 'trae_solo_cn_app_path', label: 'TRAE Work / SOLO CN EXE 路径' },
] as const;

type TraePaths = Record<(typeof TRAE_PATH_FIELDS)[number]['key'], string>;

function TraePathSettingsDialog({ onClose, onSaved }: { onClose: () => void; onSaved: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [paths, setPaths] = useState<TraePaths | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;
    if (!dialog.current?.open) dialog.current?.showModal();
    void invoke<TraePaths>('get_general_config').then(config => {
      if (!cancelled) setPaths({
        trae_cn_app_path: config.trae_cn_app_path ?? '',
        trae_solo_cn_app_path: config.trae_solo_cn_app_path ?? '',
      });
    }).catch((cause: unknown) => {
      if (!cancelled) setError(String(cause));
    });
    return () => { cancelled = true; };
  }, []);

  const save = async () => {
    if (!paths || saving) return;
    setSaving(true);
    setError('');
    try {
      await invoke('patch_general_config', { updates: {
        trae_cn_app_path: paths.trae_cn_app_path.trim(),
        trae_solo_cn_app_path: paths.trae_solo_cn_app_path.trim(),
      } });
      onSaved();
      dialog.current?.close();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  };

  return <dialog ref={dialog} className="wc-dialog wb-settings-dialog uac-path-dialog"
    aria-labelledby="trae-path-dialog-title" onClose={onClose} onCancel={event => { if (saving) event.preventDefault(); }}>
    <h3 className="wc-dialog-title" id="trae-path-dialog-title">TRAE 路径设置</h3>
    <p className="wc-dialog-note">按已安装的版本填写。留空时自动检测。</p>
    {paths && TRAE_PATH_FIELDS.map(({ key, label }) => <label key={key} className="wc-row-label">
      {label}
      <input className="wc-input" value={paths[key]} disabled={saving}
        onChange={event => setPaths({ ...paths, [key]: event.target.value })} />
    </label>)}
    {!paths && !error && <p className="wc-dialog-note" role="status">正在读取路径设置…</p>}
    {error && <p className="wc-dialog-error" role="alert">{error}</p>}
    <div className="wc-dialog-actions">
      <button type="button" className="wc-btn" disabled={saving} onClick={() => dialog.current?.close()}>取消</button>
      <button type="button" className="wc-btn wc-btn-primary" disabled={!paths || saving} onClick={() => void save()}>
        {saving ? '保存中…' : '保存路径'}
      </button>
    </div>
  </dialog>;
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
  const switchStage = useWorkCnStore((s) => s.switchStage);
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
  const setSlotCheckin = useWorkCnStore((s) => s.setSlotCheckin);
  const loadGitHubConfig = useWorkCnStore((s) => s.loadGitHubConfig);
  const syncGitHubAll = useWorkCnStore((s) => s.syncGitHubAll);
  const syncingAllGithub = useWorkCnStore((s) => s.syncingAllGithub);
  const syncAllProgress = useWorkCnStore((s) => s.syncAllProgress);
  const githubCliStatus = useWorkCnStore((s) => s.githubCliStatus);
  const sessionWatchStatus = useWorkCnStore((s) => s.sessionWatchStatus);
  const loadSessionWatchStatus = useWorkCnStore((s) => s.loadSessionWatchStatus);
  const applySessionWatchStatus = useWorkCnStore((s) => s.applySessionWatchStatus);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // 云端签到面板（共享组件，自独立面板合并后再次抽离）：
  // 运行记录 + 手动触发验证。日常签到由 GitHub Actions 北京时间 04:00 自动执行。
  // 触发条件沿用本页逻辑：gh CLI 可用且 GitHub 同步已启用。
  const [setupOpen, setSetupOpen] = useState(false);
  const [pathsOpen, setPathsOpen] = useState(false);
  const [detectionVersion, setDetectionVersion] = useState(0);
  const [localCheckingIn, setLocalCheckingIn] = useState(false);
  const [localCheckinResults, setLocalCheckinResults] = useState<AccountCheckinResult[] | null>(null);
  const accountCheckingIn = useCheckinStore((state) => Object.values(state.localCheckinById).some(result => result.loading));
  const restoring = useCheckinStore((state) => state.refreshFlow != null
    && state.refreshFlow.phase !== 'done' && state.refreshFlow.phase !== 'failed');
  const localCheckinBusy = localCheckingIn || accountCheckingIn;
  const detect = () => setDetectionVersion(value => value + 1);

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
    return () => {
      cancelled = true;
    };
  }, [detectionVersion]);

  useEffect(() => {
    void ensureAccountsLoaded();
    void loadGitHubConfig();
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

  const ghReady = !!githubCliStatus?.available && !!githubCliStatus.authed;
  const configReady =
    githubConfig.enabled &&
    /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(githubConfig.repository) &&
    githubConfig.slots.length > 0;
  const uploadReady = ghReady && configReady;

  // 上传条件不全时点击同步 → 弹出引导而不是直接报错。
  const handleSyncAll = () => {
    if (!uploadReady) {
      setSetupOpen(true);
      return;
    }
    void syncGitHubAll();
  };

  const status = renderInstallationStatus(installation, loading, error);

  // 当前活跃账号：优先取后台会话监测到的 accountId；未监测时回退到最近使用。
  const activeAccountId =
    sessionWatchStatus?.accountId ??
    (accounts.some((a) => a.lastUsed > 0)
      ? accounts.reduce((latest, a) => (a.lastUsed > latest.lastUsed ? a : latest)).id
      : null);

  const runLocalCheckin = async () => {
    if (localCheckinBusy || Object.values(useCheckinStore.getState().localCheckinById).some(result => result.loading)) return;
    setLocalCheckingIn(true);
    setLocalCheckinResults(null);
    const results: AccountCheckinResult[] = [];
    try {
      for (const account of accounts) {
        await useCheckinStore.getState().localCheckin(account.id);
        const result = useCheckinStore.getState().localCheckinById[account.id]?.result;
        results.push({
          accountId: account.id,
          displayName: account.tags?.[0] ?? account.nickname ?? account.email ?? account.userId ?? account.id,
          outcome: result?.ok
            ? { Ok: result.alreadyCheckedIn ? 'alreadySigned' : 'signed' }
            : { Err: result?.message || '未收到签到结果' },
        });
        if (result?.ok) void refreshCredits(account.id, true);
      }
    } finally {
      setLocalCheckinResults(results);
      setLocalCheckingIn(false);
    }
  };

  return (
    <AccountManagementLayout app="TRAE" icon={traeCnIcon} actions={{
      importAccount: {
        onClick: () => setDialogOpen(true), disabled: !installation?.installed,
        title: installation?.installed ? undefined : '请先安装并登录 TRAE',
      },
      sync: {
        onClick: handleSyncAll, disabled: syncingAllGithub,
        label: syncingAllGithub && syncAllProgress ? `同步中 ${syncAllProgress.done}/${syncAllProgress.total}…` : undefined,
        title: '将已绑定槽位的凭证同步到 GitHub Secrets',
      },
      refresh: {
        onClick: () => void Promise.all(accounts.map(account => refreshCredits(account.id, true))),
        disabled: !accounts.length || Object.values(refreshingCreditsIds).some(Boolean),
      },
      checkin: {
        onClick: () => void runLocalCheckin(),
        disabled: !accounts.length || storeLoading || localCheckinBusy || restoring || switchingId != null || deletingId != null,
        label: localCheckingIn ? '签到中…' : undefined, title: '依次为已保存的 TRAE 账号发起本地签到',
      },
      settings: { onClick: () => setPathsOpen(true) },
      detect: { onClick: detect, disabled: loading },
    }}>
      <AccountStatusBanner tone={status.tone}
        detail={status.detail?.split('\n').map((line, index) => <div key={index}>{line}</div>)}>
        {status.line}
      </AccountStatusBanner>

      {lastImportWarning ? (
        <section className="wc-status wc-status--warn">{lastImportWarning}</section>
      ) : null}

      <WorkCnStatusBanner />

      <StoreErrorBanner />

      <LocalCheckinFeedback running={localCheckingIn} results={localCheckinResults} />

      <TraePoolPanel accountId={activeAccountId} />
      <AccountList count={accounts.length} loading={storeLoading} emptyHint="先登录官方 TRAE 客户端，再导入当前账号">
          {accounts.map((account) => (
            <AccountCard
              key={account.id}
              account={account}
              credits={creditsById[account.id] ?? null}
              creditsError={creditsErrorById[account.id] ?? null}
              switching={switchingId === account.id}
              switchInProgress={switchingId !== null}
              switchStage={switchingId === account.id ? switchStage : null}
              deleting={deletingId === account.id}
              refreshingCredits={!!refreshingCreditsIds[account.id]}
              githubEnabled={githubConfig.enabled}
              githubSync={githubSyncResultById[account.id] ?? null}
              active={account.id === activeAccountId}
              slotCheckinEnabled={
                githubConfig.slots.find((slot) => slot.accountId === account.id)?.checkinEnabled ?? null
              }
              localCheckinBusy={localCheckinBusy}
              onSwitch={() => void switchTo(account.id)}
              onRefreshCredits={() => void refreshCredits(account.id, true)}
              onDelete={() => void deleteAccount(account.id)}
              onToggleCheckin={(enabled) => void setSlotCheckin(account.id, enabled)}
            />
          ))}
      </AccountList>

      <div className="wc-slot-actions uac-platform-actions" role="group" aria-label="TRAE 更多操作">
        <button type="button" className="wc-btn" onClick={() => setSettingsOpen(true)}>GitHub 设置</button>
        <button type="button" className="wc-btn" onClick={() => void openLogs()} title="打开日志目录">日志</button>
      </div>
      <CloudCheckinPanel canTrigger={ghReady && githubConfig.enabled} />

      <WorkCnAddAccountDialog open={dialogOpen} onClose={() => setDialogOpen(false)} />
      <WorkCnSettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      {pathsOpen && <TraePathSettingsDialog onClose={() => setPathsOpen(false)} onSaved={detect} />}
      <GhSetupDialog open={setupOpen} onClose={() => setSetupOpen(false)} />
    </AccountManagementLayout>
  );
}
