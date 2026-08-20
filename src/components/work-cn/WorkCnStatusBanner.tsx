// TRAE Work CN — 后台会话监测状态横幅（阶段 7）。
//
// 无 props，从 store 读取 `sessionWatchStatus`；按 outcome + GitHub 同步结果
// 映射为灰/绿/黄/红四态。只展示脱敏信息，绝不展示 token。
// 样式走 work-cn.css（设计系统 token，自动适配暗色主题）。

import { useWorkCnStore } from '../../stores/useWorkCnStore';
import type { WorkCnSessionWatchStatus } from '../../types/workCn';

type Tone = 'gray' | 'green' | 'yellow' | 'red';

const TONE_CLASS: Record<Tone, string> = {
  gray: '',
  green: ' wc-status--ok',
  yellow: ' wc-status--warn',
  red: ' wc-status--error',
};

const DOT_CLASS: Record<Tone, string> = {
  gray: '',
  green: ' wc-status-dot--ok',
  yellow: '',
  red: ' wc-status-dot--error',
};

function resolveBanner(
  status: WorkCnSessionWatchStatus | null,
): { tone: Tone; line: string } {
  const gray = { tone: 'gray' as Tone, line: '后台会话监测未启动' };

  if (!status || !status.running || status.outcome === 'IDLE') {
    return gray;
  }

  switch (status.outcome) {
    case 'TOKEN_UPDATED':
      if (status.githubError) {
        return { tone: 'red', line: status.message };
      }
      if (status.githubSkipped) {
        return { tone: 'yellow', line: status.message };
      }
      return {
        tone: 'green',
        line: status.message || '检测到 Token 更新，账号库与 GitHub 已同步',
      };
    case 'FAILED':
      return {
        tone: 'red',
        line: status.message || '后台监测失败，已进入退避',
      };
    case 'NO_MATCH':
      return { tone: 'gray', line: '当前客户端账号不在账号库中，可先导入' };
    case 'NO_STORAGE':
      return { tone: 'gray', line: '未检测到已登录的 TRAE Work CN 会话' };
    case 'UNCHANGED':
    case 'NO_CHANGE':
      return { tone: 'gray', line: '后台监测中 · 暂无 token 变化' };
    case 'SWITCH_BUSY':
      return { tone: 'gray', line: '正在切换账号，后台监测暂缓' };
    default:
      return gray;
  }
}

export function WorkCnStatusBanner() {
  const sessionWatchStatus = useWorkCnStore((s) => s.sessionWatchStatus);
  const { tone, line } = resolveBanner(sessionWatchStatus);

  return (
    <section className={`wc-status${TONE_CLASS[tone]}`}>
      <span className={`wc-status-dot${DOT_CLASS[tone]}`} />
      <div style={{ flex: 1, wordBreak: 'break-all' }}>{line}</div>
    </section>
  );
}
