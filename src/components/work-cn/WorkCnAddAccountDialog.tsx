// TRAE Work CN — 导入当前账号对话框。
//
// 样式走 work-cn.css（设计系统 token，自动适配暗色主题）。

import { useState } from 'react';
import { useWorkCnStore } from '../../stores/useWorkCnStore';

interface Props {
  open: boolean;
  onClose: () => void;
}

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
    <div className="wc-overlay" onClick={onClose}>
      <div className="wc-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="wc-dialog-title">导入当前 TRAE Work CN 账号</h3>
        <p className="wc-dialog-note" style={{ fontSize: 13, marginTop: 0 }}>
          读取本机已登录的 TRAE Work CN 客户端账号，保存完整登录态（令牌 + 设备密钥 +
          设备 ID）。数据以 AES-256-GCM 加密存储，界面只显示脱敏信息，不会暴露令牌与私钥。
        </p>
        <label className="wc-row-label" style={{ marginTop: 0 }}>
          账号备注（可选）
        </label>
        <input
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder="如：work1 / personal（英文备注将作为 GitHub secret 名，如 work1_TOKEN）"
          className="wc-input"
        />
        {error ? <div className="wc-dialog-error">{error}</div> : null}
        <div className="wc-dialog-actions">
          <button type="button" className="wc-btn" onClick={onClose} disabled={importing}>
            取消
          </button>
          <button
            type="button"
            className="wc-btn wc-btn-primary"
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
