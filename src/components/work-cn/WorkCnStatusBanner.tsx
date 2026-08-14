// TRAE Work CN — 后台会话监测状态横幅（阶段 7）。
//
// 无 props，从 store 读取 `sessionWatchStatus`；按 outcome + GitHub 同步结果
// 映射为灰/绿/黄/红四态。只展示脱敏信息，绝不展示 token。

import type { CSSProperties } from 'react';
import { useWorkCnStore } from '../../stores/useWorkCnStore';
import type { WorkCnSessionWatchStatus } from '../../types/workCn';

const bannerStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  padding: '10px 14px',
  background: '#ffffff',
  border: '1px solid #e4e7ec',
  borderRadius: 8,
  fontSize: 13,
  color: '#525866',
};

const dotStyle: CSSProperties = {
  width: 8,
  height: 8,
  borderRadius: '50%',
  background: '#c4c8d0',
  flexShrink: 0,
};

const dotGreenStyle: CSSProperties = { ...dotStyle, background: '#22c55e' };
const dotYellowStyle: CSSProperties = { ...dotStyle, background: '#f59e0b' };
const dotRedStyle: CSSProperties = { ...dotStyle, background: '#ef4444' };

type Tone = 'gray' | 'green' | 'yellow' | 'red';

function resolveBanner(
  status: WorkCnSessionWatchStatus | null,
): { tone: Tone; dot: CSSProperties; line: string } {
  const gray = { tone: 'gray' as Tone, dot: dotStyle, line: '后台会话监测未启动' };

  if (!status || !status.running || status.outcome === 'IDLE') {
    return gray;
  }

  switch (status.outcome) {
    case 'TOKEN_UPDATED':
      if (status.githubError) {
        return { tone: 'red', dot: dotRedStyle, line: status.message };
      }
      if (status.githubSkipped) {
        return { tone: 'yellow', dot: dotYellowStyle, line: status.message };
      }
      return {
        tone: 'green',
        dot: dotGreenStyle,
        line: status.message || '检测到 Token 更新，账号库与 GitHub 已同步',
      };
    case 'FAILED':
      return {
        tone: 'red',
        dot: dotRedStyle,
        line: status.message || '后台监测失败，已进入退避',
      };
    case 'NO_MATCH':
      return { tone: 'gray', dot: dotStyle, line: '当前客户端账号不在账号库中，可先导入' };
    case 'NO_STORAGE':
      return { tone: 'gray', dot: dotStyle, line: '未检测到已登录的 TRAE Work CN 会话' };
    case 'UNCHANGED':
    case 'NO_CHANGE':
      return { tone: 'gray', dot: dotStyle, line: '后台监测中 · 暂无 token 变化' };
    case 'SWITCH_BUSY':
      return { tone: 'gray', dot: dotStyle, line: '正在切换账号，后台监测暂缓' };
    default:
      return gray;
  }
}

const toneStyle: Record<Tone, CSSProperties> = {
  gray: {},
  green: { borderColor: '#bbf7d0', color: '#15803d', background: '#f0fdf4' },
  yellow: { borderColor: '#fde68a', color: '#b45309', background: '#fffbeb' },
  red: { borderColor: '#fecaca', color: '#b91c1c', background: '#fef2f2' },
};

export function WorkCnStatusBanner() {
  const sessionWatchStatus = useWorkCnStore((s) => s.sessionWatchStatus);
  const { tone, dot, line } = resolveBanner(sessionWatchStatus);

  return (
    <section style={{ ...bannerStyle, ...toneStyle[tone] }}>
      <span style={dot} />
      <div style={{ flex: 1, wordBreak: 'break-all' }}>{line}</div>
    </section>
  );
}
