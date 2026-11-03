import { useCallback, useEffect, useState } from 'react';
import { getTraeModelPoolModels, getTraeModelPoolStatus, saveTraeModelPoolModels, startTraeModelPool, stopTraeModelPool, registerTraeModelPoolModels } from '../../services/traePoolService';
import type { TraeModelPoolModels, TraeModelPoolStatus } from '../../types/traePool';

export function TraePoolPanel({ accountId }: { accountId: string | null }) {
  const [status, setStatus] = useState<TraeModelPoolStatus | null>(null);
  const [catalog, setCatalog] = useState<TraeModelPoolModels | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [search, setSearch] = useState('');
  const [confirmRegistration, setConfirmRegistration] = useState(false);
  const dirty = JSON.stringify([...selected].sort()) !== JSON.stringify([...(catalog?.selectedModelIds ?? [])].sort());
  const refresh = useCallback(async () => {
    try { setStatus(await getTraeModelPoolStatus()); }
    catch (cause) { setError(String(cause)); }
  }, []);
  const loadModels = async () => {
    setLoading(true); setError('');
    try { const next = await getTraeModelPoolModels(); setCatalog(next); if (!dirty) setSelected(next.selectedModelIds); }
    catch (cause) { setError(String(cause)); }
    finally { setLoading(false); }
  };
  useEffect(() => {
    void refresh(); void loadModels();
    const timer = window.setInterval(() => void refresh(), 5000);
    return () => window.clearInterval(timer);
  }, [refresh]);
  const save = async () => {
    setBusy(true); setError(''); setNotice('');
    try {
      const next = await saveTraeModelPoolModels(selected); setCatalog(next); setSelected(next.selectedModelIds);
      if (next.selectedModelIds.length) { setStatus(await startTraeModelPool()); setNotice('模型已保存，账号池已启动。请在 TRAE 中选择已配置的账号池模型。'); }
      else { setStatus(await stopTraeModelPool()); setNotice('已清空模型选择并停止账号池。'); }
    } catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  const toggle = async () => {
    setBusy(true); setError('');
    try { setStatus(await (status?.running ? stopTraeModelPool() : startTraeModelPool())); }
    catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  const register = async () => {
    if (!accountId) return;
    setBusy(true); setError(''); setNotice('');
    try {
      const result = await registerTraeModelPoolModels(accountId);
      setNotice(`已确认 ${result.registered.length + result.alreadyRegistered.length} 个模型注册到当前 TRAE 账号。请在 TRAE 的模型列表中选择「TRAE 账号池 · 模型名称」；列表未更新时重新打开模型选择器。`);
      setConfirmRegistration(false);
    } catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  const available = (catalog?.models ?? []).filter(m => !selected.includes(m.id) && `${m.name} ${m.id}`.toLowerCase().includes(search.trim().toLowerCase()));
  return <section className="wc-pool" aria-label="TRAE 连续对话账号池">
    <h2 className="wc-section-title">连续对话账号池</h2>
    <div className="wc-slot">
      <p>在 TRAE 中选择「TRAE 账号池 · 模型名称」后，原客户端继续保存当前对话并执行工具。账号额度不足时，账号池会携带完整历史尝试下一账号，保持所选模型。</p>
      <p className="wc-pool-hint">使用时保持切换应用运行。已开始输出的请求不会自动重发；中途断流会显示错误，避免重复生成或重复调用工具。</p>
      <p role="status">{status?.stopping ? '正在等待当前请求结束…' : status?.running ? `运行中 · ${status.requests} 次请求 · ${status.rotations} 次轮换 · ${status.activeRequests} 个进行中 · ${status.coolingAccounts} 个账号冷却中` : '账号池未启动'}</p>
      {error ? <div className="wc-pool-alert wc-pool-alert--error" role="alert">{error}</div> : null}
      {status?.lastError ? <div className="wc-pool-alert wc-pool-alert--error" role="alert">{status.lastError}</div> : null}
      {notice ? <div className="wc-pool-alert wc-pool-alert--ok" role="status">{notice}</div> : null}
      <details open>
        <summary>管理账号池模型（已选 {selected.length} 个）{dirty ? ' · 修改待保存' : ''}</summary>
        {selected.length ? <ul className="wb-pool-model-list" aria-label="已添加的 TRAE 账号池模型">
          {selected.map(id => <li key={id}><div><strong>{catalog?.models.find(m => m.id === id)?.name ?? id}</strong><small>{id}</small></div>
            <button type="button" className="wc-btn" disabled={busy || loading} aria-label={`移除模型 ${id}`} onClick={() => setSelected(ids => ids.filter(value => value !== id))}>移除</button></li>)}
        </ul> : <p>尚未添加模型，请从官方目录按需选择。</p>}
        <label className="wb-pool-search">搜索官方模型目录<input className="wc-input" type="search" value={search} onChange={e => setSearch(e.target.value)} placeholder="模型名称或 ID" /></label>
        {loading ? <p role="status">正在读取当前账号的官方模型目录…</p> : null}
        <ul className="wb-pool-model-list wb-pool-model-list--catalog" aria-label="可添加的 TRAE 模型">
          {available.map(model => <li key={model.id}><div><strong>{model.name}</strong><small>{model.id} · {model.multimodal ? '支持图片' : '文本模型'}{model.tools === false ? ' · 不支持工具' : ''}</small></div>
            <button type="button" className="wc-btn" disabled={busy || loading} aria-label={`添加模型 ${model.name}`} onClick={() => setSelected(ids => ids.includes(model.id) ? ids : [...ids, model.id])}>添加</button></li>)}
        </ul>
        {catalog && !available.length && !loading ? <p>没有匹配的可添加模型。</p> : null}
      </details>
      <div className="wc-slot-actions">
        <button type="button" className="wc-btn wc-btn-primary" disabled={busy || loading || !catalog || status?.stopping} onClick={() => void save()}>{busy ? '处理中…' : '保存模型并启动'}</button>
        <button type="button" className="wc-btn" disabled={busy || loading || !status || status.stopping || (!status.running && (!selected.length || dirty))} onClick={() => void toggle()}>{status?.running ? '停止账号池' : '启动账号池'}</button>
        <button type="button" className="wc-btn" disabled={busy || loading} onClick={() => void loadModels()}>刷新官方目录</button>
        <button type="button" className="wc-btn" disabled={busy} onClick={() => void refresh()}>刷新状态</button>
        <button type="button" className="wc-btn" disabled={busy || loading || !accountId || !status?.running || status.stopping || dirty || !selected.length} onClick={() => setConfirmRegistration(true)}>注册到当前 TRAE 账号</button>
      </div>
      {confirmRegistration ? <div className="wc-pool-alert" role="group" aria-label="确认 TRAE 模型注册">
        <p>将向当前 TRAE 账号添加或更新所选的 {selected.length} 个账号池模型，地址指向本机账号池。TRAE 会保存本地连接密钥；其他账号的登录令牌不会写入模型配置。</p>
        <button type="button" className="wc-btn wc-btn-primary" disabled={busy} onClick={() => void register()}>确认注册</button>
        <button type="button" className="wc-btn" disabled={busy} onClick={() => setConfirmRegistration(false)}>取消</button>
      </div> : null}
    </div>
  </section>;
}
