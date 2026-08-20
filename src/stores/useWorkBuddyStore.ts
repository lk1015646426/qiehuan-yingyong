import { create } from 'zustand';
import { listen } from '@tauri-apps/api/event';
import {
  deleteWorkBuddyAccount,
  getWorkBuddyInstallation,
  getWorkBuddySessionWatchStatus,
  importCurrentWorkBuddyAccount,
  listWorkBuddyAccounts,
  startWorkBuddySessionWatcher,
  switchWorkBuddyAccount,
  syncWorkBuddyGitHub,
  triggerWorkBuddyCheckin,
  getWorkBuddyAccountStatus,
  updateWorkBuddyAccount,
} from '../services/workBuddyService';
import type { WorkBuddyAccountStatus, WorkBuddyAccountUpdate, WorkBuddyAccountView, WorkBuddyInstallation, WorkBuddySessionWatchStatus } from '../types/workbuddy';
import { shouldLoadWorkBuddyAccounts } from '../utils/workBuddyLifecycle';

const WORKBUDDY_SESSION_WATCH_EVENT = 'workbuddy:session-watch';

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

interface WorkBuddyState {
  accounts: WorkBuddyAccountView[];
  accountsLoaded: boolean;
  installation: WorkBuddyInstallation | null;
  installationLoaded: boolean;
  installationLoading: boolean;
  monitorStarted: boolean;
  loading: boolean;
  importing: boolean;
  switchingId: string | null;
  updatingId: string | null;
  deletingId: string | null;
  syncing: boolean;
  checkingInId: string | null;
  statusById: Record<string, WorkBuddyAccountStatus>;
  statusLoadingById: Record<string, boolean>;
  sessionWatchStatus: WorkBuddySessionWatchStatus | null;
  error: string | null;
  notice: string | null;
  loadAccounts: () => Promise<void>;
  ensureAccountsLoaded: () => Promise<void>;
  refreshInstallation: () => Promise<void>;
  ensureInstallationLoaded: () => Promise<void>;
  ensureMonitoringStarted: () => Promise<void>;
  importCurrent: (displayName?: string | null) => Promise<boolean>;
  updateAccount: (accountId: string, update: WorkBuddyAccountUpdate) => Promise<void>;
  deleteAccount: (accountId: string) => Promise<void>;
  switchTo: (accountId: string) => Promise<void>;
  syncGitHub: () => Promise<void>;
  triggerCheckin: (accountId: string) => Promise<void>;
  refreshAccountStatus: (accountId: string) => Promise<void>;
  refreshAllStatuses: () => Promise<void>;
  loadSessionWatchStatus: () => Promise<void>;
  clearFeedback: () => void;
  subscribeToBackgroundEvents: () => Promise<() => void>;
}

export const useWorkBuddyStore = create<WorkBuddyState>((set, get) => ({
  accounts: [], accountsLoaded: false, installation: null, installationLoaded: false,
  installationLoading: false, monitorStarted: false, loading: false, importing: false,
  switchingId: null, updatingId: null, deletingId: null,
  syncing: false, checkingInId: null, statusById: {}, statusLoadingById: {}, sessionWatchStatus: null, error: null, notice: null,

  async loadAccounts() {
    set({ loading: true });
    try {
      const accounts = await listWorkBuddyAccounts();
      set({ accounts, accountsLoaded: true, error: null, loading: false });
      await get().refreshAllStatuses();
    } catch (error) {
      set({ error: messageOf(error), loading: false });
    }
  },
  async ensureAccountsLoaded() {
    const state = get();
    if (shouldLoadWorkBuddyAccounts(state.accountsLoaded, state.loading)) {
      await state.loadAccounts();
    }
  },
  async refreshInstallation() {
    if (get().installationLoading) return;
    set({ installationLoading: true });
    try {
      set({
        installation: await getWorkBuddyInstallation(),
        installationLoaded: true,
        installationLoading: false,
      });
    } catch (error) {
      set({ installationLoading: false, error: messageOf(error) });
    }
  },
  async ensureInstallationLoaded() {
    if (!get().installationLoaded) await get().refreshInstallation();
  },
  async ensureMonitoringStarted() {
    if (get().monitorStarted) return;
    try {
      const status = await startWorkBuddySessionWatcher();
      set({ monitorStarted: true, sessionWatchStatus: status });
    } catch {
      // 后台监测不可用时不阻塞账号管理。
    }
  },
  async importCurrent(displayName) {
    set({ importing: true, error: null });
    try {
      await importCurrentWorkBuddyAccount(displayName);
      await get().loadAccounts();
      set({ importing: false, notice: '已导入当前 WorkBuddy 账号' });
      void get().refreshInstallation();
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
      const account = await updateWorkBuddyAccount(accountId, update);
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
      const cleanupPending = await deleteWorkBuddyAccount(accountId);
      set((state) => ({
        accounts: state.accounts.filter((item) => item.id !== accountId),
        deletingId: null,
        notice: cleanupPending
          ? '本地账号已删除，GitHub 远端清理已排队'
          : 'WorkBuddy 账号已删除，GitHub 远端集合已更新',
      }));
      void get().refreshInstallation();
    } catch (error) { set({ deletingId: null, error: messageOf(error) }); }
  },
  async switchTo(accountId) {
    set({ switchingId: accountId, error: null, notice: null });
    try {
      const result = await switchWorkBuddyAccount(accountId);
      await get().loadAccounts();
      set({
        switchingId: null,
        notice: result.githubSyncPending
          ? '已切换并打开 WorkBuddy，GitHub 自动签到待同步'
          : '已切换并打开 WorkBuddy，GitHub 自动签到已同步',
      });
      void get().refreshInstallation();
    } catch (error) {
      set({ switchingId: null, error: messageOf(error) });
      await Promise.allSettled([get().loadAccounts(), get().refreshInstallation()]);
    }
  },
  async syncGitHub() {
    if (get().syncing) return;
    set({ syncing: true, error: null });
    try {
      await syncWorkBuddyGitHub();
      await get().loadAccounts();
      set({ syncing: false, notice: 'WorkBuddy 自动签到账号已同步到 GitHub' });
    } catch (error) { set({ syncing: false, error: messageOf(error) }); }
  },
  async triggerCheckin(accountId) {
    set({ checkingInId: accountId, error: null });
    try {
      await triggerWorkBuddyCheckin(accountId);
      set({ checkingInId: null, notice: '已触发该账号的云端签到任务' });
    } catch (error) { set({ checkingInId: null, error: messageOf(error) }); }
  },
  async refreshAccountStatus(accountId) {
    if (get().statusLoadingById[accountId]) return;
    set((state) => ({ statusLoadingById: { ...state.statusLoadingById, [accountId]: true } }));
    try {
      const status = await getWorkBuddyAccountStatus(accountId);
      set((state) => ({
        statusById: { ...state.statusById, [accountId]: status },
        statusLoadingById: { ...state.statusLoadingById, [accountId]: false },
      }));
    } catch (error) {
      set((state) => ({
        statusById: { ...state.statusById, [accountId]: {
          credits: null, todayReward: null, streakDays: null, updatedAt: Math.floor(Date.now() / 1000),
          creditsError: messageOf(error), activityError: messageOf(error),
        } },
        statusLoadingById: { ...state.statusLoadingById, [accountId]: false },
      }));
    }
  },
  async refreshAllStatuses() {
    for (const account of get().accounts) {
      await get().refreshAccountStatus(account.id);
    }
  },
  async loadSessionWatchStatus() {
    try { set({ sessionWatchStatus: await getWorkBuddySessionWatchStatus() }); } catch { /* 后台不可用时不影响账号管理。 */ }
  },
  async subscribeToBackgroundEvents() {
    const unlisten = await listen<WorkBuddySessionWatchStatus>(WORKBUDDY_SESSION_WATCH_EVENT, (event) => {
      set({ sessionWatchStatus: event.payload });
      void get().loadAccounts();
    });
    return unlisten;
  },
  clearFeedback() { set({ error: null, notice: null }); },
}));
