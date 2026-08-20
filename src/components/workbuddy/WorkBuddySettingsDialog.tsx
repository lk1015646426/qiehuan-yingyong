import { useEffect, useState } from 'react';
import { getWorkBuddySettings, saveWorkBuddySettings } from '../../services/workBuddyService';
import type { WorkBuddySettings } from '../../types/workbuddy';

export function WorkBuddySettingsDialog({ open, onClose, onSaved }: { open: boolean; onClose: () => void; onSaved: () => void }) {
  const [settings, setSettings] = useState<WorkBuddySettings>({ executablePath: null, authFilePath: null });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    void getWorkBuddySettings().then(setSettings).catch((value: unknown) => setError(value instanceof Error ? value.message : String(value)));
  }, [open]);
  if (!open) return null;
  const save = async () => {
    setSaving(true); setError(null);
    try {
      await saveWorkBuddySettings({
        executablePath: settings.executablePath?.trim() || null,
        authFilePath: settings.authFilePath?.trim() || null,
      });
      onSaved(); onClose();
    } catch (value) { setError(value instanceof Error ? value.message : String(value)); }
    finally { setSaving(false); }
  };
  return <div className="wc-overlay" onClick={onClose}>
    <section className="wc-dialog wb-settings-dialog" onClick={(event) => event.stopPropagation()}>
      <h3 className="wc-dialog-title">WorkBuddy 路径设置</h3>
      <p className="wc-dialog-note">留空时自动检测。认证文件路径会同时用于导入、切换与后台监测。</p>
      <label className="wc-row-label">WorkBuddy EXE 路径</label>
      <input className="wc-input" value={settings.executablePath ?? ''} onChange={(event) => setSettings((value) => ({ ...value, executablePath: event.target.value }))} placeholder="例如：D:\\Apps\\WorkBuddy\\WorkBuddy.exe" />
      <label className="wc-row-label">认证文件路径</label>
      <input className="wc-input" value={settings.authFilePath ?? ''} onChange={(event) => setSettings((value) => ({ ...value, authFilePath: event.target.value }))} placeholder="默认：%LOCALAPPDATA%\\CodeBuddyExtension\\...\\workbuddy-desktop.info" />
      {error ? <div className="wc-dialog-error">{error}</div> : null}
      <div className="wc-dialog-actions"><button type="button" className="wc-btn" disabled={saving} onClick={onClose}>取消</button><button type="button" className="wc-btn wc-btn-primary" disabled={saving} onClick={() => void save()}>{saving ? '保存中…' : '保存路径'}</button></div>
    </section>
  </div>;
}
