import { useState } from 'react';
import { useZhipuStore } from '../../stores/useZhipuStore';

export function ZhipuAddAccountDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [manualOpen, setManualOpen] = useState(false);
  const [accessToken, setAccessToken] = useState('');
  const [refreshToken, setRefreshToken] = useState('');
  const [displayName, setDisplayName] = useState('');
  const importing = useZhipuStore((state) => state.importing);
  const importCurrent = useZhipuStore((state) => state.importCurrent);
  const importManual = useZhipuStore((state) => state.importManual);
  const error = useZhipuStore((state) => state.error);
  if (!open) return null;
  const submitCurrent = async () => {
    if (await importCurrent(displayName.trim() || null)) {
      setDisplayName('');
      onClose();
    }
  };
  const submitManual = async () => {
    if (!accessToken.trim()) return;
    if (await importManual(accessToken.trim(), refreshToken.trim() || null, displayName.trim() || null)) {
      setAccessToken('');
      setRefreshToken('');
      setDisplayName('');
      onClose();
    }
  };
  return <div className="wc-overlay" onClick={onClose}>
    <section className="wc-dialog" onClick={(event) => event.stopPropagation()}>
      <h3 className="wc-dialog-title">添加智谱清言账号</h3>
      <p className="wc-dialog-note">推荐直接从本机智谱清言客户端导入：客户端每次启动会自动续期登录态，工具读取最新 token 并加密保存（AES-256-GCM），界面不显示 token 原文。</p>
      <label className="wc-row-label">账号备注（可选）</label>
      <input className="wc-input" value={displayName} maxLength={80} onChange={(event) => setDisplayName(event.target.value)} placeholder="例如：主力号" />
      {error ? <div className="wc-dialog-error">{error}</div> : null}
      <div className="wc-dialog-actions">
        <button type="button" className="wc-btn" disabled={importing} onClick={onClose}>取消</button>
        <button type="button" className="wc-btn wc-btn-primary" disabled={importing} onClick={() => void submitCurrent()}>{importing ? '验证并导入中…' : '导入当前客户端账号'}</button>
      </div>
      {manualOpen ? <div className="zp-manual-import">
        <label className="wc-row-label">chatglm_token（必填，JWT 三段式）</label>
        <input className="wc-input" value={accessToken} maxLength={2000} spellCheck={false} autoComplete="off" onChange={(event) => setAccessToken(event.target.value)} placeholder="浏览器 chatglm.cn Cookie 中的 chatglm_token" />
        <label className="wc-row-label">chatglm_refresh_token（可选）</label>
        <input className="wc-input" value={refreshToken} maxLength={2000} spellCheck={false} autoComplete="off" onChange={(event) => setRefreshToken(event.target.value)} placeholder="Cookie 中的 chatglm_refresh_token" />
        <div className="wc-dialog-actions">
          <button type="button" className="wc-btn" disabled={importing} onClick={() => setManualOpen(false)}>收起</button>
          <button type="button" className="wc-btn wc-btn-primary" disabled={importing || !accessToken.trim()} onClick={() => void submitManual()}>{importing ? '验证并导入中…' : '手动导入'}</button>
        </div>
      </div> : <button type="button" className="zp-manual-toggle" onClick={() => setManualOpen(true)}>客户端不可用？手动粘贴 token</button>}
    </section>
  </div>;
}
