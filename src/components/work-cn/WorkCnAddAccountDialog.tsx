import { useState } from 'react';
import type { CSSProperties } from 'react';
import { useWorkCnStore } from '../../stores/useWorkCnStore';

interface Props {
  open: boolean;
  onClose: () => void;
}

const overlayStyle: CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'rgba(15, 23, 42, 0.35)',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 1000,
};

const dialogStyle: CSSProperties = {
  width: 440,
  maxWidth: '90vw',
  background: '#ffffff',
  borderRadius: 12,
  padding: '22px 24px',
  boxShadow: '0 12px 40px rgba(15, 23, 42, 0.25)',
  boxSizing: 'border-box',
};

const inputStyle: CSSProperties = {
  width: '100%',
  height: 34,
  padding: '0 10px',
  fontSize: 13,
  border: '1px solid #d7dae0',
  borderRadius: 6,
  boxSizing: 'border-box',
  marginBottom: 4,
};

const actionsStyle: CSSProperties = {
  display: 'flex',
  justifyContent: 'flex-end',
  gap: 8,
  marginTop: 16,
};

const buttonStyle: CSSProperties = {
  height: 34,
  padding: '0 16px',
  fontSize: 13,
  color: '#1f2430',
  background: '#f1f3f6',
  border: '1px solid #d7dae0',
  borderRadius: 6,
  cursor: 'pointer',
};

const primaryStyle: CSSProperties = {
  color: '#ffffff',
  background: '#2563eb',
  border: '1px solid #2563eb',
};

const errorStyle: CSSProperties = {
  marginTop: 12,
  fontSize: 12,
  color: '#ef4444',
  background: '#fef2f2',
  border: '1px solid #fecaca',
  borderRadius: 6,
  padding: '8px 10px',
};

export function WorkCnAddAccountDialog({ open, onClose }: Props) {
  const importCurrent = useWorkCnStore((s) => s.importCurrent);
  const importing = useWorkCnStore((s) => s.importing);
  const error = useWorkCnStore((s) => s.error);
  const [label, setLabel] = useState('');

  if (!open) {
    return null;
  }

  const handleImport = async () => {
    await importCurrent(label.trim() || null);
    setLabel('');
    onClose();
  };

  return (
    <div style={overlayStyle} onClick={onClose}>
      <div style={dialogStyle} onClick={(e) => e.stopPropagation()}>
        <h3 style={{ margin: '0 0 12px', fontSize: 16 }}>导入当前 TRAE Work CN 账号</h3>
        <p style={{ fontSize: 13, color: '#525866', marginTop: 0, lineHeight: 1.6 }}>
          读取本机已登录的 TRAE Work CN 客户端账号，保存完整登录态（令牌 + 设备密钥 +
          设备 ID）。数据以 AES-256-GCM 加密存储，界面只显示脱敏信息，不会暴露令牌与私钥。
        </p>
        <label style={{ fontSize: 13, color: '#1f2430', display: 'block', marginBottom: 6 }}>
          账号备注（可选）
        </label>
        <input
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder="如：工作号 / 个人号"
          style={inputStyle}
        />
        {error ? <div style={errorStyle}>{error}</div> : null}
        <div style={actionsStyle}>
          <button type="button" style={buttonStyle} onClick={onClose} disabled={importing}>
            取消
          </button>
          <button
            type="button"
            style={{ ...buttonStyle, ...primaryStyle }}
            onClick={handleImport}
            disabled={importing}
          >
            {importing ? '导入中…' : '导入当前账号'}
          </button>
        </div>
      </div>
    </div>
  );
}
