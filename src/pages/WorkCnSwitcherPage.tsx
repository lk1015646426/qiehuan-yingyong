// TRAE Work CN 账号切换器 - 主页面
//
// 阶段 1：渲染静态应用壳框架。
// 阶段 2：调用 get_work_cn_installation 在状态栏显示客户端检测结果。
// 账号导入、一键切换、积分查询等能力将在后续阶段接入上游既有 Tauri 命令。

import { useEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import { getWorkCnInstallation } from '../services/workCnService';
import type { WorkCnInstallation } from '../types/workCn';

const ACCOUNT_SLOT_COUNT = 4;

interface AccountSlot {
  index: number;
  label: string;
}

const EMPTY_SLOTS: AccountSlot[] = Array.from(
  { length: ACCOUNT_SLOT_COUNT },
  (_, index) => ({ index, label: `账号 ${index + 1}` }),
);

const pageStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  minWidth: 0,
  minHeight: '100vh',
  padding: '20px 24px',
  boxSizing: 'border-box',
  background: '#f5f6f8',
  color: '#1f2430',
  fontFamily:
    "'Segoe UI', 'Microsoft YaHei', 'PingFang SC', system-ui, sans-serif",
  gap: 16,
};

const headerStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  gap: 12,
};

const titleStyle: CSSProperties = {
  margin: 0,
  fontSize: 20,
  fontWeight: 600,
  letterSpacing: 0.2,
};

const headerActionsStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
};

const buttonStyle: CSSProperties = {
  height: 32,
  padding: '0 14px',
  fontSize: 13,
  color: '#1f2430',
  background: '#ffffff',
  border: '1px solid #d7dae0',
  borderRadius: 6,
  cursor: 'pointer',
};

const statusStyle: CSSProperties = {
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

const statusDotStyle: CSSProperties = {
  width: 8,
  height: 8,
  borderRadius: '50%',
  background: '#c4c8d0',
  flexShrink: 0,
};

const slotsStyle: CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
  gap: 12,
};

const slotStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 6,
  padding: '16px 18px',
  background: '#ffffff',
  border: '1px solid #e4e7ec',
  borderRadius: 10,
  minHeight: 96,
};

const slotTitleStyle: CSSProperties = {
  fontSize: 14,
  fontWeight: 600,
  color: '#1f2430',
};

const slotStatusStyle: CSSProperties = {
  fontSize: 12,
  color: '#8a909c',
};

const statusDetailStyle: CSSProperties = {
  fontSize: 12,
  color: '#8a909c',
  marginTop: 4,
  wordBreak: 'break-all',
};

const statusDotOkStyle: CSSProperties = {
  ...statusDotStyle,
  background: '#22c55e',
};

const statusDotErrorStyle: CSSProperties = {
  ...statusDotStyle,
  background: '#ef4444',
};

function renderInstallationStatus(
  installation: WorkCnInstallation | null,
  loading: boolean,
  error: string | null,
): { dot: CSSProperties; line: string; detail?: string } {
  if (loading) {
    return { dot: statusDotStyle, line: '客户端检测：正在检测 TRAE Work CN 安装…' };
  }
  if (error) {
    return {
      dot: statusDotErrorStyle,
      line: '客户端检测失败，请稍后重试或在设置中手动指定路径',
      detail: error,
    };
  }
  if (!installation || !installation.installed) {
    const legacyNote = installation?.legacyPath ? '（兼容旧数据目录）' : '';
    const detail = installation?.userDataDir
      ? `数据目录：${installation.userDataDir}${legacyNote}`
      : undefined;
    return {
      dot: statusDotErrorStyle,
      line: '未检测到 TRAE Work CN，请在设置中选择 EXE 路径',
      detail,
    };
  }
  const versionText = installation.version ? ` ${installation.version}` : '';
  const dataDirText = installation.userDataDir ?? '';
  const legacyNote = installation.legacyPath
    ? '（兼容旧数据目录）'
    : '';
  return {
    dot: statusDotOkStyle,
    line: `已检测到 TRAE Work CN${versionText}`,
    detail: [installation.executablePath, dataDirText ? `数据目录：${dataDirText}${legacyNote}` : '']
      .filter(Boolean)
      .join('\n'),
  };
}

export function WorkCnSwitcherPage() {
  const [installation, setInstallation] = useState<WorkCnInstallation | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    getWorkCnInstallation()
      .then((result) => {
        if (!cancelled) {
          setInstallation(result);
          setError(null);
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const status = renderInstallationStatus(installation, loading, error);

  return (
    <div style={pageStyle}>
      <header style={headerStyle}>
        <h1 style={titleStyle}>TRAE Work CN 账号切换器</h1>
        <div style={headerActionsStyle}>
          <button type="button" style={buttonStyle}>
            设置
          </button>
          <button type="button" style={buttonStyle}>
            日志
          </button>
        </div>
      </header>

      <section style={statusStyle}>
        <span style={status.dot} />
        <div>
          <div>{status.line}</div>
          {status.detail ? (
            <div style={statusDetailStyle}>
              {status.detail.split('\n').map((line, index) => (
                <div key={index}>{line}</div>
              ))}
            </div>
          ) : null}
        </div>
      </section>

      <section style={slotsStyle}>
        {EMPTY_SLOTS.map((slot) => (
          <div key={slot.index} style={slotStyle}>
            <div style={slotTitleStyle}>{slot.label}</div>
            <div style={slotStatusStyle}>空槽位 · 可导入</div>
          </div>
        ))}
      </section>
    </div>
  );
}
