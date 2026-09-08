import { create } from 'zustand';
import {
  deleteZhipuAccount,
  getZhipuAccountStatus,
  importCurrentZhipuAccount,
  importZhipuAccount,
  listZhipuAccounts,
  syncZhipuGitHub,
  triggerZhipuCheckin,
  updateZhipuAccount,
} from '../services/zhipuService';
import type { ZhipuAccountStatus, ZhipuAccountUpdate, ZhipuAccountView } from '../types/zhipu';

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

interface ZhipuState {
  accounts: ZhipuAccountView[];
  accountsLoaded: boolean;
  loading: boolean;
  importing: boolean;
  updatingId: string | null;
  deletingId: string | null;
  syncing: boolean;
  checkingInId: string | null;
  statusById: Record<string, ZhipuAccountStatus>;
  statusLoadingById: Record<string, boolean>;
  error: string | null;
  notice: string | null;
  loadAccounts: () => Promise<void>;
  ensureAccountsLoaded: () => Promise<void>;
  importCurrent: (displayName?: string | null) => Promise<boolean>;
  importManual: (accessToken: string, refreshToken?: string | null, displayName?: string | null) => Promise<boolean>;
  updateAccount: (accountId: string, update: ZhipuAccountUpdate) => Promise<void>;
  deleteAccount: (accountId: string) => Promise<void>;
  syncGitHub: () => Promise<void>;
  triggerCheckin: (accountId: string) => Promise<void>;
  refreshAccountStatus: (accountId: string) => Promise<void>;
  refreshAllStatuses: () => Promise<void>;
  clearFeedback: () => void;
}

export const useZhipuStore = create<ZhipuState>((set, get) => ({
  accounts: [], accountsLoaded: false, loading: false, importing: false,
  updatingId: null, deletingId: null, syncing: false, checkingInId: null,
  statusById: {}, statusLoadingById: {}, error: null, notice: null,

  async loadAccounts() {
    set({ loading: true });
    try {
      const accounts = await listZhipuAccounts();
      set({ accounts, accountsLoaded: true, error: null, loading: false });
      await get().refreshAllStatuses();
    } catch (error) {
      set({ error: messageOf(error), loading: false });
    }
  },
  async ensureAccountsLoaded() {
    if (!get().accountsLoaded) await get().loadAccounts();
  },
  async importCurrent(displayName) {
    set({ importing: true, error: null });
    try {
      const [, notice] = await importCurrentZhipuAccount(displayName);
      await get().loadAccounts();
      set({ importing: false, notice: `已导入当前清言账号：${notice.message}` });
      return true;
    } catch (error) {
      set({ importing: false, error: messageOf(error) });
      return false;
    }
  },
  async importManual(accessToken, refreshToken, displayName) {
    set({ importing: true, error: null });
    try {
      const [, notice] = await importZhipuAccount(accessToken, refreshToken, displayName);
      await get().loadAccounts();
      set({ importing: false, notice: notice.message });
      return true;
    } catch (error) {
      set({ importing: false, error: messageOf(error) });
      return false;
    }
  },
  async updateAccount(accountId, update) {
    if (get().updatingId !== null) return;
    const previous = get().accounts.find((item) => item.id === accountId);
    set((state) => ({
      updatingId: accountId,
      error: null,
      accounts: state.accounts.map((item) => item.id === accountId ? {
        ...item,
        displayName: update.displayName ?? item.displayName,
        checkinEnabled: update.checkinEnabled ?? item.checkinEnabled,
        lastGithubSyncState: 'pending',
        lastGithubSyncError: null,
      } : item),
    }));
    try {
      const account = await updateZhipuAccount(accountId, update);
      set((state) => ({
        updatingId: null,
        accounts: state.accounts.map((item) => item.id === accountId ? account : item),
      }));
    } catch (error) {
      set((state) => ({
        updatingId: null,
        error: messageOf(error),
        accounts: previous
          ? state.accounts.map((item) => item.id === accountId ? previous : item)
          : state.accounts,
      }));
    }
  },
  async deleteAccount(accountId) {
    set({ deletingId: accountId, error: null });
    try {
      await deleteZhipuAccount(accountId);
      set((state) => ({
        accounts: state.accounts.filter((item) => item.id !== accountId),
        deletingId: null,
        notice: '智谱账号已删除，GitHub 远端集合将在后台同步时更新',
      }));
    } catch (error) { set({ deletingId: null, error: messageOf(error) }); }
  },
  async syncGitHub() {
    if (get().syncing) return;
    set({ syncing: true, error: null });
    try {
      await syncZhipuGitHub();
      await get().loadAccounts();
      set({ syncing: false, notice: '智谱自动签到账号已同步到 GitHub' });
    } catch (error) { set({ syncing: false, error: messageOf(error) }); }
  },
  async triggerCheckin(accountId) {
    set({ checkingInId: accountId, error: null });
    try {
      await triggerZhipuCheckin(accountId);
      set({ checkingInId: null, notice: '已触发该账号的云端签到任务' });
    } catch (error) { set({ checkingInId: null, error: messageOf(error) }); }
  },
  async refreshAccountStatus(accountId) {
    if (get().statusLoadingById[accountId]) return;
    set((state) => ({ statusLoadingById: { ...state.statusLoadingById, [accountId]: true } }));
    try {
      const status = await getZhipuAccountStatus(accountId);
      set((state) => ({
        statusById: { ...state.statusById, [accountId]: status },
        statusLoadingById: { ...state.statusLoadingById, [accountId]: false },
      }));
    } catch (error) {
      set((state) => ({
        statusById: { ...state.statusById, [accountId]: {
          currentScore: null, activityStatus: null,
          updatedAt: Math.floor(Date.now() / 1000),
          scoreError: messageOf(error),
        } },
        statusLoadingById: { ...state.statusLoadingById, [accountId]: false },
      }));
    }
  },
  async refreshAllStatuses() {
    // 逐账号联网查询积分；并行执行避免 N 个账号的网络等待叠加。
    await Promise.allSettled(get().accounts.map((account) => get().refreshAccountStatus(account.id)));
  },
  clearFeedback() { set({ error: null, notice: null }); },
}));
