import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useWorkBuddyStore } from '../../stores/useWorkBuddyStore';
import { triggerWorkBuddyCheckin } from '../../services/workBuddyService';
import '../../styles/components/workbuddy-activities.css';

interface Settings {
  enabled: boolean;
  compensation: boolean;
  tasks: boolean;
  travel: boolean;
  expiryDays: number;
}

interface ActivityTask {
  code: string;
  title: string;
  state: string;
  current: number | null;
  target: number | null;
  rewardCredit: number | null;
  locked: boolean;
  endsAt: number | null;
}

interface Overview {
  accountId: string;
  checkedAt: number;
  tasks: ActivityTask[];
  travel: { state: string | null; arriveAt: number | null; dailyLimitReached: boolean | null } | null;
  energy: number | null;
  expiries: { credits: number; endsAt: number }[];
  compensation: string;
  errors: string[];
}

const TASK_STATES: Record<string, string> = {
  not_accepted: '待接取', accepted: '已接取，待完成', in_progress: '进行中',
  completed: '可领奖', claimed: '奖励已领',
};
const TRAVEL_STATES: Record<string, string> = { idle: '尚未出发', traveling: '旅行中', arrived: '礼物待领取', unowned: '尚未领取，请先在官方成长中心领取一只 Buddy' };
const date = (value: number) => new Date(value * 1000).toLocaleString('zh-CN', {
  month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false,
});
const number = (value: number) => value.toLocaleString('zh-CN', { maximumFractionDigits: 2 });

export function WorkBuddyActivitiesPanel() {
  const accounts = useWorkBuddyStore(state => state.accounts);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saved, setSaved] = useState<Settings | null>(null);
  const [selected, setSelected] = useState('');
  const [overview, setOverview] = useState<Overview | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState('');
  const [queryError, setQueryError] = useState('');
  const [notice, setNotice] = useState('');
  const request = useRef(0);
  const dirty = settings !== null && JSON.stringify(settings) !== JSON.stringify(saved);

  useEffect(() => {
    let active = true;
    invoke<Settings>('get_workbuddy_activity_settings').then(value => {
      if (active) { setSettings(value); setSaved(value); }
    }).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; request.current += 1; };
  }, []);

  useEffect(() => {
    if (!accounts.some(account => account.id === selected)) setSelected(accounts[0]?.id ?? '');
  }, [accounts, selected]);

  const selectAccount = (value: string) => {
    request.current += 1;
    setSelected(value); setOverview(null); setQueryError(''); setLoading(false);
  };
  const query = async () => {
    if (!selected) return;
    const generation = ++request.current;
    setLoading(true); setQueryError('');
    try {
      const value = await invoke<Overview>('get_workbuddy_activity_overview', { accountId: selected });
      if (generation === request.current) setOverview(value);
    } catch (cause) {
      if (generation === request.current) { setQueryError(String(cause)); setOverview(null); }
    } finally { if (generation === request.current) setLoading(false); }
  };
  const save = async () => {
    if (!settings) return;
    setSaving(true); setError(''); setNotice('');
    try {
      const value = await invoke<Settings>('save_workbuddy_activity_settings', { settings });
      setSettings(value); setSaved(value);
      setNotice(value.enabled ? '活动配置已保存并同步云端，定时检查已开启。' : '活动配置已同步云端，自动活动已关闭。');
    } catch (cause) { setError(String(cause)); }
    finally { setSaving(false); }
  };
  const run = async () => {
    if (!selected || !saved?.enabled || dirty) return;
    setRunning(true); setError(''); setNotice('');
    try {
      await triggerWorkBuddyCheckin(selected);
      setNotice('云端任务已提交。执行结果可在下方「云端签到任务」查看，完成后点击查询更新总览。');
    } catch (cause) { setError(String(cause)); }
    finally { setRunning(false); }
  };
  const account = accounts.find(value => value.id === selected);

  return <section className="wb-activity-panel" aria-labelledby="wb-activity-title">
    <h2 id="wb-activity-title" className="wc-section-title">积分活动与成长中心</h2>
    <div className="wc-slot">
      <p className="wb-activity-description">开启后，为已开启自动签到的账号检查活动补偿、接取任务与领取已完成奖励，并安排 Buddy 旅行。任务实际使用条件仍需完成。</p>
      <p className="wb-activity-description">每天 04:00 随签到执行，其余每四小时静默检查；奖励和到期提醒合并到每天一次的完整汇总。</p>
      {settings ? <fieldset className="wb-activity-options" disabled={saving}>
        <legend className="wb-activity-sr-only">自动活动设置</legend>
        <label className="wb-activity-option wb-activity-option-main">
          <input type="checkbox" checked={settings.enabled} onChange={event => setSettings({ ...settings, enabled: event.target.checked })} />
          <span>开启云端自动活动</span>
        </label>
        {([
          ['compensation', '领取可用活动补偿'], ['tasks', '接取任务并补领奖'], ['travel', '派 Buddy 旅行与收礼物'],
        ] as const).map(([key, title]) => <label key={key} className="wb-activity-option">
          <input type="checkbox" checked={settings[key]} onChange={event => setSettings({ ...settings, [key]: event.target.checked })} />
          <span>{title}</span>
        </label>)}
        <label className="wb-activity-expiry">到期提醒
          <select aria-label="到期提醒天数" value={settings.expiryDays} onChange={event => setSettings({ ...settings, expiryDays: Number(event.target.value) })}>
            {[1, 3, 7, 14, 30].map(days => <option key={days} value={days}>提前 {days} 天</option>)}
          </select>
        </label>
      </fieldset> : !error ? <p role="status">正在读取活动设置…</p> : null}
      <div className="wc-slot-actions">
        <button type="button" className="wc-btn wc-btn-primary" disabled={!settings || saving || running} onClick={() => void save()}>{saving ? '正在保存并同步…' : '保存并同步云端'}</button>
        <span className="wb-activity-muted" role="status">{dirty ? '有修改待保存' : saved?.enabled ? '已保存：自动活动开启' : saved ? '已保存：自动活动关闭' : ''}</span>
      </div>
      {error ? <p className="wc-status wc-status--error" role="alert">{error}</p> : null}
      {notice ? <p className="wc-status wc-status--ok" role="status">{notice}</p> : null}
      <div className="wb-activity-query">
        <label htmlFor="wb-activity-account">查看账号</label>
        <select id="wb-activity-account" value={selected} onChange={event => selectAccount(event.target.value)} disabled={!accounts.length}>
          {!accounts.length ? <option value="">暂无账号</option> : accounts.map(value => <option key={value.id} value={value.id}>{value.displayName}</option>)}
        </select>
        <button type="button" className="wc-btn" disabled={!selected || loading} onClick={() => void query()}>{loading ? '正在查询…' : '查询活动总览'}</button>
        <button type="button" className="wc-btn" disabled={!account?.checkinEnabled || !saved?.enabled || dirty || saving || running} onClick={() => void run()}>{running ? '正在提交…' : '云端执行此账号'}</button>
      </div>
      {!accounts.length ? <p className="wb-activity-muted">先导入账号，即可查询成长中心。</p> : !overview && !loading && !queryError ? <p className="wb-activity-muted">选择账号并查询，查看任务、旅行、补偿和积分到期情况。</p> : null}
      {loading ? <p role="status">正在查询当前账号的活动数据…</p> : null}
      {queryError ? <p className="wc-status wc-status--error" role="alert">{queryError}</p> : null}
      {overview && overview.accountId === selected ? <div aria-live="polite">
        <div className="wb-activity-summary">
          <span>活动补偿：{overview.compensation}</span>
          <span>Buddy：{TRAVEL_STATES[overview.travel?.state ?? ''] ?? '状态未确认'}{overview.travel?.state === 'traveling' && overview.travel.arriveAt ? ` · 预计 ${date(overview.travel.arriveAt)} 回来` : ''}</span>
          <span>能量：{overview.energy === null ? '未查询到' : number(overview.energy)}</span>
          <span>可领奖：{overview.tasks.filter(task => task.state === 'completed').length} 项</span>
        </div>
        {overview.travel?.dailyLimitReached ? <p className="wb-activity-muted">今天的旅行名额已用完，已出发的旅行仍会继续。</p> : null}
        {overview.errors.map(message => <p key={message} className="wc-status wc-status--error" role="alert">{message}</p>)}
        <div className="wb-activity-table-wrap">
          <table className="wb-activity-table">
            <caption>成长任务（{overview.tasks.length} 项）· 查询于 {date(overview.checkedAt)}</caption>
            <thead><tr><th scope="col">任务</th><th scope="col">状态与进度</th><th scope="col">积分奖励</th><th scope="col">截止时间</th></tr></thead>
            <tbody>{overview.tasks.map(task => <tr key={task.code}>
              <td>{task.title}</td>
              <td>{task.locked ? '未开放或已结束' : TASK_STATES[task.state] ?? '状态未确认'}{task.target !== null && task.current !== null ? <span className="wb-activity-progress">{number(task.current)} / {number(task.target)}</span> : null}</td>
              <td>{task.rewardCredit === null ? '—' : number(task.rewardCredit)}</td>
              <td>{task.endsAt ? date(task.endsAt) : '未设置截止时间'}</td>
            </tr>)}</tbody>
          </table>
          {!overview.tasks.length && !overview.errors.length ? <p className="wb-activity-muted">当前没有下发成长任务。</p> : null}
        </div>
        <div className="wb-activity-expiries">
          <h3>{saved?.expiryDays ?? 7} 天内即将到期的积分</h3>
          {overview.expiries.length ? <ul>{overview.expiries.map((item, index) => <li key={`${item.endsAt}-${index}`}>{number(item.credits)} 积分 · {date(item.endsAt)} 到期</li>)}</ul> : <p className="wb-activity-muted">{overview.errors.some(value => value.startsWith('到期查询')) ? '到期查询未完成。' : '查询范围内没有即将到期的可用积分。'}</p>}
        </div>
      </div> : null}
    </div>
  </section>;
}
