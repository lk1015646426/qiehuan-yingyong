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
      if (next.selectedModelIds.length) { setStatus(await startTraeModelPool()); setNotice('已保存并启用账号池模型。请在 TRAE 中选择「TRAE 账号池 · 模型名称」。'); }
      else { setStatus(await stopTraeModelPool()); setNotice('已清空账号池模型，默认不会展示任何账号池模型。'); }
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
  const selectedSet = new Set(selected);
  const available = (catalog?.models ?? []).filter(model => !selectedSet.has(model.id)
    && `${model.name} ${model.id}`.toLowerCase().includes(search.toLowerCase().trim()));
  return <section aria-label="TRAE 连续对话账号池">
    <h2 className="wc-section-title">连续对话账号池</h2>
    <div className="wc-slot">
      <p>只展示你添加的模型，首次默认不选。保存后自动启用账号池，程序重启后自动恢复。随后，在原客户端选择「TRAE 账号池 · 模型名称」，账号额度不足时自动携带完整历史尝试下一账号，保持所选模型。</p>
      <p>已开始输出的请求不会自动重发；中途断流会显示错误，避免重复生成或重复调用工具。模型列表未更新时，请在保存当前任务后重新打开 TRAE。使用期间保持切换应用运行。</p>
      <p role="status">{status?.stopping ? '正在停止：等待当前请求结束并释放端口…' : status?.running ? `运行中 · ${status.selectedModelIds.length} 个会话模型 · ${status.requests} 次请求 · ${status.rotations} 次轮换 · ${status.activeRequests} 个进行中 · ${status.coolingAccounts} 个账号冷却中` : '账号池未启动'}</p>
      {status?.lastError ? <p className="wc-status wc-status--error" role="alert">{status.lastError}</p> : null}
      {error ? <p className="wc-status wc-status--error" role="alert">{error}</p> : null}
      {notice ? <p className="wc-status wc-status--ok" role="status">{notice}</p> : null}
      <details className="wb-pool-model-manager" open>
        <summary>管理账号池模型（已选 {selected.length} 个）{dirty ? ' · 有修改待保存' : ''}</summary>
        {catalog ? <>
          <div className="wb-pool-model-heading"><strong>已添加的模型</strong>
            <button type="button" className="wc-btn" disabled={busy || loading || !selected.length} onClick={() => setSelected([])}>清空选择</button></div>
          {!selected.length ? <p className="wb-pool-muted">尚未添加模型，请从下方目录按需添加。</p> : <ul className="wb-pool-model-list" aria-label="已添加的 TRAE 账号池模型">
            {selected.map(id => <li key={id}><div><strong>{catalog.models.find(model => model.id === id)?.name ?? '目录暂不可用'}</strong><small>{id}</small></div>
              <button type="button" className="wc-btn" disabled={busy || loading} aria-label={`移除模型 ${id}`} onClick={() => setSelected(value => value.filter(item => item !== id))}>移除</button></li>)}
          </ul>}
          <label className="wb-pool-search">从官方目录添加模型
            <input type="search" className="wc-input" value={search} onChange={event => setSearch(event.target.value)} placeholder="搜索模型名称或 ID" /></label>
          <ul className="wb-pool-model-list wb-pool-model-list--catalog" aria-label="可添加的 TRAE 模型">
            {available.map(model => <li key={model.id}><div><strong>{model.name}</strong><small>{model.id} · {model.multimodal ? '支持图片' : '文本模型'}{model.tools === false ? ' · 不支持工具' : ''}</small></div>
              <button type="button" className="wc-btn" disabled={busy || loading} aria-label={`添加模型 ${model.name}`} onClick={() => setSelected(value => value.includes(model.id) ? value : [...value, model.id])}>添加</button></li>)}
          </ul>
          {!available.length ? <p className="wb-pool-muted">没有匹配的可添加模型。</p> : null}
          <p className="wb-pool-muted">保存只更新账号池模型列表；TRAE 原有官方模型和自定义模型保留。</p>
        </> : <p role="status">{loading ? '正在读取当前账号的官方模型目录…' : '模型目录暂不可用，请刷新目录重试。'}</p>}
      </details>
      <div className="wc-slot-actions">
        <button type="button" className="wc-btn wc-btn-primary" disabled={busy || loading || status?.stopping || !catalog} onClick={() => void save()}>{busy ? '处理中…' : '保存并启用模型'}</button>
        <button type="button" className="wc-btn" disabled={busy || loading || status?.stopping || !status || (!status.running && (dirty || !selected.length))} onClick={() => void toggle()}>{status?.stopping ? '正在停止…' : status?.running ? '停止账号池' : '启动账号池'}</button>
        <button type="button" className="wc-btn" disabled={busy} onClick={() => void refresh()}>刷新状态</button>
        <button type="button" className="wc-btn" disabled={busy || loading} onClick={() => void loadModels()}>{loading ? '正在读取目录…' : '刷新官方目录'}</button>
        <button type="button" className="wc-btn" disabled={busy || loading || !accountId || !status?.running || status.stopping || dirty || !selected.length} onClick={() => setConfirmRegistration(true)}>注册到当前 TRAE 账号</button>
      </div>
      {confirmRegistration ? <div role="group" aria-label="确认 TRAE 模型注册">
        <p className="wb-pool-muted">将向当前 TRAE 账号添加或更新所选的 {selected.length} 个账号池模型，地址指向本机账号池。TRAE 会保存本地连接密钥；其他账号的登录令牌不会写入模型配置。</p>
        <div className="wc-slot-actions">
          <button type="button" className="wc-btn wc-btn-primary" disabled={busy} onClick={() => void register()}>{busy ? '正在注册…' : '确认注册'}</button>
          <button type="button" className="wc-btn" disabled={busy} onClick={() => setConfirmRegistration(false)}>取消</button>
        </div>
      </div> : null}
    </div>
  </section>;
}
