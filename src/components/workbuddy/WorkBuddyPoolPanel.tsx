import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

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
  const [search, setSearch] = useState('');
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
    } catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  const selectedSet = new Set(selected);
  const available = (catalog?.models ?? []).filter(model => !selectedSet.has(model.id)
    && `${model.name} ${model.id}`.toLowerCase().includes(search.toLowerCase().trim()));
  return <section aria-label="WorkBuddy 多账号连续使用">
    <h2 className="wc-section-title">多账号连续使用</h2>
    <div className="wc-slot">
      <p>只展示你添加的模型，首次默认不选。保存后自动启用账号池，程序重启后自动恢复。随后，在原客户端选择「WorkBuddy 账号池 · 模型名称」，积分不足时自动尝试下一账号。</p>
      <p>各模型的视觉、思考和工具能力按实时目录保留。绘图等专用模型继续使用原客户端专用入口。模型列表未更新时，请在保存当前任务后重新打开 WorkBuddy。使用期间保持切换应用运行。</p>
      <p role="status">{status?.stopping ? '正在停止：等待当前请求结束并释放端口…' : status?.running ? `运行中 · ${status.modelCount} 个会话模型 · ${status.catalogSource === 'cache' ? '缓存目录' : '实时目录'} · ${status.requests} 次请求 · ${status.rotations} 次轮换 · ${status.coolingAccounts} 个账号冷却中` : '账号池未启动'}</p>
      {status?.lastError ? <p className="wc-status wc-status--error" role="alert">{status.lastError}</p> : null}
      {error ? <p className="wc-status wc-status--error" role="alert">{error}</p> : null}
      {notice ? <p className="wc-status wc-status--ok" role="status">{notice}</p> : null}
      <details className="wb-pool-model-manager" open>
        <summary>管理账号池模型（已选 {selected.length} 个）{dirty ? ' · 有修改待保存' : ''}</summary>
        {catalog ? <>
          <div className="wb-pool-model-heading"><strong>已添加的模型</strong>
            <button type="button" className="wc-btn" disabled={busy || catalogBusy || !selected.length} onClick={() => setSelected([])}>清空选择</button></div>
          {!selected.length ? <p className="wb-pool-muted">尚未添加模型，请从下方目录按需添加。</p> : <ul className="wb-pool-model-list" aria-label="已添加的账号池模型">
            {selected.map(id => <li key={id}><div><strong>{catalog.models.find(model => model.id === id)?.name ?? '目录暂不可用'}</strong><small>{id}</small></div>
              <button type="button" className="wc-btn" disabled={busy || catalogBusy} aria-label={`移除模型 ${id}`} onClick={() => setSelected(value => value.filter(item => item !== id))}>移除</button></li>)}
          </ul>}
          <label className="wb-pool-search">从官方目录添加模型
            <input type="search" className="wc-input" value={search} onChange={event => setSearch(event.target.value)} placeholder="搜索模型名称或 ID" /></label>
          <ul className="wb-pool-model-list wb-pool-model-list--catalog" aria-label="可添加的账号池模型">
            {available.map(model => <li key={model.id}><div><strong>{model.name}</strong><small>{model.id}</small></div>
              <button type="button" className="wc-btn" disabled={busy || catalogBusy} aria-label={`添加模型 ${model.id}`} onClick={() => setSelected(value => value.includes(model.id) ? value : [...value, model.id])}>添加</button></li>)}
          </ul>
          {!available.length ? <p className="wb-pool-muted">没有匹配的可添加模型。</p> : null}
          <p className="wb-pool-muted">{catalog.catalogSource === 'cache' ? '使用缓存目录' : '使用官方实时目录'}。保存只更新账号池模型列表；原有官方模型和自定义模型保留。</p>
        </> : <p role="status">{catalogBusy ? '正在读取模型目录…' : '模型目录暂不可用，请刷新目录重试。'}</p>}
      </details>
      <div className="wc-slot-actions">
        <button type="button" className="wc-btn wc-btn-primary" disabled={busy || catalogBusy || status?.stopping || !catalog} onClick={() => void saveModels()}>{busy ? '处理中…' : '保存并启用模型'}</button>
        <button type="button" className="wc-btn" disabled={busy || catalogBusy || status?.stopping || !status || (!status.running && (dirty || !selected.length))} onClick={() => void toggle()}>{status?.stopping ? '正在停止…' : status?.running ? '停止账号池' : '启动账号池'}</button>
        <button type="button" className="wc-btn" disabled={busy} onClick={() => void refresh()}>刷新状态</button>
        <button type="button" className="wc-btn" disabled={busy || catalogBusy} onClick={() => void loadModels(true)}>{catalogBusy ? '正在读取目录…' : '刷新官方目录'}</button>
      </div>
    </div>
  </section>;
}
