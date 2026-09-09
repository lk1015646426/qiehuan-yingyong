// 切换应用的 TRAE Work CN 账号页面
//
// 样式主要走 work-cn.css（基于 base.css 设计系统 token，自动适配暗色主题），
// 云端签到相关复用 checkin.css 的 ck-* 体系：
// - 快照完整度折叠为一行摘要（悬浮 title 展示缺失项明细）
// - 积分区带 已用/总量 进度条
// - 当前活跃账号（后台会话监测 accountId）高亮描边 + 角标
// - 云端签到：可折叠任务区（运行记录 + 触发验证）已抽为共享组件
//   CloudCheckinPanel，WorkBuddy / 智谱页复用同一签到仓库的面板

import { useEffect, useState } from 'react';
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
import { GhSetupDialog } from '../components/work-cn/GhSetupDialog';
import type { WorkCnInstallation, WorkCnAccountView, WorkCnCreditsSummary, WorkCnGitHubSyncResult, WorkCnSessionWatchStatus } from '../types/workCn';
import type { RefreshFlowState } from '../types/checkin';
import { compactUid, formatTokenExpiryDate, tokenDaysLabel, tokenDaysLeft } from '../utils/accountCardPresentation';

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

// —— 云端签到（自独立面板合并）—————————————————————————
// token 剩余天数预警（tokenDaysLeft/tokenDaysLabel 为三页共享实现，
// 阈值与 auto 刷新逻辑对齐：客户端剩 1/3 寿命时刷新）。

// 切号各阶段文案（stage 与后端 WORK_CN_SWITCH_STAGE_* 常量一致）。

// 切号各阶段文案（stage 与后端 WORK_CN_SWITCH_STAGE_* 常量一致）。
// 切换全程最长约 50 秒（关闭 20s + 验证 30s），无阶段提示时用户只能干等。
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
  switchStage,
  deleting,
  refreshingCredits,
  githubEnabled,
  githubSync,
  active,
  slotCheckinEnabled,
  onSwitch,
  onRefreshCredits,
  onDelete,
  onToggleCheckin,
}: {
  account: WorkCnAccountView;
  credits: WorkCnCreditsSummary | null;
  creditsError: string | null;
  switching: boolean;
  switchStage: string | null;
  deleting: boolean;
  refreshingCredits: boolean;
  githubEnabled: boolean;
  githubSync: WorkCnGitHubSyncResult | null;
  active: boolean;
  /** 绑定槽位的自动签到开关；null 表示该账号未绑定槽位（无云端签到）。 */
  slotCheckinEnabled: boolean | null;
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
  const canSwitch = account.validForSwitch && !switching;
  const { tone: tokenTone, days: tokenDays } = tokenDaysLeft(account.tokenExpiresAt);
  const flow = refreshFlow?.accountId === account.id ? refreshFlow : null;
  const flowRunning = !!flow && flow.phase !== 'done' && flow.phase !== 'failed';
  const localChecking = localCheckinState?.loading ?? false;
  const localResult = localCheckinState?.result ?? null;
  const tokenLabel = tokenDaysLabel(tokenDays);

  // 指标行数值（照抄 WorkBuddy 卡片的 wb-status-line 结构）。
  const creditsLabel = credits?.unlimited
    ? '无限'
    : credits?.total == null
      ? '暂无数据'
      : formatCreditsValue(credits.remaining ?? 0);

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
      {active ? <span className="wc-slot-active-badge">当前账号</span> : null}
      <div className="wc-slot-head account-card__head">
        <div className="wb-account-title">
          <div className="wc-slot-title" title={title}>{title}</div>
        </div>
        <SnapshotSummary account={account} />
      </div>
      <div className="wc-slot-sub account-card__identity" title={account.userId ?? undefined}>
        {account.email ?? '未提供邮箱'} · UID {compactUid(account.userId)}
      </div>
      <div className="account-card__metrics">
        <div className="wc-slot-sub">令牌到期 {formatTokenExpiryDate(account.tokenExpiresAt)}</div>
      </div>
      <div className="wb-status-block">
        <div className="wb-status-line">
          <span title="仅查询，绝不签到">剩余积分 <strong>{creditsLabel}</strong></span>
          <span title={tokenDays != null && tokenDays <= 0 ? '云端签到将失败，请刷新凭证' : undefined}>
            Token 剩余 <strong className={`wc-token-strong wc-token-strong--${tokenTone}`}>{tokenLabel}</strong>
          </span>
        </div>
        {switching ? (
          <div className="wc-slot-sub">{SWITCH_STAGE_LABELS[switchStage ?? ''] ?? '正在准备'}…</div>
        ) : null}
        {creditsError ? <div className="wb-status-error">{creditsError}</div> : null}
      </div>
      <div className="account-card__status account-card__status--toggle">
        {slotCheckinEnabled != null ? (
          <label
            className="wb-checkin-toggle"
            title="关闭后：云端 Actions 不再为该账号签到，该槽位的 GitHub Secrets 会被删除；重新开启后立即恢复同步"
          >
            <input
              type="checkbox"
              checked={slotCheckinEnabled}
              disabled={switching || deleting}
              onChange={(event) => onToggleCheckin(event.target.checked)}
            />
            <span>{slotCheckinEnabled ? '云端自动签到已开启' : '云端自动签到已关闭'}</span>
          </label>
        ) : null}
        <div
          className={`wc-github-line${githubTone}`}
          title={[githubLine, ...account.warnings].join('；')}
        >
          {githubLine}{account.warnings.length ? ` · ${account.warnings.join('；')}` : ''}
        </div>
        {flow ? <FlowLine flow={flow} accountLabel={title} /> : null}
        {localResult ? (
          <div
            className={localResult.ok ? 'ck-flow-line ck-flow-line--done' : 'ck-flow-line ck-flow-line--failed'}
            title={`诊断详情：stage=${localResult.stage}${localResult.businessCode == null ? '' : ` code=${localResult.businessCode}`}${localResult.httpStatus == null ? '' : ` HTTP=${localResult.httpStatus}`}`}
          >
            本地签到：{localCheckinOutcomeLabel(localResult)} · {localResult.message}
          </div>
        ) : null}
      </div>
      {editing ? <div className="wb-edit-name">
        <input className="wc-input" value={name} maxLength={80} onChange={(event) => setName(event.target.value)} />
        <button type="button" className="wc-btn" onClick={() => void saveName()}>保存</button>
        <button type="button" className="wc-btn" onClick={() => { setName(title); setEditing(false); }}>取消</button>
      </div> : null}
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
        <button
          type="button"
          className={tokenTone === 'ok' || tokenTone === 'unknown' ? 'wc-btn' : 'wc-btn wc-btn-primary'}
          disabled={flowRunning || switching}
          onClick={() => void startRefreshFlow(account.id)}
          title="切换到该账号并验证官方设备身份；有效 Token 直接同步，临近过期时等待客户端轮换"
        >
          {flowRunning ? '刷新中…' : '刷新凭证'}
        </button>
        <button
          type="button"
          className="wc-btn"
          disabled={localChecking || flowRunning}
          onClick={() => void localCheckin(account.id)}
          title="从本机直接调用 TRAE 签到 API（诊断/补签）：与云端 Actions 形成对照，用于定位『操作太过频繁』的来源"
        >
          {localChecking ? '签到中…' : '本地签到'}
        </button>
        <button
          type="button"
          className="wc-btn"
          disabled={switching}
          onClick={() => setEditing((value) => !value)}
          title="修改账号备注（显示名），不影响 GitHub 槽位绑定"
        >
          备注
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

  return (
    <div className="wc-page work-cn-page">
      <header className="wc-header">
        <h1 className="wc-title">
          <img className="wc-title-icon" src={traeCnIcon} alt="TRAE" />
          切换应用 · TRAE Work CN
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
            onClick={handleSyncAll}
            disabled={syncingAllGithub}
            title="把所有已绑定槽位账号的最新凭证（按账号槽位名命名的 *_TOKEN / *_DEVICE_ID）一键同步到 GitHub Secrets，供 daily-checkin 工作流签到使用；条件未就绪时点击可查看引导"
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
        <div className="wc-slots wb-slots">
          {accounts.map((account) => (
            <AccountCard
              key={account.id}
              account={account}
              credits={creditsById[account.id] ?? null}
              creditsError={creditsErrorById[account.id] ?? null}
              switching={switchingId === account.id}
              switchStage={switchingId === account.id ? switchStage : null}
              deleting={deletingId === account.id}
              refreshingCredits={!!refreshingCreditsIds[account.id]}
              githubEnabled={githubConfig.enabled}
              githubSync={githubSyncResultById[account.id] ?? null}
              active={account.id === activeAccountId}
              slotCheckinEnabled={
                githubConfig.slots.find((slot) => slot.accountId === account.id)?.checkinEnabled ?? null
              }
              onSwitch={() => void switchTo(account.id)}
              onRefreshCredits={() => void refreshCredits(account.id, true)}
              onDelete={() => void deleteAccount(account.id)}
              onToggleCheckin={(enabled) => void setSlotCheckin(account.id, enabled)}
            />
          ))}
          <div className="wc-slot wc-slot--empty">
            <span className="wc-slot-empty-icon">＋</span>
            <span className="wc-slot-empty-hint">新增槽位 · 点击右上「导入当前账号」按顺序追加</span>
          </div>
        </div>
      </section>

      {/* 云端签到任务（共享组件）：低频诊断区，默认收起。
          日常签到由 GitHub Actions 北京时间 04:00 自动执行。 */}
      <CloudCheckinPanel canTrigger={ghReady && githubConfig.enabled} />

      <WorkCnAddAccountDialog open={dialogOpen} onClose={() => setDialogOpen(false)} />
      <WorkCnSettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      <GhSetupDialog open={setupOpen} onClose={() => setSetupOpen(false)} />
    </div>
  );
}
