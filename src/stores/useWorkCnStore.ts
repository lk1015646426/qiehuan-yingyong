import { create } from 'zustand';
import {
  clearWorkCnCredentials,
  deleteWorkCnAccount,
  getWorkCnCredits,
  getWorkCnGitHubCliStatus,
  getWorkCnGitHubConfig,
  getWorkCnSessionWatchStatus,
  importCurrentWorkCnAccount,
  listWorkCnAccounts,
  openWorkCnLogFolder,
  saveWorkCnGitHubConfig,
  switchWorkCnAccount,
  syncWorkCnGitHubAccount,
} from '../services/workCnService';
import type {
  WorkCnAccountView,
  WorkCnCreditsSummary,
  WorkCnSwitchResult,
  WorkCnGitHubConfig,
  WorkCnGitHubCliStatus,
  WorkCnGitHubSyncResult,
  WorkCnSessionWatchStatus,
} from '../types/workCn';

interface WorkCnState {
  accounts: WorkCnAccountView[];
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
  refreshingCreditsId: string | null;
  // GitHub Secrets 同步（阶段 6）。
  githubConfig: WorkCnGitHubConfig;
  githubCliStatus: WorkCnGitHubCliStatus | null;
  githubSyncingById: Record<string, boolean>;
  githubSyncResultById: Record<string, WorkCnGitHubSyncResult | null>;
  // 后台会话监测（阶段 7）。
  sessionWatchStatus: WorkCnSessionWatchStatus | null;
  loadGitHubConfig: () => Promise<void>;
  saveGitHubConfig: (config: WorkCnGitHubConfig) => Promise<void>;
  refreshGitHubCliStatus: () => Promise<void>;
  syncGitHub: (accountId: string) => Promise<void>;
  loadSessionWatchStatus: () => Promise<void>;
  applySessionWatchStatus: (status: WorkCnSessionWatchStatus) => void;
  clearCredentials: () => Promise<void>;
  loadAccounts: () => Promise<void>;
  importCurrent: (label?: string | null) => Promise<void>;
  switchTo: (accountId: string) => Promise<void>;
  deleteAccount: (accountId: string) => Promise<void>;
  openLogs: () => Promise<void>;
  refreshCredits: (accountId: string, forceRefresh?: boolean) => Promise<void>;
  clearError: () => void;
}

export const useWorkCnStore = create<WorkCnState>((set, get) => ({
  accounts: [],
  loading: false,
  importing: false,
  switchingId: null,
  deletingId: null,
  error: null,
  lastImportWarning: null,
  lastSwitchResult: null,
  creditsById: {},
  creditsErrorById: {},
  refreshingCreditsId: null,
  githubConfig: { enabled: false, repository: '', slots: [] },
  githubCliStatus: null,
  githubSyncingById: {},
  githubSyncResultById: {},
  sessionWatchStatus: null,
  async loadAccounts() {
    set({ loading: true, error: null });
    try {
      const accounts = await listWorkCnAccounts();
      set({ accounts, loading: false });
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
    set({ refreshingCreditsId: accountId });
    try {
      const summary = await getWorkCnCredits(accountId, forceRefresh);
      set((state) => ({
        creditsById: { ...state.creditsById, [accountId]: summary },
        creditsErrorById: { ...state.creditsErrorById, [accountId]: null },
        refreshingCreditsId: null,
      }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      set((state) => ({
        refreshingCreditsId: null,
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
