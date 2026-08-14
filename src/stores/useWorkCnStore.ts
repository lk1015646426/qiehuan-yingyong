import { create } from 'zustand';
import {
  getWorkCnCredits,
  importCurrentWorkCnAccount,
  listWorkCnAccounts,
  switchWorkCnAccount,
} from '../services/workCnService';
import type {
  WorkCnAccountView,
  WorkCnCreditsSummary,
  WorkCnSwitchResult,
} from '../types/workCn';

interface WorkCnState {
  accounts: WorkCnAccountView[];
  loading: boolean;
  importing: boolean;
  switchingId: string | null;
  error: string | null;
  lastImportWarning: string | null;
  lastSwitchResult: WorkCnSwitchResult | null;
  // 积分余额（仅查询，绝不签到）。key 为账号 id。
  creditsById: Record<string, WorkCnCreditsSummary>;
  creditsErrorById: Record<string, string | null>;
  refreshingCreditsId: string | null;
  loadAccounts: () => Promise<void>;
  importCurrent: (label?: string | null) => Promise<void>;
  switchTo: (accountId: string) => Promise<void>;
  refreshCredits: (accountId: string, forceRefresh?: boolean) => Promise<void>;
  clearError: () => void;
}

export const useWorkCnStore = create<WorkCnState>((set) => ({
  accounts: [],
  loading: false,
  importing: false,
  switchingId: null,
  error: null,
  lastImportWarning: null,
  lastSwitchResult: null,
  creditsById: {},
  creditsErrorById: {},
  refreshingCreditsId: null,
  async loadAccounts() {
    set({ loading: true, error: null });
    try {
      const accounts = await listWorkCnAccounts();
      set({ accounts, loading: false });
      // 载入时仅解析各账号已缓存的积分（本地解析，不联网、不签到）。
      void Promise.all(
        accounts.map(async (account) => {
          try {
            const summary = await getWorkCnCredits(account.id, false);
            set((state) => ({
              creditsById: { ...state.creditsById, [account.id]: summary },
            }));
          } catch {
            // 查询失败不影响列表展示，等用户点击“刷新积分”。
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
  clearError() {
    set({ error: null });
  },
}));
