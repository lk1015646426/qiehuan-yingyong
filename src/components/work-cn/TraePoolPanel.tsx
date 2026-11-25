import { useCallback, useEffect, useState } from 'react';
import { getTraeModelPoolModels, getTraeModelPoolStatus, saveTraeModelPoolModels, startTraeModelPool, stopTraeModelPool, registerTraeModelPoolModels } from '../../services/traePoolService';
import { AccountPoolPanel } from '../AccountPoolPanel';
import type { TraeModelPoolModels, TraeModelPoolStatus } from '../../types/traePool';

export function TraePoolPanel({ accountId }: { accountId: string | null }) {
  const [status, setStatus] = useState<TraeModelPoolStatus | null>(null);
  const [catalog, setCatalog] = useState<TraeModelPoolModels | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
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
  const save = async (sync = false) => {
    setBusy(true); setError(''); setNotice('');
    try {
      const next = await saveTraeModelPoolModels(selected); setCatalog(next); setSelected(next.selectedModelIds);
      if (next.selectedModelIds.length) { setStatus(await startTraeModelPool()); setNotice('已保存并启用账号池模型。请在 TRAE 中选择「TRAE 账号池 · 模型名称」。'); }
      else { setStatus(await stopTraeModelPool()); setNotice('已清空账号池模型，默认不会展示任何账号池模型。'); }
      if (sync && next.selectedModelIds.length && accountId) {
        const result = await registerTraeModelPoolModels(accountId);
        setNotice(`已确认 ${result.registered.length + result.alreadyRegistered.length} 个模型同步到当前 TRAE 账号，请在原客户端选择「TRAE 账号池 · 模型名称」。`);
      }
      return true;
    } catch (cause) { setNotice(''); setError(String(cause)); return false; }
    finally { setBusy(false); }
  };
  const toggle = async () => {
    setBusy(true); setError('');
    try { setStatus(await (status?.running ? stopTraeModelPool() : startTraeModelPool())); }
    catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  return <AccountPoolPanel
    app="TRAE" running={status?.running} stopping={status?.stopping}
    busy={busy} loading={loading} ready={!!catalog}
    modelCount={catalog?.selectedModelIds.length ?? status?.selectedModelIds.length ?? 0}
    models={(catalog?.models ?? []).map(m => ({ ...m, description: `${m.id} · ${m.multimodal ? '支持图片' : '文本模型'}${m.tools === false ? ' · 不支持工具' : ''}` }))}
    selected={selected} dirty={dirty} error={error || status?.lastError} notice={notice}
    metrics={[{ label: '累计请求', value: status?.requests ?? 0 },
      { label: '账号轮换', value: status?.rotations ?? 0 },
      { label: '进行中', value: status?.activeRequests ?? 0 },
      { label: '冷却账号', value: status?.coolingAccounts ?? 0 }]}
    instructions={<><p>在 TRAE 中选择「TRAE 账号池 · 模型名称」。额度不足时携带完整历史尝试下一账号，保持所选模型。使用期间保持切换应用运行，程序重启后自动恢复账号池。</p><p>已开始输出的请求不会自动重发；中途断流会显示错误，避免重复生成或重复调用工具。模型列表未更新时，请重新打开模型选择器。</p></>}
    onSelect={setSelected} onDiscard={() => setSelected(catalog?.selectedModelIds ?? [])}
    onToggle={() => void toggle()} onRefresh={() => void loadModels()} onSave={() => save()}
    sync={{ enabled: !!accountId, explanation: accountId
      ? '保存并同步会向当前 TRAE 账号添加或更新所选模型，并保存本地连接密钥。原有官方模型和其他自定义模型保留。'
      : '请先导入并选用当前 TRAE 账号，才能同步模型。现在可以先保存本机配置。',
      onConfirm: () => save(true) }}
  />;
}
