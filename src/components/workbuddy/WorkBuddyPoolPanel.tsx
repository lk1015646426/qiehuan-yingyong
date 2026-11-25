import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AccountPoolPanel } from '../AccountPoolPanel';

interface PoolStatus {
  running: boolean;
  stopping: boolean;
  lastError?: string | null;
  model: string;
  endpoint: string;
  requests: number;
  rotations: number;
  coolingAccounts: number;
  modelCount: number;
  catalogSource: string;
}

interface PoolModel {
  id: string;
  name: string;
}
interface ModelSelection {
  models: PoolModel[];
  selectedModelIds: string[];
  catalogSource: string;
  updatedAt: number;
}

export function WorkBuddyPoolPanel() {
  const [status, setStatus] = useState<PoolStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [catalog, setCatalog] = useState<ModelSelection | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [catalogBusy, setCatalogBusy] = useState(false);
  const dirty = JSON.stringify([...selected].sort()) !== JSON.stringify([...(catalog?.selectedModelIds ?? [])].sort());
  const refresh = async () => {
    try { setStatus(await invoke<PoolStatus>('get_workbuddy_pool_status')); }
    catch (cause) { setError(String(cause)); }
  };
  const loadModels = async (refreshCatalog = false) => {
    setCatalogBusy(true); setError('');
    try {
      const next = await invoke<ModelSelection>('get_workbuddy_pool_models', { refreshCatalog });
      setCatalog(next);
      if (!dirty) setSelected(next.selectedModelIds);
    } catch (cause) { setError(String(cause)); }
    finally { setCatalogBusy(false); }
  };
  useEffect(() => {
    void refresh();
    void loadModels();
    const timer = window.setInterval(() => { void refresh(); }, 5000);
    return () => window.clearInterval(timer);
  }, []);
  const toggle = async () => {
    setBusy(true); setError('');
    try { setStatus(await invoke<PoolStatus>(status?.running ? 'stop_workbuddy_pool' : 'start_workbuddy_pool')); }
    catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  const saveModels = async () => {
    setBusy(true); setError(''); setNotice('');
    try {
      const next = await invoke<ModelSelection>('save_workbuddy_pool_models', { modelIds: selected });
      setCatalog(next); setSelected(next.selectedModelIds);
      setNotice(next.selectedModelIds.length ? `已保存并启用 ${next.selectedModelIds.length} 个账号池模型。` : '已清空账号池模型，默认不会展示任何账号池模型。');
      await refresh();
      return true;
    } catch (cause) { setError(String(cause)); return false; }
    finally { setBusy(false); }
  };
  return <AccountPoolPanel
    app="WorkBuddy" running={status?.running} stopping={status?.stopping}
    busy={busy} loading={catalogBusy} ready={!!catalog}
    modelCount={catalog?.selectedModelIds.length ?? status?.modelCount ?? 0}
    models={catalog?.models ?? []} selected={selected} dirty={dirty}
    error={error || status?.lastError} notice={notice}
    metrics={[{ label: '累计请求', value: status?.requests ?? 0 },
      { label: '账号轮换', value: status?.rotations ?? 0 },
      { label: '冷却账号', value: status?.coolingAccounts ?? 0 },
      { label: '模型目录', value: status?.catalogSource === 'cache' ? '缓存目录' : '实时目录' }]}
    instructions={<><p>在 WorkBuddy 中选择「WorkBuddy 账号池 · 模型名称」。积分不足时自动尝试下一账号。使用期间保持切换应用运行，程序重启后自动恢复账号池。</p><p>各模型能力按官方目录保留；绘图等专用模型使用原客户端专用入口。列表未更新时，保存当前任务后重新打开 WorkBuddy。</p></>}
    onSelect={setSelected} onDiscard={() => setSelected(catalog?.selectedModelIds ?? [])}
    onToggle={() => void toggle()} onRefresh={() => void loadModels(true)} onSave={saveModels}
  />;
}
