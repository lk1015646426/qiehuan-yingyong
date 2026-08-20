import { create } from 'zustand';
import { listen } from '@tauri-apps/api/event';
import {
  clearWorkCnCredentials,
  deleteWorkCnAccount,
  getWorkCnCredits,
  getWorkCnGitHubCliStatus,
  getWorkCnGitHubConfig,
  getWorkCnSessionWatchStatus,
  GH_SETUP_EVENT,
  ghLoginWithToken,
  importCurrentWorkCnAccount,
  listWorkCnAccounts,
  openWorkCnLogFolder,
  saveWorkCnGitHubConfig,
  setupGhCli,
  switchWorkCnAccount,
  syncWorkCnGitHubAccount,
} from '../services/workCnService';
import type { GhSetupProgress } from '../services/workCnService';
import type {
  WorkCnAccountView,
  WorkCnCreditsSummary,
  WorkCnSwitchResult,
  WorkCnGitHubConfig,
  WorkCnGitHubCliStatus,
  WorkCnGitHubSyncResult,
  WorkCnSessionWatchStatus,
} from '../types/workCn';
import { shouldLoadWorkCnAccounts } from '../utils/workCnLifecycle';

interface WorkCnState {
  accounts: WorkCnAccountView[];
  accountsLoaded: boolean;
  loading: boolean;
  importing: boolean;
  switchingId: string | null;
  deletingId: string | null;
  error: string | null;
  lastImportWarning: string | null;
  lastSwitchResult: WorkCnSwitchResult | null;
  // 积分余额（仅查询，绝不签到）。key 为账号 id。
  creditsById: Record<string, WorkCnCreditsSummary>;
  creditsErrorById: Record<string, string | null>;
  refreshingCreditsIds: Record<string, boolean>;
  // GitHub Secrets 同步（阶段 6）。
  githubConfig: WorkCnGitHubConfig;
  githubCliStatus: WorkCnGitHubCliStatus | null;
  githubSyncingById: Record<string, boolean>;
  githubSyncResultById: Record<string, WorkCnGitHubSyncResult | null>;
  // 一键同步全部（顶栏按钮）：进行中标志 + 进度。
  syncingAllGithub: boolean;
  syncAllProgress: { done: number; total: number } | null;
  // gh 自动安装（引导弹窗「自动下载并安装」按钮）。
  ghSetup: GhSetupProgress | null;
  ghSetupError: string | null;
  setupGh: () => Promise<void>;
  ghLogin: (token: string) => Promise<void>;
  // 后台会话监测（阶段 7）。
  sessionWatchStatus: WorkCnSessionWatchStatus | null;
  loadGitHubConfig: () => Promise<void>;
  saveGitHubConfig: (config: WorkCnGitHubConfig) => Promise<void>;
  refreshGitHubCliStatus: () => Promise<void>;
  syncGitHub: (accountId: string) => Promise<void>;
  syncGitHubAll: () => Promise<void>;
  loadSessionWatchStatus: () => Promise<void>;
  applySessionWatchStatus: (status: WorkCnSessionWatchStatus) => void;
  clearCredentials: () => Promise<void>;
  loadAccounts: () => Promise<void>;
  refreshAccountMetadata: () => Promise<void>;
  ensureAccountsLoaded: () => Promise<void>;
  importCurrent: (label?: string | null) => Promise<void>;
  switchTo: (accountId: string) => Promise<void>;
  deleteAccount: (accountId: string) => Promise<void>;
  openLogs: () => Promise<void>;
  refreshCredits: (accountId: string, forceRefresh?: boolean) => Promise<void>;
  clearError: () => void;
}

export const useWorkCnStore = create<WorkCnState>((set, get) => ({
  accounts: [],
  accountsLoaded: false,
  loading: false,
  importing: false,
  switchingId: null,
  deletingId: null,
  error: null,
  lastImportWarning: null,
  lastSwitchResult: null,
  creditsById: {},
  creditsErrorById: {},
  refreshingCreditsIds: {},
  githubConfig: { enabled: false, repository: '', slots: [] },
  githubCliStatus: null,
  githubSyncingById: {},
  githubSyncResultById: {},
  syncingAllGithub: false,
  syncAllProgress: null,
  ghSetup: null,
  ghSetupError: null,
  sessionWatchStatus: null,
  async loadAccounts() {
    if (get().loading) return;
    set({ loading: true, error: null });
    try {
      const accounts = await listWorkCnAccounts();
      set({ accounts, accountsLoaded: true, loading: false });
      // 载入时先本地解析缓存积分快速展示，再静默联网刷新一次真实余额
      // （仅查询用量接口，绝不签到/领取）。
      void Promise.all(
        accounts.map(async (account) => {
          try {
            const cached = await getWorkCnCredits(account.id, false);
            set((state) => ({
              creditsById: { ...state.creditsById, [account.id]: cached },
            }));
          } catch {
            // 缓存解析失败不影响列表展示。
          }
          try {
            const fresh = await getWorkCnCredits(account.id, true);
            set((state) => ({
              creditsById: { ...state.creditsById, [account.id]: fresh },
            }));
          } catch {
            // 联网刷新失败保留缓存值，等用户点击“刷新积分”重试。
          }
        }),
      );
    } catch (err) {
      set({
        error: err instanceof Error ? err.message : String(err),
        loading: false,
      });
    }
  },
  async refreshAccountMetadata() {
    if (get().loading) return;
    set({ loading: true });
    try {
      set({ accounts: await listWorkCnAccounts(), accountsLoaded: true, loading: false });
    } catch (err) {
      set({ error: err instanceof Error ? err.message : String(err), loading: false });
    }
  },
  async ensureAccountsLoaded() {
    const state = get();
    if (shouldLoadWorkCnAccounts(state.accountsLoaded, state.loading)) {
      await state.loadAccounts();
    }
  },
  async importCurrent(label) {
    set({ importing: true, error: null, lastImportWarning: null });
    try {
      const account = await importCurrentWorkCnAccount(label);
      set((state) => {
        const others = state.accounts.filter((a) => a.id !== account.id);
        return {
          accounts: [account, ...others],
          importing: false,
          lastImportWarning: account.warnings.length
            ? account.warnings.join('；')
            : null,
        };
      });
      // 导入成功后立即联网查询一次积分（仅查用量接口，绝不签到/领取），
      // 否则新账号无缓存，界面只会显示“暂无积分数据”。
      void get().refreshCredits(account.id, true);
    } catch (err) {
      set({
        error: err instanceof Error ? err.message : String(err),
        importing: false,
      });
    }
  },
  async switchTo(accountId) {
    set({ switchingId: accountId, error: null, lastSwitchResult: null });
    try {
      const result = await switchWorkCnAccount(accountId);
      set({ switchingId: null, lastSwitchResult: result });
      // Refresh the account list so the "current" marker (if any) updates.
      void listWorkCnAccounts()
        .then((accounts) => set({ accounts }))
        .catch(() => undefined);
    } catch (err) {
      set({
        error: err instanceof Error ? err.message : String(err),
        switchingId: null,
      });
    }
  },
  async deleteAccount(accountId) {
    set({ deletingId: accountId, error: null });
    try {
      await deleteWorkCnAccount(accountId);
      set((state) => {
        const creditsById = { ...state.creditsById };
        delete creditsById[accountId];
        const creditsErrorById = { ...state.creditsErrorById };
        delete creditsErrorById[accountId];
        const githubSyncResultById = { ...state.githubSyncResultById };
        delete githubSyncResultById[accountId];
        return {
          accounts: state.accounts.filter((a) => a.id !== accountId),
          creditsById,
          creditsErrorById,
          githubSyncResultById,
          deletingId: null,
        };
      });
    } catch (err) {
      set({
        error: err instanceof Error ? err.message : String(err),
        deletingId: null,
      });
    }
  },
  async openLogs() {
    try {
      await openWorkCnLogFolder();
    } catch (err) {
      set({ error: err instanceof Error ? err.message : String(err) });
    }
  },
  async refreshCredits(accountId, forceRefresh = false) {
    if (get().refreshingCreditsIds[accountId]) return;
    set((state) => ({ refreshingCreditsIds: { ...state.refreshingCreditsIds, [accountId]: true } }));
    try {
      const summary = await getWorkCnCredits(accountId, forceRefresh);
      set((state) => ({
        creditsById: { ...state.creditsById, [accountId]: summary },
        creditsErrorById: { ...state.creditsErrorById, [accountId]: null },
        refreshingCreditsIds: Object.fromEntries(Object.entries(state.refreshingCreditsIds).filter(([id]) => id !== accountId)),
      }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      set((state) => ({
        refreshingCreditsIds: Object.fromEntries(Object.entries(state.refreshingCreditsIds).filter(([id]) => id !== accountId)),
        creditsErrorById: { ...state.creditsErrorById, [accountId]: message },
      }));
    }
  },
  async loadGitHubConfig() {
    try {
      const config = await getWorkCnGitHubConfig();
      set({ githubConfig: config });
    } catch {
      // keep defaults
    }
  },
  async saveGitHubConfig(config) {
    await saveWorkCnGitHubConfig(config);
    set({ githubConfig: config });
    // re-read CLI status after a config change
    void getWorkCnGitHubCliStatus()
      .then((status) => set({ githubCliStatus: status }))
      .catch(() => undefined);
  },
  async refreshGitHubCliStatus() {
    try {
      const status = await getWorkCnGitHubCliStatus();
      set({ githubCliStatus: status });
    } catch {
      // ignore
    }
  },
  // 自动下载并安装 gh：订阅进度事件更新 ghSetup，结束后自动刷新 CLI 状态。
  async setupGh() {
    set({ ghSetupError: null, ghSetup: { phase: 'downloading', received: 0, total: 0 } });
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<GhSetupProgress>(GH_SETUP_EVENT, (event) => {
      if (!disposed) {
        set({ ghSetup: event.payload });
      }
    }).then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    try {
      await setupGhCli();
      await get().refreshGitHubCliStatus();
    } catch (err) {
      set({
        ghSetupError: err instanceof Error ? err.message : String(err),
        ghSetup: { phase: 'failed', received: 0, total: 0 },
      });
    } finally {
      disposed = true;
      unlisten?.();
    }
  },
  // PAT 登录 gh：成功后刷新 CLI 状态。
  async ghLogin(token) {
    try {
      await ghLoginWithToken(token);
      await get().refreshGitHubCliStatus();
    } catch (err) {
      throw err instanceof Error ? err : new Error(String(err));
    }
  },
  async syncGitHub(accountId) {
    set((state) => ({
      githubSyncingById: { ...state.githubSyncingById, [accountId]: true },
    }));
    try {
      const result = await syncWorkCnGitHubAccount(accountId);
      set((state) => ({
        githubSyncResultById: { ...state.githubSyncResultById, [accountId]: result },
        githubSyncingById: { ...state.githubSyncingById, [accountId]: false },
      }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      set((state) => ({
        githubSyncResultById: {
          ...state.githubSyncResultById,
          [accountId]: {
            accountId,
            synced: false,
            skipped: false,
            skipReason: null,
            error: message,
            syncedAt: Math.floor(Date.now() / 1000),
          },
        },
        githubSyncingById: { ...state.githubSyncingById, [accountId]: false },
      }));
    }
  },
  // 一键同步全部：顺序同步所有已绑定槽位的账号（避免 gh CLI 并发竞争），
  // 单个失败不中断，最终逐卡展示结果。
  async syncGitHubAll() {
    const { accounts, githubConfig, syncingAllGithub } = get();
    if (syncingAllGithub) return;
    if (!githubConfig.enabled) {
      set({ error: 'GitHub 同步未启用，请先在设置中启用并绑定槽位' });
      return;
    }
    const boundIds = new Set(githubConfig.slots.map((slot) => slot.accountId));
    const targets = accounts.filter((account) => boundIds.has(account.id));
    if (targets.length === 0) {
      set({ error: '没有账号绑定 GitHub 槽位，请先在设置中完成「账号 → 槽位」绑定' });
      return;
    }
    set({ syncingAllGithub: true, syncAllProgress: { done: 0, total: targets.length } });
    for (let i = 0; i < targets.length; i += 1) {
      await get().syncGitHub(targets[i].id);
      set({ syncAllProgress: { done: i + 1, total: targets.length } });
    }
    set({ syncingAllGithub: false, syncAllProgress: null });
  },
  async loadSessionWatchStatus() {
    try {
      const status = await getWorkCnSessionWatchStatus();
      set({ sessionWatchStatus: status });
    } catch {
      // 后台命令不可用时保持 null，横幅按「未启动」展示。
    }
  },
  applySessionWatchStatus(status) {
    set({ sessionWatchStatus: status });
    // Token 更新后刷新账号卡（含积分/快照完整度）与 GitHub 绑定状态。
    if (status.outcome === 'TOKEN_UPDATED') {
      void get().loadAccounts();
      void get().loadGitHubConfig();
    }
  },
  async clearCredentials() {
    await clearWorkCnCredentials();
    set({
      accounts: [],
      accountsLoaded: true,
      creditsById: {},
      creditsErrorById: {},
      githubConfig: { enabled: false, repository: '', slots: [] },
      githubSyncResultById: {},
      lastImportWarning: null,
      lastSwitchResult: null,
    });
  },
  clearError() {
    set({ error: null });
  },
}));
