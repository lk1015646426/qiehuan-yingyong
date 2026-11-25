import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AccountPoolPanel } from '../AccountPoolPanel';
import { AccountManagementLayout, AccountList, AccountCardFrame, AccountCardActions, AccountStatusBanner } from '../AccountManagementLayout';

interface PoolStatus {
  running: boolean;
  stopping: boolean;
  lastError?: string | null;
  endpoint: string;
  requests: number;
  rotations: number;
  coolingAccounts: number;
  coolingAccountIds?: string[];
  modelCount: number;
  selectedModelIds: string[];
  catalogSource: string;
  keyCount: number;
}

interface PoolModel {
  id: string;
  name: string;
}
interface Catalog {
  models: PoolModel[];
  selectedModelIds: string[];
  catalogSource: string;
  updatedAt: number;
}

interface KeyEntry {
  id: string;
  name: string;
  key: string;
  enabled: boolean;
}

interface KeySnapshot {
  entries: KeyEntry[];
  revision: number[] | null;
}

type QuotaKind = 'package' | 'bonus' | 'fiveHour' | 'weekly';

interface QuotaWindow {
  kind: QuotaKind;
  remaining: number | null;
  total: number | null;
  remainingPercent: number | null;
  endsAt: number | null;
}

interface DoubaoQuota {
  plan: string | null;
  availablePoints: number | null;
  windows: QuotaWindow[];
  checkedAt: number;
}

interface QuotaState {
  result?: DoubaoQuota;
  loading: boolean;
  error?: string;
}

const QUOTA_LABELS: Record<QuotaKind, string> = {
  package: '套餐积分',
  bonus: '活动赠送额度',
  fiveHour: '5 小时额度',
  weekly: '周额度',
};
const quotaNumber = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 2 });
const quotaTime = new Intl.DateTimeFormat('zh-CN', {
  month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit',
});

function quotaCaption(state?: QuotaState) {
  if (!state?.result) {
    if (state?.loading) return '查询中…';
    return state?.error ? '查询失败' : '待查询';
  }
  const { availablePoints, windows, checkedAt } = state.result;
  const active = windows.filter(row => row.endsAt === null || row.endsAt > checkedAt);
  const percent = active.length === 1 ? active[0].remainingPercent : null;
  if (availablePoints != null) return quotaNumber.format(availablePoints);
  if (percent != null) return `约 ${quotaNumber.format(percent)}% 额度`;
  return active.some(row => row.remaining !== null || row.remainingPercent !== null)
    ? '见分项'
    : '未提供';
}

function QuotaDetails({ state }: { state?: QuotaState }) {
  const result = state?.result;
  return <>
    {state?.error && <div className="wb-status-error" role="status">{state.error}</div>}
    {result && <>
      <div className="wb-status-line">
        {result.plan && <span>订阅套餐 <strong>{result.plan}</strong></span>}
        <span>查询时间 <strong>{quotaTime.format(result.checkedAt)}</strong></span>
      </div>
      {result.windows.map((row, index) => {
        const expired = row.endsAt !== null && row.endsAt <= result.checkedAt;
        let balance = '未提供';
        if (row.remaining !== null && row.total !== null) {
          balance = `${quotaNumber.format(row.remaining)} / ${quotaNumber.format(row.total)}`;
        } else if (row.remainingPercent !== null) {
          balance = `约 ${quotaNumber.format(row.remainingPercent)}%`;
        }
        return <div key={index} className="wb-status-line">
          <span>{QUOTA_LABELS[row.kind]} <strong>{expired ? '已到期' : balance}</strong></span>
          {row.endsAt !== null && !expired && <span>
            {row.kind === 'bonus' ? '到期' : '重置'} <strong>{quotaTime.format(row.endsAt)}</strong>
          </span>}
        </div>;
      })}
    </>}
  </>;
}

export function DoubaoPoolPanel() {
  const [status, setStatus] = useState<PoolStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [accountError, setAccountError] = useState('');
  const [notice, setNotice] = useState('');
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [catalogBusy, setCatalogBusy] = useState(false);
  const [snapshot, setSnapshot] = useState<KeySnapshot | null>(null);
  const keys = snapshot?.entries ?? [];
  const [newKey, setNewKey] = useState('');
  const [newKeyName, setNewKeyName] = useState('');
  const [keysBusy, setKeysBusy] = useState(true);
  const [keysError, setKeysError] = useState('');
  const keysRequest = useRef(0);
  const savingAccounts = useRef(false);
  const [quotaByAccount, setQuotaByAccount] = useState<Record<string, QuotaState>>({});
  const [refreshingAll, setRefreshingAll] = useState(false);
  const quotaRequests = useRef(new Set<string>());
  const quotaAttempted = useRef(new Set<string>());
  const [dialogMode, setDialogMode] = useState<'desktop' | 'web'>('desktop');
  const dialog = useRef<HTMLDialogElement>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [editName, setEditName] = useState('');
  const [deleting, setDeleting] = useState<string | null>(null);
  const accountsLocked = busy || keysBusy || Boolean(keysError) || !snapshot;
  const refreshingQuota = refreshingAll || Object.values(quotaByAccount).some(state => state.loading);
  const dirty = JSON.stringify([...selected].sort()) !== JSON.stringify([...(catalog?.selectedModelIds ?? [])].sort());

  const refreshStatus = async () => {
    try { setStatus(await invoke<PoolStatus>('get_doubao_pool_status')); }
    catch (cause) { setError(String(cause)); }
  };

  const loadModels = async (refreshCatalog = false) => {
    setCatalogBusy(true); setError('');
    try {
      const next = await invoke<Catalog>('get_doubao_pool_catalog', { refresh: refreshCatalog });
      setCatalog(next);
      setSelected(next.selectedModelIds);
    } catch (cause) { setError(String(cause)); }
    finally { setCatalogBusy(false); }
  };

  const loadKeys = async () => {
    const request = ++keysRequest.current;
    setKeysBusy(true);
    setKeysError('');
    try {
      const saved = await invoke<KeySnapshot>('get_doubao_pool_keys');
      if (!Array.isArray(saved?.entries)) throw new Error('豆包账号列表格式不匹配，请重新打开应用');
      if (request === keysRequest.current) setSnapshot(saved);
    } catch (cause) {
      if (request === keysRequest.current) setKeysError(String(cause));
    } finally {
      if (request === keysRequest.current) setKeysBusy(false);
    }
  };

  useEffect(() => {
    void refreshStatus();
    void loadModels();
    void loadKeys();
    const timer = window.setInterval(() => { void refreshStatus(); }, 5000);
    return () => window.clearInterval(timer);
  }, []);

  const refreshQuota = useCallback(async (accountId: string) => {
    if (quotaRequests.current.has(accountId)) return;
    quotaRequests.current.add(accountId);
    quotaAttempted.current.add(accountId);
    setQuotaByAccount(previous => ({
      ...previous,
      [accountId]: { result: previous[accountId]?.result, loading: true },
    }));
    try {
      const result = await invoke<DoubaoQuota>('get_doubao_account_quota', { accountId });
      if (!result || !Array.isArray(result.windows) || !Number.isFinite(result.checkedAt)) {
        throw new Error('豆包额度响应格式不匹配');
      }
      setQuotaByAccount(previous => ({ ...previous, [accountId]: { result, loading: false } }));
    } catch (cause) {
      const error = cause instanceof Error ? cause.message : String(cause);
      setQuotaByAccount(previous => ({
        ...previous,
        [accountId]: { result: previous[accountId]?.result, loading: false, error },
      }));
    } finally {
      quotaRequests.current.delete(accountId);
    }
  }, []);

  useEffect(() => {
    if (!snapshot) return;
    let cancelled = false;
    void (async () => {
      for (const { id } of snapshot.entries) {
        if (cancelled) break;
        if (!quotaAttempted.current.has(id)) await refreshQuota(id);
      }
    })();
    return () => { cancelled = true; };
  }, [snapshot, refreshQuota]);

  const refreshAllQuotas = async () => {
    if (refreshingAll || quotaRequests.current.size > 0 || accountsLocked) return;
    setRefreshingAll(true);
    try {
      for (const { id } of keys) await refreshQuota(id);
    } finally {
      setRefreshingAll(false);
    }
  };

  const toggle = async () => {
    setBusy(true); setError('');
    try {
      setStatus(await invoke<PoolStatus>(status?.running ? 'stop_doubao_pool' : 'start_doubao_pool'));
    } catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };

  const saveModels = async () => {
    setBusy(true); setError(''); setNotice('');
    try {
      const next = await invoke<Catalog>('save_doubao_pool_models', { modelIds: selected });
      setCatalog(next); setSelected(next.selectedModelIds);
      setNotice(next.selectedModelIds.length ? `已保存 ${next.selectedModelIds.length} 个豆包模型。` : '已清空模型选择。');
      await refreshStatus();
      return true;
    } catch (cause) { setError(String(cause)); return false; }
    finally { setBusy(false); }
  };

  const persistAccounts = async (update: (saved: KeyEntry[]) => KeyEntry[] | Promise<KeyEntry[]>) => {
    if (accountsLocked || !snapshot || savingAccounts.current) return false;
    savingAccounts.current = true;
    setBusy(true); setAccountError(''); setNotice('');
    try {
      const entries = await update(snapshot.entries);
      const started = performance.now();
      try {
        const saved = await invoke<KeySnapshot>('save_doubao_pool_keys', { entries, expectedRevision: snapshot.revision });
        setSnapshot(saved);
      } catch (cause) {
        void loadKeys();
        throw cause;
      }
      setNotice('豆包账号已保存');
      console.info('[Doubao account update timing] save_ms=%d', Math.round(performance.now() - started));
      void refreshStatus();
      return true;
    } catch (cause) { setAccountError(String(cause)); return false; }
    finally { savingAccounts.current = false; setBusy(false); }
  };

  const addKey = async () => {
    if (!newKey.trim()) return;
    if (await persistAccounts(saved => [...saved, {
      id: crypto.randomUUID(), name: newKeyName.trim() || `豆包账号 ${saved.length + 1}`,
      key: newKey.trim(), enabled: true,
    }])) dialog.current?.close();
  };

  const openLogin = async () => {
    setBusy(true); setAccountError('');
    try {
      await invoke('open_doubao_login');
      setNotice('请在豆包窗口完成登录，再返回这里点击「导入已登录账号」。');
    } catch (cause) { setAccountError(String(cause)); }
    finally { setBusy(false); }
  };

  const importAccount = async (source: 'desktop' | 'web') => {
    const saved = await persistAccounts(async entries => {
      const entry = await invoke<KeyEntry>(source === 'desktop' ? 'import_doubao_desktop' : 'import_doubao_login', {
        name: newKeyName.trim() || `豆包账号 ${entries.length + 1}`,
      });
      return [...entries, entry];
    });
    if (!saved) return;
    dialog.current?.close();
    if (source === 'web') {
      try {
        await invoke('close_doubao_login');
      } catch (cause) {
        setAccountError(`账号已保存。${String(cause)}`);
      }
    }
  };

  const showDialog = () => {
    if (accountsLocked) return;
    setDialogMode('desktop');
    setAccountError('');
    setNotice('');
    dialog.current?.showModal();
  };

  return <AccountManagementLayout app="豆包" actions={{
    importAccount: { onClick: showDialog, disabled: accountsLocked },
    sync: { title: '豆包账号池暂未接入 GitHub 同步' },
    refresh: {
      onClick: () => void refreshAllQuotas(), disabled: accountsLocked || !keys.length || refreshingQuota,
      title: '刷新已保存豆包账号的积分和额度',
    },
    checkin: { title: '豆包暂未接入本地签到' },
    settings: { title: '豆包使用自动检测的客户端数据目录' },
    detect: {
      onClick: () => { void loadKeys(); void refreshStatus(); },
      disabled: busy || keysBusy, title: '重新读取豆包账号列表和账号池状态',
    },
  }}>
    <AccountStatusBanner>在豆包客户端登录并完全退出后，可导入当前账号</AccountStatusBanner>
    {accountError || notice ? <section className={`wc-status ${accountError ? 'wc-status--error' : 'wc-status--ok'}`} role={accountError ? 'alert' : 'status'}><div style={{ flex: 1 }}>{accountError || notice}</div><button type="button" className="wc-btn" onClick={() => { setAccountError(''); setNotice(''); }}>知道了</button></section> : null}
    <AccountPoolPanel
      app="豆包" running={status?.running} stopping={status?.stopping}
      busy={busy} loading={catalogBusy} ready={!!catalog}
      modelCount={catalog?.selectedModelIds.length ?? status?.modelCount ?? 0}
      models={catalog?.models ?? []} selected={selected} dirty={dirty}
      error={error || status?.lastError} notice=""
      metrics={[
        { label: '累计请求', value: status?.requests ?? 0 },
        { label: '账号轮换', value: status?.rotations ?? 0 },
        { label: '冷却账号', value: status?.coolingAccounts ?? 0 },
        { label: '可用账号数', value: status?.keyCount ?? 0 },
        { label: '模型目录', value: '网页对话模式' },
      ]}
      instructions={<><p>先登录并导入账号，勾选对话模式并保存，再启动账号池。客户端使用下方本地 API Key；它与账号登录 Cookie 分别使用。</p><p>支持文本对话、流式与非流式输出；多轮消息以文本历史发送。当前不支持图片和外部工具调用，接入 Harness 时请使用普通对话。</p><p>网页接口可能随豆包更新而变化。账号失效或需要验证时，请重新登录并导入。普通/深度思考是网页模式，不代表指定底层模型版本。</p></>}
      onSelect={setSelected} onDiscard={() => setSelected(catalog?.selectedModelIds ?? [])}
      onToggle={() => void toggle()} onRefresh={() => void loadModels(true)} onSave={saveModels}
    />
    {keysError && <section className="wc-status wc-status--error" role="alert">
      <span style={{ flex: 1 }}>{keysError}</span>
      <button type="button" className="wc-btn" disabled={busy || keysBusy} onClick={() => void loadKeys()}>重新读取</button>
    </section>}
    {keysBusy && !snapshot && <AccountStatusBanner>正在读取已保存账号…</AccountStatusBanner>}
    {snapshot && <AccountList count={keys.length} loading={keysBusy} emptyHint="通过右上角导入或添加账号">
        {keys.map((account) => {
          const quota = quotaByAccount[account.id];
          const scheduling = !account.enabled ? '已停用'
            : status?.coolingAccountIds?.includes(account.id) ? '冷却中'
            : status?.stopping ? '正在停止' : status?.running ? '可调度' : '等待启动';

          return <AccountCardFrame key={account.id} title={account.name}
            credits={<span aria-live="polite">
              {quotaCaption(quota)}{quota?.result && (quota.error ? '（上次）' : quota.loading ? '（刷新中）' : '')}
            </span>}
            creditsTitle="豆包订阅及活动额度"
            alert={!quota?.result && quota?.error ? <div className="wb-status-error" role="alert">{quota.error}</div> : undefined}
            details={<>
              <div className="wb-status-line"><span>调度状态 <strong>{scheduling}</strong></span></div>
              <label className="wb-checkin-toggle">
                <input type="checkbox" checked={account.enabled} disabled={accountsLocked || refreshingAll}
                  onChange={event => {
                    const enabled = event.target.checked;
                    void persistAccounts(saved => saved.map(key => key.id === account.id ? { ...key, enabled } : key));
                  }} />
                <span>{account.enabled ? '账号池调度已启用' : '账号池调度已停用'}</span>
              </label>
              <QuotaDetails state={quota} />
            </>}
            recovery={{ title: '请重新登录豆包后导入账号' }}
            cloudCheckin={{ title: '豆包暂未接入云端签到' }}>
          {editing === account.id ? <div className="wb-edit-name"><input className="wc-input" aria-label="账号备注" value={editName} maxLength={80} onChange={e => setEditName(e.target.value)} />
            <button type="button" className="wc-btn" disabled={accountsLocked || !editName.trim()} onClick={() => void persistAccounts(saved => saved.map(k => k.id === account.id ? { ...k, name: editName.trim() } : k)).then(ok => { if (ok) setEditing(null); })}>保存</button>
            <button type="button" className="wc-btn" disabled={busy} onClick={() => setEditing(null)}>取消</button></div> : null}
          <AccountCardActions
            switchAccount={{ title: '豆包账号可在账号池内调度，客户端切换暂未接入' }}
            checkin={{ title: '豆包暂未接入客户端签到' }}
            refresh={{
              onClick: () => void refreshQuota(account.id), disabled: accountsLocked || quota?.loading || refreshingAll,
              label: quota?.loading ? '刷新中…' : undefined,
            }}
            rename={{
              onClick: () => { setEditName(account.name); setEditing(account.id); },
              disabled: accountsLocked || refreshingAll,
            }}
            remove={{
              onClick: deleting === account.id
                ? () => void persistAccounts(saved => saved.filter(key => key.id !== account.id)).then(ok => { if (ok) setDeleting(null); })
                : () => setDeleting(account.id),
              disabled: accountsLocked || refreshingAll,
            }}
            cancelRemove={deleting === account.id ? { onClick: () => setDeleting(null), disabled: busy } : undefined}
          />
        </AccountCardFrame>;
        })}
    </AccountList>}
    <dialog ref={dialog} className="wc-dialog doubao-account-dialog" aria-labelledby="doubao-dialog-title"
      onCancel={event => { if (busy) event.preventDefault(); else setAccountError(''); }}
      onClose={() => { setNewKeyName(''); setNewKey(''); }}>
        <h3 className="wc-dialog-title" id="doubao-dialog-title">{dialogMode === 'desktop' ? '导入当前豆包账号' : '添加豆包账号'}</h3>
        <p className="wc-dialog-note">{dialogMode === 'desktop' ? '请先在豆包桌面客户端登录，完全退出客户端后导入。登录信息会加密保存。' : '可通过网页登录后导入，也可手动输入登录 Cookie。'}</p>
        <label className="wc-row-label" htmlFor="doubao-name">账号备注（可选）</label>
        <input id="doubao-name" className="wc-input" value={newKeyName} maxLength={80} onChange={e => setNewKeyName(e.target.value)} placeholder="例如：个人号" />
        {dialogMode === 'web' ? <><div className="wc-dialog-actions"><button type="button" className="wc-btn" disabled={accountsLocked} onClick={() => void openLogin()}>打开网页登录窗口</button><button type="button" className="wc-btn" disabled={accountsLocked} onClick={() => void importAccount('web')}>导入已登录账号</button></div>
          <label className="wc-row-label" htmlFor="doubao-cookie">或手动导入 Cookie</label><input id="doubao-cookie" className="wc-input" type="password" value={newKey} onChange={e => setNewKey(e.target.value)} placeholder="粘贴含 sessionid= 的 Cookie" /></> : null}
        {keysError || accountError ? <div className="wc-dialog-error" role="alert">{keysError || accountError}</div> : null}
        {notice && <p className="wc-dialog-note" role="status">{notice}</p>}
        <div className="wc-dialog-actions">
          <button type="button" className="wc-btn" disabled={accountsLocked}
            onClick={() => { setAccountError(''); setDialogMode(dialogMode === 'desktop' ? 'web' : 'desktop'); }}>
            {dialogMode === 'desktop' ? '添加账号' : '返回客户端导入'}
          </button>
          <button type="button" className="wc-btn" disabled={busy} onClick={() => { setAccountError(''); dialog.current?.close(); }}>取消</button>
          {keysError && <button type="button" className="wc-btn" disabled={busy || keysBusy} onClick={() => void loadKeys()}>重新读取账号</button>}
          {dialogMode === 'desktop' ? <button type="button" className="wc-btn wc-btn-primary" disabled={accountsLocked} onClick={() => void importAccount('desktop')}>{busy ? '导入中…' : '导入当前账号'}</button> : <button type="button" className="wc-btn wc-btn-primary" disabled={accountsLocked || !newKey.trim()} onClick={() => void addKey()}>添加并保存</button>}
        </div>
    </dialog>
  </AccountManagementLayout>;
}
