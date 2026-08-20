import { useState } from 'react';
import { useWorkBuddyStore } from '../../stores/useWorkBuddyStore';

export function WorkBuddyAddAccountDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [displayName, setDisplayName] = useState('');
  const importing = useWorkBuddyStore((state) => state.importing);
  const importCurrent = useWorkBuddyStore((state) => state.importCurrent);
  const error = useWorkBuddyStore((state) => state.error);
  if (!open) return null;
  const submit = async () => {
    if (await importCurrent(displayName.trim() || null)) {
      setDisplayName('');
      onClose();
    }
  };
  return <div className="wc-overlay" onClick={onClose}>
    <section className="wc-dialog" onClick={(event) => event.stopPropagation()}>
      <h3 className="wc-dialog-title">导入当前 WorkBuddy 账号</h3>
      <p className="wc-dialog-note">请先在官方 WorkBuddy 客户端完成登录。工具会加密保存完整认证快照，界面不会显示令牌或完整手机号。</p>
      <label className="wc-row-label">账号备注（可选）</label>
      <input className="wc-input" value={displayName} maxLength={80} onChange={(event) => setDisplayName(event.target.value)} placeholder="例如：个人号" />
      {error ? <div className="wc-dialog-error">{error}</div> : null}
      <div className="wc-dialog-actions">
        <button type="button" className="wc-btn" disabled={importing} onClick={onClose}>取消</button>
        <button type="button" className="wc-btn wc-btn-primary" disabled={importing} onClick={() => void submit()}>{importing ? '导入中…' : '导入当前账号'}</button>
      </div>
    </section>
  </div>;
}
