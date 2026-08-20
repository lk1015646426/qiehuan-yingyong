// 云端签到面板 store：云端运行状态 + 半自动凭证刷新流编排。
//
// 半自动刷新流（用户确认的方案）：
//   切换到目标账号并打开客户端 → 等待客户端刷新 token（事件 + 轮询双通道）
//   → 同步新凭证到 GitHub Secrets → 完成，提供「切回原账号」按钮（不自动切回）。
// 编排全部复用 useWorkCnStore 的现有原子动作，不新增后端状态机。

import { create } from 'zustand';
import { listen } from '@tauri-apps/api/event';
import {
  listCheckinWorkflowRuns,
  localCheckin as localCheckinAccount,
  triggerCheckinWorkflow,
} from '../services/checkinService';
import { WORK_CN_SESSION_WATCH_EVENT } from '../services/workCnService';
import type { CheckinWorkflowRun, LocalCheckinResult, RefreshFlowState } from '../types/checkin';
import type { WorkCnSessionWatchStatus } from '../types/workCn';
import {
  didTokenRotate,
  hasCompleteOfficialDeviceSnapshot,
  isVerifiedRefreshTarget,
  shouldSyncWithoutWaiting,
  shouldWaitForTokenRotation,
} from '../utils/tokenRotation';
import { useWorkCnStore } from './useWorkCnStore';

// 等待客户端刷新的总超时。
const WAIT_TIMEOUT_MS = 180_000;
// 等待阶段轮询间隔。
const POLL_INTERVAL_MS = 5_000;

interface CheckinState {
  runs: CheckinWorkflowRun[];
  runsLoading: boolean;
  runsError: string | null;
  triggering: boolean;
  triggerMessage: string | null;
  refreshFlow: RefreshFlowState | null;
  /** 本地单账号签到（诊断/补签）状态，按账号 id 索引。 */
  localCheckinById: Record<string, { loading: boolean; result: LocalCheckinResult | null }>;

  loadRuns: () => Promise<void>;
  triggerRun: () => Promise<void>;
  startRefreshFlow: (accountId: string) => Promise<void>;
  resetRefreshFlow: () => void;
  /** 本地单账号签到（诊断/补签）：从本机直接调用 TRAE 签到 API。 */
  localCheckin: (accountId: string) => Promise<void>;
}

function flowRunning(flow: RefreshFlowState | null): boolean {
  return !!flow && flow.phase !== 'done' && flow.phase !== 'failed';
}

export const useCheckinStore = create<CheckinState>((set, get) => ({
  runs: [],
  runsLoading: false,
  runsError: null,
  triggering: false,
  triggerMessage: null,
  refreshFlow: null,
  localCheckinById: {},

  async loadRuns() {
    set({ runsLoading: true, runsError: null });
    try {
      const runs = await listCheckinWorkflowRuns(5);
      set({ runs, runsLoading: false });
    } catch (err) {
      set({
        runsLoading: false,
        runsError: err instanceof Error ? err.message : String(err),
      });
    }
  },

  async triggerRun() {
    if (get().triggering) return;
    set({ triggering: true, triggerMessage: null });
    try {
      await triggerCheckinWorkflow();
      set({
        triggering: false,
        triggerMessage: '已触发，约 1-2 分钟出结果，稍后可点「刷新状态」查看',
      });
      // 触发后延迟自动刷新一次，尽量让用户看到 queued/in_progress 状态。
      setTimeout(() => {
        void get().loadRuns();
      }, 4_000);
    } catch (err) {
      set({
        triggering: false,
        triggerMessage: err instanceof Error ? err.message : String(err),
      });
    }
  },

  // —— 本地单账号签到（诊断/补签，用户 2026-08-16 决策） ——

  async localCheckin(accountId) {
    set((state) => ({
      localCheckinById: {
        ...state.localCheckinById,
        [accountId]: { loading: true, result: state.localCheckinById[accountId]?.result ?? null },
      },
    }));
    try {
      const result = await localCheckinAccount(accountId);
      set((state) => ({
        localCheckinById: {
          ...state.localCheckinById,
          [accountId]: { loading: false, result },
        },
      }));
    } catch (err) {
      set((state) => ({
        localCheckinById: {
          ...state.localCheckinById,
          [accountId]: {
            loading: false,
            result: {
              accountId,
              ok: false,
              message: err instanceof Error ? err.message : String(err),
              alreadyCheckedIn: false,
              outcome: 'failed',
              stage: 'status',
              businessCode: null,
              httpStatus: null,
              claimAttempted: false,
              tokenExpiryState: 'unknown',
              devicePresent: false,
            },
          },
        },
      }));
    }
  },

  // —— 半自动刷新流 ——————————————————————————————

  async startRefreshFlow(accountId) {
    if (flowRunning(get().refreshFlow)) return;
    const workCn = useWorkCnStore.getState();

    // 1) 记录原活跃账号（优先后台会话监测，回退最近使用）。
    const accounts = workCn.accounts;
    const targetBefore = accounts.find((account) => account.id === accountId);
    const tokenMetadataBefore = {
      tokenIssuedAt: targetBefore?.tokenIssuedAt ?? null,
      tokenExpiresAt: targetBefore?.tokenExpiresAt ?? null,
    };
    let previousAccountId: string | null = workCn.sessionWatchStatus?.accountId ?? null;
    if (!previousAccountId && accounts.some((a) => a.lastUsed > 0)) {
      previousAccountId = accounts.reduce((latest, a) =>
        a.lastUsed > latest.lastUsed ? a : latest,
      ).id;
    }
    if (previousAccountId === accountId) {
      previousAccountId = null; // 目标就是当前账号，无需切回
    }

    const startedAt = Date.now();
    const fail = (message: string) =>
      set((state) => ({
        refreshFlow: state.refreshFlow ? { ...state.refreshFlow, phase: 'failed', message } : null,
      }));
    const update = (phase: RefreshFlowState['phase'], message: string) =>
      set((state) => ({
        refreshFlow: state.refreshFlow ? { ...state.refreshFlow, phase, message } : null,
      }));

    // 2) 切换并打开客户端（switchTo 失败不抛错，通过 lastSwitchResult 判定）。
    set({
      refreshFlow: {
        accountId,
        phase: 'switching',
        message: '正在切换账号并打开客户端…',
        startedAt,
        previousAccountId,
      },
    });
    await workCn.switchTo(accountId);
    const switchResult = useWorkCnStore.getState().lastSwitchResult;
    if (!switchResult || !isVerifiedRefreshTarget(switchResult, accountId)) {
      fail(`切换失败：${useWorkCnStore.getState().error ?? '未知错误'}`);
      return;
    }

    // 切换后从账号库读取脱敏快照，继续验证设备快照是否可用于同步。
    // UID 已脱敏，不能与后端切换阶段验证过的真实 UID 直接比较。
    await useWorkCnStore.getState().refreshAccountMetadata();
    const targetAfterSwitch = useWorkCnStore
      .getState()
      .accounts.find((account) => account.id === accountId);
    if (!targetAfterSwitch || !hasCompleteOfficialDeviceSnapshot(targetAfterSwitch)) {
      fail('当前账号官方设备快照不完整，请在官方客户端完成登录后重新导入账号');
      return;
    }

    const needsTokenRotation = shouldWaitForTokenRotation(
      {
        tokenIssuedAt: targetAfterSwitch.tokenIssuedAt,
        tokenExpiresAt: targetAfterSwitch.tokenExpiresAt,
      },
      Math.floor(Date.now() / 1000),
    );

    const tokenChangedDuringSwitch =
      switchResult.tokenChanged ||
      didTokenRotate(tokenMetadataBefore, {
        tokenIssuedAt: targetAfterSwitch.tokenIssuedAt,
        tokenExpiresAt: targetAfterSwitch.tokenExpiresAt,
      });
    if (shouldSyncWithoutWaiting(needsTokenRotation, tokenChangedDuringSwitch)) {
      update(
        'syncing',
        needsTokenRotation
          ? '切换期间 Token 已更新，正在同步 GitHub…'
          : '当前 Token 仍有效，已确认官方设备身份，正在同步 GitHub…',
      );
      await useWorkCnStore.getState().syncGitHub(accountId);
      const result = useWorkCnStore.getState().githubSyncResultById[accountId];
      if (!result?.synced) {
        fail(`同步 GitHub 失败：${result?.error ?? result?.skipReason ?? '未知原因'}`);
        return;
      }
      update(
        'done',
        needsTokenRotation
          ? '已同步切换期间更新的 Token 与官方设备身份，可切回原账号'
          : '有效 Token 与官方设备身份已同步 GitHub，可切回原账号',
      );
      return;
    }

    // 3) 仅对已过期、即将过期或有效期未知的 Token 等待客户端轮换。
    update('waiting', '等待客户端刷新 token（最长 3 分钟）…');
    let unlisten: (() => void) | undefined;
    let disposed = false;
    const tokenUpdated = new Promise<void>((resolve) => {
      void listen<WorkCnSessionWatchStatus>(WORK_CN_SESSION_WATCH_EVENT, (event) => {
        const payload = event.payload;
        if (
          !disposed &&
          payload.outcome === 'TOKEN_UPDATED' &&
          payload.accountId === accountId
        ) {
          resolve();
        }
      }).then((fn) => {
        if (disposed) {
          fn();
        } else {
          unlisten = fn;
        }
      });
    });

    const deadline = Date.now() + WAIT_TIMEOUT_MS;
    let tokenFresh = false;
    while (Date.now() < deadline) {
      const race = await Promise.race([
        tokenUpdated.then(() => 'event' as const),
        new Promise<'poll' | 'timeout'>((resolve) => {
          const delay = Math.min(POLL_INTERVAL_MS, deadline - Date.now());
          setTimeout(() => resolve(delay > 0 ? 'poll' : 'timeout'), Math.max(delay, 0));
        }),
      ]);
      if (race === 'event') {
        tokenFresh = true;
        break;
      }
      if (race === 'timeout') {
        break;
      }
      // 轮询兜底：只有 iat/exp 相比切换前真实变化才算刷新成功。
      await useWorkCnStore.getState().refreshAccountMetadata();
      const target = useWorkCnStore
        .getState()
        .accounts.find((a) => a.id === accountId);
      if (
        target &&
        didTokenRotate(tokenMetadataBefore, {
          tokenIssuedAt: target.tokenIssuedAt,
          tokenExpiresAt: target.tokenExpiresAt,
        })
      ) {
        tokenFresh = true;
        break;
      }
    }
    disposed = true;
    unlisten?.();
    if (!tokenFresh) {
      fail('等待客户端刷新 token 超时：Token 未发生变化，请确认客户端已打开并登录该账号后重试');
      return;
    }

    // 4) 同步已轮换的凭证到 GitHub Secrets。
    update('syncing', '正在同步新凭证到 GitHub Secrets…');
    await useWorkCnStore.getState().syncGitHub(accountId);
    const result = useWorkCnStore.getState().githubSyncResultById[accountId];
    if (!result?.synced) {
      fail(`同步 GitHub 失败：${result?.error ?? result?.skipReason ?? '未知原因'}`);
      return;
    }

    // 5) 完成。
    update('done', 'Token 已轮换并同步 GitHub，可切回原账号');
  },

  resetRefreshFlow() {
    set({ refreshFlow: null });
  },
}));
