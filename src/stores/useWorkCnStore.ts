import { create } from 'zustand';
import {
  importCurrentWorkCnAccount,
  listWorkCnAccounts,
} from '../services/workCnService';
import type { WorkCnAccountView } from '../types/workCn';

interface WorkCnState {
  accounts: WorkCnAccountView[];
  loading: boolean;
  importing: boolean;
  error: string | null;
  lastImportWarning: string | null;
  loadAccounts: () => Promise<void>;
  importCurrent: (label?: string | null) => Promise<void>;
  clearError: () => void;
}

export const useWorkCnStore = create<WorkCnState>((set) => ({
  accounts: [],
  loading: false,
  importing: false,
  error: null,
  lastImportWarning: null,
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
  clearError() {
    set({ error: null });
  },
}));
