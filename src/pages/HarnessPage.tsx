import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Cable, RefreshCw } from 'lucide-react';
import './harness.css';

interface Status {
  pool: { running: boolean; stopping: boolean; lastError?: string; requests: number; rotations: number; selectedModelIds?: string[] };
  harness: { detected: boolean; version?: string; configPath: string; configured: boolean; configuredPool?: string; configuredPools?: string[] };
  baseUrl: string;
  recordSummary?: { recordCount: number; nonstreamSuccess: number; streamRelayOk: number; limited: number; entryRejected: number; failed: number; timingSamples: number; meanRelayMs?: number | null; p95RelayMs?: number | null };
  records: { time: string; model: string; stream: boolean; status: number; result: string; relayMs?: number | null }[];
}
interface Catalog { models: { id: string; name: string }[]; selectedModelIds: string[]; catalogSource: string }

export function HarnessPage() {
  const [pool, setPool] = useState<'workbuddy' | 'trae' | 'doubao'>('trae');
  return <><div className="harness-pool-selector"><label htmlFor="harness-pool">账号池</label><select id="harness-pool" value={pool} onChange={e => setPool(e.target.value as 'workbuddy' | 'trae' | 'doubao')}><option value="trae">TRAE</option><option value="workbuddy">WorkBuddy</option><option value="doubao">豆包（Volcengine Ark）</option></select></div><HarnessPoolPage key={pool} pool={pool} /></>;
}
function HarnessPoolPage({ pool }: { pool: 'workbuddy' | 'trae' | 'doubao' }) {
  const [status, setStatus] = useState<Status>();
  const [catalog, setCatalog] = useState<Catalog>();
  const [model, setModel] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [test, setTest] = useState('');
  async function refresh(online = false) {
    const [statusResult, catalogResult] = await Promise.allSettled([
      invoke<Status>('get_harness_model_service', { pool }),
      pool === 'trae' ? invoke<Catalog>('get_trae_model_pool_models')
        : pool === 'workbuddy' ? invoke<Catalog>('get_workbuddy_pool_models', { refreshCatalog: online })
        : invoke<Catalog>('get_doubao_pool_catalog', { refresh: online }),
    ]);
    if (statusResult.status === 'fulfilled') setStatus(statusResult.value);
    let next: Catalog | undefined = catalogResult.status === 'fulfilled' ? catalogResult.value : undefined;
    if (!next && statusResult.status === 'fulfilled') {
      const ids = statusResult.value.pool.selectedModelIds ?? [];
      next = { models: ids.map(id => ({ id, name: id })), selectedModelIds: ids, catalogSource: '本地保存' };
    }
    if (next) {
      setCatalog(next);
      const ids = next.selectedModelIds;
      setModel(old => ids.includes(old) ? old : ids[0] ?? '');
    }
    if (statusResult.status === 'rejected') throw statusResult.reason;
    if (catalogResult.status === 'rejected') setError('模型目录暂时不可用；若有本地保存的选择，可继续查看服务状态。');
  }
  async function run(action: () => Promise<void>, refreshAfter = true) {
    setBusy(true); setError(''); setNotice('');
    try { await action(); if (refreshAfter) await refresh(); } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }
  useEffect(() => {
    void run(() => refresh(), false);
    const timer = window.setInterval(() => { void invoke<Status>('get_harness_model_service', { pool }).then(setStatus).catch(() => {}); }, 5000);
    return () => window.clearInterval(timer);
  }, [pool]);
  const selected = catalog?.models.filter(m => catalog.selectedModelIds.includes(m.id)) ?? [];
  const running = status?.pool.running && !status.pool.stopping;
  return <div className="harness-page" aria-busy={busy}>
    <header className="harness-heading"><div><p className="harness-eyebrow">模型服务</p><h1><Cable size={25} /> DeepSeek Harness</h1><p>复用现有账号池，Harness 独立思考并执行任务。</p></div>
      <button disabled={busy} onClick={() => void run(() => refresh(true), false)} title="重新检测并刷新上游目录"><RefreshCw size={16} /> 重新检测</button>
    </header>
    {error && <p className="harness-error" role="alert">{error}</p>}
    {notice && <p className="harness-notice" role="status">{notice}</p>}
    {!status && <p role="status">{busy ? '正在检测本机服务与 Harness…' : '检测未完成，请重新检测。'}</p>}
    <section className="harness-card"><h2>连接状态</h2><dl className="harness-status">
      <div><dt>Harness</dt><dd>{status ? status.harness.detected ? `已发现 desktop 配置 · ${status.harness.version ?? '版本未知'}` : '未发现 desktop 配置，请先启动 Harness' : '检测中'}</dd></div>
      <div><dt>模型服务</dt><dd>{status ? running ? '正在监听 · 仅限本机 · 已启用鉴权' : status.pool.stopping ? '正在停止' : '未启动' : '检测中'}</dd></div>
      <div><dt>接口地址</dt><dd><code>{status?.baseUrl ?? (pool === 'trae' ? 'http://127.0.0.1:18790/v1' : pool === 'workbuddy' ? 'http://127.0.0.1:18789/v1' : 'http://127.0.0.1:18791/v1')}</code></dd></div>
      <div><dt>配置状态</dt><dd>{status?.harness.configuredPool === pool ? '已配置为默认账号池' : status?.harness.configuredPools?.includes(pool) ? '已配置，可在 Harness 中选用' : status?.harness.configured ? '已配置其他账号池，可重新配置' : '尚未配置'}</dd></div>
    </dl>{status?.pool.lastError && <p role="alert">{status.pool.lastError}</p>}
    <button disabled={busy || running || !model} onClick={() => void run(async () => { await invoke('start_harness_model_service', { pool }); setNotice('模型服务已启动，可在 WorkBuddy 桌面程序关闭时使用。'); })}>启动模型服务</button>
    <p className="harness-hint">退出切换工具会停止服务。与对应账号池共用服务生命周期，停用账号池也会影响 Harness。</p>
    </section>
    {pool === 'workbuddy' && <p className="harness-error" role="status">WorkBuddy 的简单对话已测试通过，Harness Agent 请求仍可能被上游渠道校验拒绝，完整接入尚未验证。</p>}
    <section className="harness-card"><h2>模型与配置</h2>
      <label htmlFor="harness-model">默认模型</label>
      <select id="harness-model" disabled={busy || !selected.length} value={model} onChange={e => setModel(e.target.value)}>
        {!selected.length && <option value="">账号池尚未选择模型</option>}
        {selected.map(m => <option key={m.id} value={m.id}>{m.name} · {m.id}</option>)}
      </select>
      <p className="harness-hint">自动使用现有账号池中已保存的模型。增减模型请使用对应应用页的账号池管理。目录来源：{catalog?.catalogSource || (pool === 'trae' ? 'TRAE 账号池' : pool === 'workbuddy' ? 'WorkBuddy 账号池' : '豆包账号池')}。未提供可核验的模型剩余额度。</p>
      <div className="harness-actions">
        <button disabled={busy || !model || !status?.harness.detected} onClick={() => void run(async () => setNotice(await invoke<string>('configure_harness_model_service', { model: `${pool}-pool-${model}` })))}>自动配置 Harness</button>
        {[true, false].map(stream => <button key={String(stream)} disabled={busy || !running || !model} onClick={() => void run(async () => {
          const result = await invoke<{ timing?: { headersMs: number; totalMs: number; responseBytes: number }; completion?: { usage?: { total_tokens?: number } } }>('test_harness_model_service', { model: `${pool}-pool-${model}`, stream, pool });
          const timing = result.timing;
          const tokens = result.completion?.usage?.total_tokens;
          const detail = timing ? ` · 响应头 ${timing.headersMs} ms · 总耗时 ${timing.totalMs} ms · 响应 ${timing.responseBytes} B${typeof tokens === 'number' ? ` · ${tokens} tokens` : ''}` : '';
          setTest(JSON.stringify(result, null, 2)); setNotice(`${stream ? '流式' : '非流式'}真实模型调用通过（消耗账号额度）${detail}`);
        })}>{stream ? '测试流式调用' : '测试非流式调用'}</button>)}
        <button disabled={busy || !running || !model} onClick={() => void run(async () => {
          const result = await invoke('test_harness_tool_roundtrip', { model: `${pool}-pool-${model}`, pool });
          setTest(JSON.stringify(result, null, 2)); setNotice('工具调用与结果回传测试通过，共发送两次真实模型请求。');
        })}>测试工具调用</button>
      </div><p className="harness-hint">配置前自动备份。配置后重启 Harness，并新建会话。测试会消耗账号额度；工具协议测试不能代替完整 Harness Agent 验收。</p>
      {test && <details open><summary>最近测试响应</summary><pre>{test}</pre></details>}
    </section>
    <section className="harness-card"><h2>调用记录</h2><p className="harness-hint">当前账号池本次进程最近 50 条处理记录。不保存提示词、账号凭证。包含已记录的入口过载拒绝；最近50条不代表全部历史，流式转发结束不代表客户端完整接收。</p>
      {status?.recordSummary && <dl className="harness-status">
        <div><dt>记录数</dt><dd>{status.recordSummary.recordCount}</dd></div>
        <div><dt>非流式响应完成</dt><dd>{status.recordSummary.nonstreamSuccess}</dd></div>
        <div><dt>流式转发结束</dt><dd>{status.recordSummary.streamRelayOk}</dd></div>
        <div><dt>已记录限流 / 其他异常</dt><dd>{status.recordSummary.limited} / {status.recordSummary.failed}</dd></div>
        <div><dt>其中入口过载拒绝</dt><dd>{status.recordSummary.entryRejected ?? 0}</dd></div>
        <div><dt>非流式平均转发耗时</dt><dd>{typeof status.recordSummary.meanRelayMs === 'number' ? Math.round(status.recordSummary.meanRelayMs) + ' ms' : '暂无数据'} · {status.recordSummary.timingSamples} 条</dd></div>
        <div><dt>非流式P95转发耗时</dt><dd>{typeof status.recordSummary.p95RelayMs === "number" ? status.recordSummary.p95RelayMs + " ms" : "暂无数据"} · {status.recordSummary.timingSamples} 条</dd></div>
      </dl>}
      {status?.records.length ? <div className="harness-table"><table><thead><tr><th>时间</th><th>模型</th><th>模式</th><th>状态 / 结果</th><th>转发耗时</th></tr></thead><tbody>{[...status.records].reverse().map((r, i) => <tr key={`${r.time}-${i}`}><td>{new Date(r.time).toLocaleTimeString()}</td><td>{r.model || '请求校验'}</td><td>{r.stream ? '流式' : '非流式'}</td><td>{r.status} · {r.result}</td><td>{typeof r.relayMs === "number" ? r.relayMs + " ms" : "—"}</td></tr>)}</tbody></table></div> : <p>暂无调用记录。启动服务后可进行真实调用测试。</p>}
    </section>
  </div>;
}
