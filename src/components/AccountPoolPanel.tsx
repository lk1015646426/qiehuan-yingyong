import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { ArrowLeftRight, ChevronDown, Search, X } from 'lucide-react';
import './account-pool.css';

interface Model { id: string; name: string; description?: string }
interface Props {
  app: string;
  running?: boolean;
  stopping?: boolean;
  busy: boolean;
  loading: boolean;
  ready: boolean;
  modelCount: number;
  models: Model[];
  selected: string[];
  dirty: boolean;
  error?: string | null;
  notice?: string;
  metrics: { label: string; value: number | string }[];
  instructions: ReactNode;
  onSelect: (ids: string[]) => void;
  onDiscard: () => void;
  onToggle: () => void;
  onRefresh: () => void;
  onSave: () => Promise<boolean>;
  sync?: { enabled: boolean; explanation: string; onConfirm: () => Promise<boolean> };
}

/** 两个原客户端共用布局；服务调用和注册逻辑由各自面板提供。 */
export function AccountPoolPanel(p: Props) {
  const id = useId();
  const dialog = useRef<HTMLDialogElement>(null);
  const [details, setDetails] = useState(false);
  const [search, setSearch] = useState('');
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [confirmSync, setConfirmSync] = useState(false);
  const confirmation = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (confirmSync || confirmDiscard) {
      confirmation.current?.scrollIntoView({ block: 'nearest' });
      confirmation.current?.querySelector('button')?.focus({ preventScroll: true });
    }
  }, [confirmSync, confirmDiscard]);
  const locked = p.busy || p.loading || !!p.stopping;
  const selectedSet = new Set(p.selected);
  const available = p.models.filter(m => `${m.name} ${m.id}`.toLowerCase().includes(search.trim().toLowerCase()));
  function close(discard = false) {
    if (locked) return;
    if (p.dirty && !discard) { setConfirmSync(false); setConfirmDiscard(true); return; }
    if (discard) p.onDiscard();
    setConfirmDiscard(false); setConfirmSync(false);
    dialog.current?.close();
  }
  async function save(sync = false) {
    const ok = await (sync ? p.sync!.onConfirm() : p.onSave());
    if (ok) { setConfirmSync(false); dialog.current?.close(); }
  }
  const feedback = <>
    {p.error ? <p className="ap-feedback ap-feedback--error" role="alert">{p.error}</p> : null}
    {p.notice ? <p className="ap-feedback" role="status">{p.notice}</p> : null}
  </>;
  return <section className="account-pool" aria-label={`${p.app} 连续对话账号池`}>
    <div className="ap-summary">
      <span className="ap-icon"><ArrowLeftRight size={21} aria-hidden="true" /></span>
      <div className="ap-heading"><div className="ap-title"><h2>连续对话账号池</h2><span className={`ap-badge ${p.running ? 'is-running' : ''}`} role="status">{p.stopping ? '正在停止…' : p.running === undefined ? '正在读取…' : p.running ? '运行中' : '已停用'}</span></div>
        <p>已启用 <strong>{p.modelCount}</strong> 个模型<span className="ap-separator">·</span>{p.dirty ? '模型选择有修改待保存' : p.running ? '额度不足时自动尝试下一账号' : '启用后可在原客户端使用'}</p>
      </div>
      <div className="ap-actions"><button type="button" className="ap-text" aria-expanded={details} aria-controls={`${id}-details`} onClick={() => setDetails(!details)}>运行详情<ChevronDown size={13} className={details ? 'is-expanded' : ''} /></button>
        <button type="button" className="ap-button" onClick={() => { setSearch(''); setConfirmDiscard(false); setConfirmSync(false); dialog.current?.showModal(); }}>管理模型</button>
        <span className="ap-divider" />
        <button type="button" className="ap-switch" role="switch" aria-label={`${p.running ? '停用' : '启用'}${p.app}账号池`} aria-checked={!!p.running} disabled={locked || p.running === undefined || (!p.running && (p.dirty || !p.modelCount))} onClick={p.onToggle} />
      </div>
    </div>
    {details ? <div className="ap-details" id={`${id}-details`}>
      <dl>{p.metrics.map(m => <div key={m.label}><dt>{m.label}</dt><dd>{m.value}</dd></div>)}</dl>
      <details><summary>使用说明</summary><div className="ap-help">{p.instructions}</div></details>
    </div> : null}
    {feedback}
    <dialog ref={dialog} className="ap-drawer" aria-labelledby={`${id}-title`} onCancel={e => { e.preventDefault(); close(); }}>
      <div className="ap-drawer-layout">
        <header className="ap-drawer-header"><div><h2 id={`${id}-title`}>管理账号池模型</h2><p>{p.app} · 仅展示你选择的账号池模型</p></div><button type="button" className="ap-close" aria-label="关闭模型管理" disabled={locked} onClick={() => close()}><X size={18} /></button></header>
        <div className="ap-drawer-body">
          {feedback}
          <div className="ap-label"><h3>已选择 <span>{p.selected.length} 个</span></h3><button type="button" className="ap-text" disabled={locked || !p.selected.length} onClick={() => { p.onSelect([]); setConfirmSync(false); }}>清空选择</button></div>
          <div className="ap-chips">{p.selected.length ? p.selected.map(mid => <span className="ap-chip" key={mid}><span>{p.models.find(m => m.id === mid)?.name ?? mid}</span><button type="button" disabled={locked} aria-label={`移除模型 ${mid}`} onClick={() => { p.onSelect(p.selected.filter(i => i !== mid)); setConfirmSync(false); }}><X size={12} /></button></span>) : <p className="ap-muted">尚未选择模型，从下方添加即可。</p>}</div>
          <div className="ap-label"><h3>官方模型目录</h3><button type="button" className="ap-text" disabled={locked} onClick={p.onRefresh}>{p.loading ? '正在读取…' : '刷新目录'}</button></div>
          <label className="ap-search"><Search size={16} aria-hidden="true" /><input aria-label="搜索模型名称或 ID" type="search" placeholder="搜索模型名称或 ID" value={search} onChange={e => setSearch(e.target.value)} /></label>
          {p.loading ? <p className="ap-muted" role="status">正在读取官方模型目录…</p> : !p.ready ? <p className="ap-muted">模型目录暂不可用，请刷新目录重试。</p> : <ul className="ap-models" aria-label="官方模型目录">{available.map(m => <li key={m.id}><div><strong>{m.name}</strong><small>{m.description ?? m.id}</small></div><button type="button" className="ap-button" disabled={locked || selectedSet.has(m.id)} onClick={() => { p.onSelect([...p.selected, m.id]); setConfirmSync(false); }}>{selectedSet.has(m.id) ? '已添加' : '＋ 添加'}</button></li>)}</ul>}
          {p.ready && !p.loading && !available.length ? <p className="ap-muted">没有匹配的模型，试试其他关键词。</p> : null}
          <div className="ap-note">{p.sync ? p.sync.explanation : '保存后启用所选账号池模型。原有官方模型和其他自定义模型保留。'}{!p.selected.length ? <p>清空并保存会停用本地账号池；不会删除原客户端中已有的模型注册项。</p> : null}</div>
          {confirmSync ? <div ref={confirmation} className="ap-confirm" role="group" aria-label="确认 TRAE 模型同步"><strong>同步到当前 TRAE 账号？</strong><p>将添加或更新所选的 {p.selected.length} 个账号池模型，并保存本地连接密钥；其他账号的登录令牌不会写入模型配置。</p><div className="ap-actions"><button type="button" className="ap-button" disabled={locked} onClick={() => setConfirmSync(false)}>返回修改</button><button type="button" className="ap-button ap-primary" disabled={locked || !p.sync?.enabled} onClick={() => void save(true)}>{p.busy ? '正在同步…' : '确认同步'}</button></div></div> : null}
          {confirmDiscard ? <div ref={confirmation} className="ap-confirm" role="alert"><strong>还有未保存的修改</strong><p>关闭会放弃本次模型选择，已保存的配置不受影响。</p><div className="ap-actions"><button type="button" className="ap-button" onClick={() => setConfirmDiscard(false)}>继续编辑</button><button type="button" className="ap-button" onClick={() => close(true)}>放弃修改并关闭</button></div></div> : null}
        </div>
        <footer className="ap-drawer-footer"><span>{p.busy ? '正在处理…' : p.dirty ? '有修改待保存' : '模型选择已保存'}</span><div className="ap-actions"><button type="button" className="ap-button" disabled={locked} onClick={() => close()}>取消</button>{p.sync && p.selected.length ? <button type="button" className="ap-button" disabled={locked || !p.ready || !p.dirty} onClick={() => void save()}>仅保存本机</button> : null}<button type="button" className="ap-button ap-primary" disabled={locked || !p.ready || (p.sync && p.selected.length ? !p.sync.enabled : !p.dirty)} onClick={() => p.sync && p.selected.length ? setConfirmSync(true) : void save()}>{p.sync && p.selected.length ? '保存并同步' : p.selected.length ? '保存并启用' : '保存并停用'}</button></div></footer>
      </div>
    </dialog>
  </section>;
}
