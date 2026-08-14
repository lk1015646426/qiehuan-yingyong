import { create } from 'zustand';
import {
  importCurrentWorkCnAccount,
  listWorkCnAccounts,
  switchWorkCnAccount,
} from '../services/workCnService';
import type { WorkCnAccountView, WorkCnSwitchResult } from '../types/workCn';

interface WorkCnState {
  accounts: WorkCnAccountView[];
  loading: boolean;
  importing: boolean;
  switchingId: string | null;
  error: string | null;
  lastImportWarning: string | null;
  lastSwitchResult: WorkCnSwitchResult | null;
  loadAccounts: () => Promise<void>;
  importCurrent: (label?: string | null) => Promise<void>;
  switchTo: (accountId: string) => Promise<void>;
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
  async loadAccounts() {
    set({ loading: true, error: null });
    try {
      const accounts = await listWorkCnAccounts();
      set({ accounts, loading: false });
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
  clearError() {
    set({ error: null });
  },
}));
