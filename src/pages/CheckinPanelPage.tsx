// 云端签到管理面板（阶段 8）。
//
// 面板职责：让"云端每天自动签到"可观测、可维护——
// - token 倒计时预警 + 半自动刷新流（切换→等刷新→同步 GitHub）
// - 一键同步全部凭证
// - 立即验证签到（触发 workflow）+ 最近运行状态
// - 本地签到（诊断/补签）：云端被拒“操作太过频繁”时，从本机直接调用签到 API 做对照定位
// 日常签到由云端 Actions 凌晨 4 点自动执行，本面板不重复该职责；
// 例外：本地签到是有意打破「本地绝不签到」原则的诊断入口（用户 2026-08-16 决策）。

import { useEffect, useState } from 'react';
import { CloudUpload } from 'lucide-react';
import traeCnIcon from '../assets/icons/trae-cn.png';
import workBuddyIcon from '../assets/icons/workbuddy.png';
import { useCheckinStore } from '../stores/useCheckinStore';
import { localCheckinOutcomeLabel } from '../utils/checkinPresentation';
import { useWorkCnStore } from '../stores/useWorkCnStore';
import { openExternalUrl } from '../services/workCnService';
import { GhSetupDialog } from '../components/work-cn/GhSetupDialog';
import { accountSecretStem } from '../utils/accountNaming';
import type { WorkCnAccountView } from '../types/workCn';
import type { CheckinWorkflowRun, RefreshFlowState } from '../types/checkin';
import type { CheckinProduct } from '../types/checkin';
import type { WorkBuddyAccountView } from '../types/workbuddy';
import { checkinProductLabel, groupCheckinProducts } from '../utils/checkinProducts';
import { useWorkBuddyStore } from '../stores/useWorkBuddyStore';
import { compactUid, githubSyncPresentation } from '../utils/accountCardPresentation';

// token 剩余天数预警阈值（与 auto 刷新逻辑对齐：客户端剩 1/3 寿命时刷新）。
const WARN_DAYS = 5;
const DANGER_DAYS = 1;

// 引导弹窗自动弹出标记：模块级（应用会话内共享），切页重进不重复弹；
// 用户仍可点顶部 gh 徽标或「同步全部 GitHub」手动打开。
let ghSetupAutoPrompted = false;

type TokenTone = 'ok' | 'warn' | 'danger' | 'unknown';

function tokenDaysLeft(account: WorkCnAccountView): { tone: TokenTone; text: string } {
  if (!account.tokenExpiresAt) {
    return { tone: 'unknown', text: 'token 有效期未知' };
  }
  const msLeft = account.tokenExpiresAt * 1000 - Date.now();
  if (msLeft <= 0) {
    return { tone: 'danger', text: 'token 已过期 · 云端签到将失败' };
  }
  const days = Math.ceil(msLeft / 86_400_000);
  if (days <= DANGER_DAYS) {
    return { tone: 'danger', text: `token 仅剩 ${days} 天 · 立即刷新` };
  }
  if (days <= WARN_DAYS) {
    return { tone: 'warn', text: `token 剩 ${days} 天 · 建议刷新` };
  }
  return { tone: 'ok', text: `token 剩 ${days} 天` };
}

function formatRelative(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) {
    return iso;
  }
  const diff = Date.now() - then;
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 1) {
    return '刚刚';
  }
  if (minutes < 60) {
    return `${minutes} 分钟前`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours} 小时前`;
  }
  const days = Math.floor(hours / 24);
  if (days < 30) {
    return `${days} 天前`;
  }
  return new Date(then).toLocaleDateString('zh-CN');
}

function runTone(run: CheckinWorkflowRun): 'ok' | 'error' | 'running' | 'unknown' {
  if (run.status !== 'completed') {
    return 'running';
  }
  if (run.conclusion === 'success') {
    return 'ok';
  }
  if (run.conclusion === 'failure' || run.conclusion === 'cancelled') {
    return 'error';
  }
  return 'unknown';
}

function latestBanner(
  runs: CheckinWorkflowRun[],
): { tone: string; main: string; sub: string; url?: string } | null {
  const run = runs[0];
  if (!run) {
    return null;
  }
  const when = formatRelative(run.createdAt);
  const tone = runTone(run);
  if (tone === 'ok') {
    return {
      tone: 'ck-banner--ok',
      main: `最近一次共享签到任务成功（${when}）`,
      sub: '该仓库同时承载 TRAE 与 WorkBuddy，请在运行标题中确认产品',
      url: run.url,
    };
  }
  if (tone === 'error') {
    return {
      tone: 'ck-banner--error',
      main: `最近一次共享签到任务失败（${when}）`,
      sub: '请打开 Actions 记录确认是 TRAE 还是 WorkBuddy，再按对应账号卡片处理',
      url: run.url,
    };
  }
  if (tone === 'running') {
    return {
      tone: 'ck-banner--running',
      main: '共享签到任务正在运行中',
      sub: '稍后刷新运行记录，并在运行标题中确认 TRAE 或 WorkBuddy',
      url: run.url,
    };
  }
  return null;
}

function WorkBuddyCheckinCard({ account }: { account: WorkBuddyAccountView }) {
  const checkingInId = useWorkBuddyStore((state) => state.checkingInId);
  const syncing = useWorkBuddyStore((state) => state.syncing);
  const triggerCheckin = useWorkBuddyStore((state) => state.triggerCheckin);
  const syncGitHub = useWorkBuddyStore((state) => state.syncGitHub);
  const accountBusy = checkingInId === account.id;
  const githubSync = githubSyncPresentation(account.lastGithubSyncState, account.lastGithubSyncError);
  return (
    <div className="ck-card account-card account-card--compact ck-card--workbuddy">
      <div className="ck-card-head account-card__head">
        <img className="ck-product-icon" src={workBuddyIcon} alt="WorkBuddy" width={20} height={20} />
        <span className="ck-product-pill ck-product-pill--workbuddy">WorkBuddy</span>
        <span className="ck-card-title" title={account.displayName}>{account.displayName}</span>
      </div>
      <div className="ck-card-sub account-card__identity" title={account.uid}>{account.maskedPhone ?? '未提供手机号'} · UID {compactUid(account.uid)}</div>
      <div className="account-card__metrics">
        <div className="ck-card-sub">令牌到期 {account.tokenExpiresAt ? new Date(account.tokenExpiresAt * 1000).toLocaleString('zh-CN', { hour12: false }) : '未知'}</div>
      </div>
      <div className="account-card__status">
        <div
          className={`ck-flow-line${githubSync.tone === 'failed' ? ' ck-flow-line--failed' : githubSync.tone === 'synced' ? ' ck-flow-line--done' : ''}`}
          title={githubSync.detail ?? githubSync.label}
        >
          {account.checkinEnabled ? '自动签到已开启' : '自动签到未开启'} · {githubSync.label}
        </div>
      </div>
      <div className="ck-card-actions account-card__actions">
        <button type="button" className="wc-btn wc-btn-primary" disabled={accountBusy || !account.checkinEnabled} onClick={() => void triggerCheckin(account.id)} title={account.checkinEnabled ? '只触发该 WorkBuddy 账号的远端签到' : '请先在 WorkBuddy 页面启用自动签到'}>{checkingInId === account.id ? '触发中…' : '触发云端签到'}</button>
        <button type="button" className="wc-btn" disabled={syncing} onClick={() => void syncGitHub()}>{syncing ? '同步中…' : '同步 WorkBuddy'}</button>
      </div>
    </div>
  );
}

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

function CredentialCard({ account }: { account: WorkCnAccountView }) {
  const githubConfig = useWorkCnStore((s) => s.githubConfig);
  const syncGitHub = useWorkCnStore((s) => s.syncGitHub);
  const githubSyncingById = useWorkCnStore((s) => s.githubSyncingById);
  const githubSyncResultById = useWorkCnStore((s) => s.githubSyncResultById);
  const sessionWatchStatus = useWorkCnStore((s) => s.sessionWatchStatus);
  const startRefreshFlow = useCheckinStore((s) => s.startRefreshFlow);
  const refreshFlow = useCheckinStore((s) => s.refreshFlow);
  const localCheckin = useCheckinStore((s) => s.localCheckin);
  const localCheckinState = useCheckinStore((s) => s.localCheckinById[account.id]);

  const slot = githubConfig.slots.find((s) => s.accountId === account.id);
  const slotPill = slot ? `${accountSecretStem(account)}_TOKEN` : '未绑定槽位';
  const title = account.tags?.length
    ? account.tags[0]
    : account.nickname ?? account.email ?? account.userId ?? account.id;
  const { tone, text } = tokenDaysLeft(account);
  const active = sessionWatchStatus?.accountId === account.id;
  const syncing = githubSyncingById[account.id] ?? false;
  const flow = refreshFlow?.accountId === account.id ? refreshFlow : null;
  const flowRunning = !!flow && flow.phase !== 'done' && flow.phase !== 'failed';
  const syncResult = githubSyncResultById[account.id];
  const localChecking = localCheckinState?.loading ?? false;
  const localResult = localCheckinState?.result ?? null;

  return (
    <div className={active ? 'ck-card account-card account-card--compact ck-card--active' : 'ck-card account-card account-card--compact'}>
      <div className="ck-card-head account-card__head">
        <img className="ck-product-icon" src={traeCnIcon} alt="TRAE" width={20} height={20} />
        <span className="ck-product-pill">TRAE</span>
        <span className="ck-card-title" title={title}>{title}</span>
        {active ? <span className="ck-slot-pill">使用中</span> : null}
        <span className={slot ? 'ck-slot-pill' : 'ck-slot-pill ck-slot-pill--none'}>{slotPill}</span>
      </div>
      <div className="ck-card-sub account-card__identity" title={account.userId ?? undefined}>
        {account.email ?? '未提供邮箱'} · UID {compactUid(account.userId)}
      </div>
      <div className="account-card__metrics">
        <div className={`ck-token-pill ck-token-pill--${tone}`}>{text}</div>
      </div>
      <div className="account-card__status">
        {flow ? <FlowLine flow={flow} accountLabel={title} /> : null}
        {syncResult && !flow ? (
        <div
          className={syncResult.synced ? 'ck-flow-line ck-flow-line--done' : 'ck-flow-line ck-flow-line--failed'}
        >
          {syncResult.synced
            ? 'GitHub：已同步'
            : `GitHub：${syncResult.error ?? syncResult.skipReason ?? '未同步'}`}
        </div>
        ) : null}
        {localResult ? (
        <div
          className={localResult.ok ? 'ck-flow-line ck-flow-line--done' : 'ck-flow-line ck-flow-line--failed'}
        >
          本地签到：{localCheckinOutcomeLabel(localResult)}
          {'：'}{localResult.message}
          {' '}({localResult.stage}
          {localResult.businessCode == null ? '' : `, code=${localResult.businessCode}`}
          {localResult.httpStatus == null ? '' : `, HTTP=${localResult.httpStatus}`})
        </div>
        ) : null}
        {!flow && !syncResult && !localResult ? <div className="ck-flow-line">等待云端签到</div> : null}
      </div>
      <div className="ck-card-actions account-card__actions">
        <button
          type="button"
          className={tone === 'ok' ? 'wc-btn' : 'wc-btn wc-btn-primary'}
          disabled={flowRunning || syncing}
          onClick={() => void startRefreshFlow(account.id)}
          title="切换到该账号并验证官方设备身份；有效 Token 直接同步，临近过期时等待客户端轮换"
        >
          {flowRunning ? '刷新中…' : tone === 'ok' ? '刷新凭证' : '刷新凭证'}
        </button>
        <button
          type="button"
          className="wc-btn"
          disabled={syncing || flowRunning || !githubConfig.enabled || !slot}
          onClick={() => void syncGitHub(account.id)}
          title={slot ? '把该账号最新凭证推送到 GitHub Secrets' : '请先在设置中绑定槽位'}
        >
          {syncing ? '同步中…' : '同步'}
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
      </div>
    </div>
  );
}

export function CheckinPanelPage() {
  const accounts = useWorkCnStore((s) => s.accounts);
  const ensureAccountsLoaded = useWorkCnStore((s) => s.ensureAccountsLoaded);
  const githubConfig = useWorkCnStore((s) => s.githubConfig);
  const loadGitHubConfig = useWorkCnStore((s) => s.loadGitHubConfig);
  const githubCliStatus = useWorkCnStore((s) => s.githubCliStatus);
  const syncGitHubAll = useWorkCnStore((s) => s.syncGitHubAll);
  const syncingAllGithub = useWorkCnStore((s) => s.syncingAllGithub);
  const syncAllProgress = useWorkCnStore((s) => s.syncAllProgress);
  const workBuddyAccounts = useWorkBuddyStore((s) => s.accounts);
  const ensureWorkBuddyAccountsLoaded = useWorkBuddyStore((s) => s.ensureAccountsLoaded);
  const ensureWorkBuddyMonitoringStarted = useWorkBuddyStore((s) => s.ensureMonitoringStarted);
  const syncWorkBuddyGitHub = useWorkBuddyStore((s) => s.syncGitHub);
  const workBuddySyncing = useWorkBuddyStore((s) => s.syncing);
  const [productFilter, setProductFilter] = useState<CheckinProduct>('all');

  const runs = useCheckinStore((s) => s.runs);
  const runsLoading = useCheckinStore((s) => s.runsLoading);
  const runsError = useCheckinStore((s) => s.runsError);
  const loadRuns = useCheckinStore((s) => s.loadRuns);
  const triggering = useCheckinStore((s) => s.triggering);
  const triggerMessage = useCheckinStore((s) => s.triggerMessage);
  const triggerRun = useCheckinStore((s) => s.triggerRun);

  // 上传条件引导弹窗：自动检测缺失项并就地补全（安装 gh / 登录 / 配置）。
  const [setupOpen, setSetupOpen] = useState(false);

  const refreshGitHubCliStatus = useWorkCnStore((s) => s.refreshGitHubCliStatus);

  useEffect(() => {
    void ensureAccountsLoaded();
    void loadGitHubConfig();
    void refreshGitHubCliStatus();
    void loadRuns();
  }, [ensureAccountsLoaded, loadGitHubConfig, refreshGitHubCliStatus, loadRuns]);

  useEffect(() => {
    if (productFilter === 'trae') return;
    void ensureWorkBuddyAccountsLoaded();
    void ensureWorkBuddyMonitoringStarted();
  }, [productFilter, ensureWorkBuddyAccountsLoaded, ensureWorkBuddyMonitoringStarted]);

  const ghReady = !!githubCliStatus?.available && !!githubCliStatus.authed;
  const configReady =
    githubConfig.enabled &&
    /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(githubConfig.repository) &&
    githubConfig.slots.length > 0;
  const uploadReady = ghReady && configReady;

  // 检测完成但上传条件不全 → 自动弹出引导（每次应用会话仅一次）。
  useEffect(() => {
    if (ghSetupAutoPrompted || !githubCliStatus) return;
    if (!uploadReady) {
      ghSetupAutoPrompted = true;
      setSetupOpen(true);
    }
  }, [githubCliStatus, uploadReady]);

  // 点「同步全部 GitHub」时条件不全 → 弹出引导而不是直接报错。
  const handleSyncAll = () => {
    if (!uploadReady) {
      setSetupOpen(true);
      return;
    }
    void syncGitHubAll();
  };

  const banner = latestBanner(runs);
  const checkinItems = [
    ...accounts.map((account) => ({ product: 'trae' as const, account })),
    ...workBuddyAccounts.map((account) => ({ product: 'workbuddy' as const, account })),
  ];
  const productGroups = groupCheckinProducts(checkinItems, productFilter);

  return (
    <div className="ck-page">
      <header className="ck-header">
        <h1 className="ck-title">
          <CloudUpload size={22} />
          云端签到管理
        </h1>
        <div className="ck-header-actions">
          <span
            className="ck-gh-badge"
            style={ghReady ? undefined : { cursor: 'pointer' }}
            onClick={() => {
              if (!ghReady) setSetupOpen(true);
            }}
            title={ghReady ? undefined : '点击查看 GitHub 上传准备引导'}
          >
            <span className={ghReady ? 'ck-gh-dot ck-gh-dot--ok' : 'ck-gh-dot ck-gh-dot--warn'} />
            {githubCliStatus
              ? ghReady
                ? 'gh 已就绪'
                : githubCliStatus.available
                  ? 'gh 未登录'
                  : '未检测到 gh'
              : 'gh 检测中…'}
          </span>
          <button
            type="button"
            className="wc-btn"
            onClick={() => void loadRuns()}
            disabled={runsLoading}
          >
            {runsLoading ? '查询中…' : '刷新运行记录'}
          </button>
          <button
            type="button"
            className="wc-btn wc-btn-primary"
            onClick={() => void triggerRun()}
            disabled={triggering || !ghReady || !githubConfig.enabled}
            title="手动触发云端签到（用于凭证修复后的验证/补签，日常由 Actions 凌晨 4 点自动执行）"
          >
            {triggering ? '触发中…' : '验证云端任务'}
          </button>
        </div>
      </header>

      {triggerMessage ? (
        <section className={triggerMessage.includes('失败') || triggerMessage.includes('中止') ? 'ck-banner ck-banner--error' : 'ck-banner ck-banner--running'}>
          {triggerMessage}
        </section>
      ) : null}

      {banner ? (
        <section className={`ck-banner ${banner.tone}`}>
          <div className="ck-banner-main">
            <div>{banner.main}</div>
            {banner.sub ? <div className="ck-banner-sub">{banner.sub}</div> : null}
          </div>
          {banner.url ? (
            <button
              type="button"
              className="ck-banner-link ck-banner-link--btn"
              onClick={() => void openExternalUrl(banner.url!)}
            >
              到 Actions 查看
            </button>
          ) : null}
        </section>
      ) : null}

      <section>
        <h2 className="ck-section-title">
          凭证健康（{checkinItems.length} 个账号）
          <span className="ck-section-actions">
            <div className="ck-product-filter" role="tablist" aria-label="签到产品筛选">
              {(['all', 'trae', 'workbuddy'] as const).map((product) => (
                <button key={product} type="button" role="tab" aria-selected={productFilter === product} className={productFilter === product ? 'ck-product-filter__item is-active' : 'ck-product-filter__item'} onClick={() => setProductFilter(product)}>{checkinProductLabel(product)}</button>
              ))}
            </div>
            {productFilter !== 'workbuddy' ? <button
              type="button"
              className="wc-btn"
              onClick={handleSyncAll}
              disabled={syncingAllGithub}
              title="把所有 TRAE 账号的最新凭证同步到 GitHub Secrets"
            >
              {syncingAllGithub && syncAllProgress
                ? `同步 TRAE ${syncAllProgress.done}/${syncAllProgress.total}…`
                : '同步 TRAE'}
            </button> : null}
            {productFilter !== 'trae' ? <button type="button" className="wc-btn" onClick={() => void syncWorkBuddyGitHub()} disabled={workBuddySyncing || workBuddyAccounts.length === 0} title="把启用自动签到的 WorkBuddy 账号同步到聚合 Secret">{workBuddySyncing ? '同步 WorkBuddy 中…' : '同步 WorkBuddy'}</button> : null}
          </span>
        </h2>
        <p className="ck-note">
          点击「刷新凭证」会切换并验证官方设备身份：有效 Token 直接同步 GitHub；临近过期时等待客户端轮换后再同步。
        </p>
        <div className="ck-product-groups">
          {productGroups.map((group) => (
            <section className="ck-product-group" key={group.product}>
              <h3 className="ck-product-group__title">
                <img src={group.product === 'trae' ? traeCnIcon : workBuddyIcon} alt="" width={18} height={18} />
                {checkinProductLabel(group.product)}
                <span>{group.items.length} 个账号</span>
              </h3>
              <div className="ck-cards">
                {group.items.map((item) => item.product === 'trae'
                  ? <CredentialCard key={`trae-${item.account.id}`} account={item.account} />
                  : <WorkBuddyCheckinCard key={`workbuddy-${item.account.id}`} account={item.account} />)}
              </div>
            </section>
          ))}
          {productGroups.length === 0 ? (
            <div className="ck-runs-empty">{checkinItems.length === 0 ? '还没有已导入的账号，请先到 TRAE 或 WorkBuddy 页面导入。' : `当前筛选没有 ${checkinProductLabel(productFilter)} 账号。`}</div>
          ) : null}
        </div>
      </section>

      <section>
        <h2 className="ck-section-title">运行历史（最近 {runs.length || 5} 次）</h2>
        <div className="ck-runs">
          {runsError ? <div className="ck-runs-error">{runsError}</div> : null}
          {!runsError && runs.length === 0 && !runsLoading ? (
            <div className="ck-runs-empty">暂无运行记录（每天北京时间 04:00 自动执行）</div>
          ) : null}
          {runs.map((run) => {
            const tone = runTone(run);
            const dotClass =
              tone === 'ok'
                ? 'ck-run-dot ck-run-dot--ok'
                : tone === 'error'
                  ? 'ck-run-dot ck-run-dot--error'
                  : tone === 'running'
                    ? 'ck-run-dot ck-run-dot--running'
                    : 'ck-run-dot';
            const label =
              tone === 'ok'
                ? '成功'
                : tone === 'error'
                  ? `失败（${run.conclusion}）`
                  : tone === 'running'
                    ? '运行中'
                    : run.conclusion ?? run.status;
            return (
              <div
                key={run.databaseId}
                className="ck-run-row"
                role="link"
                tabIndex={0}
                onClick={() => void openExternalUrl(run.url)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    void openExternalUrl(run.url);
                  }
                }}
              >
                <span className={dotClass} />
                <span className="ck-run-title">{run.displayTitle || 'Daily Checkin'}</span>
                <span className="ck-run-meta">{label}</span>
                <span className="ck-run-meta">
                  {run.event === 'workflow_dispatch' ? '手动' : '定时'} · {formatRelative(run.createdAt)}
                </span>
              </div>
            );
          })}
        </div>
      </section>

      <GhSetupDialog open={setupOpen} onClose={() => setSetupOpen(false)} />
    </div>
  );
}
