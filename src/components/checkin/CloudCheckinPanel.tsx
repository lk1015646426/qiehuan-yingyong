// 共享云端签到面板：TRAE / WorkBuddy / 智谱 三页复用。
//
// 数据与触发走仓库级命令（list_checkin_workflow_runs / trigger_checkin_workflow），
// 仓库与 workflow 由共享的 github.json 决定（三页同步到同一签到仓库），
// 与账号类型无关；日常签到由 GitHub Actions 北京时间 04:00 自动执行，
// 这里面板只做低频诊断：查看运行记录 + 手动触发验证/补签，默认收起。
import { useEffect, useState } from 'react';
import { ChevronDown } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useCheckinStore } from '../../stores/useCheckinStore';
import type { CheckinWorkflowRun } from '../../types/checkin';

function runTone(run: CheckinWorkflowRun): 'ok' | 'error' | 'running' | 'unknown' {
  if (run.status !== 'completed') {
    return 'running';
  }
  if (run.conclusion === 'success') {
    return 'ok';
  }
  if (run.conclusion === 'failure' || run.conclusion === 'cancelled') {
    return 'error';
  }
  return 'unknown';
}

function formatRelative(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) {
    return iso;
  }
  const diff = Date.now() - then;
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 1) {
    return '刚刚';
  }
  if (minutes < 60) {
    return `${minutes} 分钟前`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours} 小时前`;
  }
  const days = Math.floor(hours / 24);
  if (days < 30) {
    return `${days} 天前`;
  }
  return new Date(then).toLocaleDateString('zh-CN');
}

export function CloudCheckinPanel({ canTrigger = true }: { canTrigger?: boolean }) {
  const runs = useCheckinStore((s) => s.runs);
  const runsLoading = useCheckinStore((s) => s.runsLoading);
  const runsError = useCheckinStore((s) => s.runsError);
  const loadRuns = useCheckinStore((s) => s.loadRuns);
  const triggering = useCheckinStore((s) => s.triggering);
  const triggerMessage = useCheckinStore((s) => s.triggerMessage);
  const triggerRun = useCheckinStore((s) => s.triggerRun);
  const [open, setOpen] = useState(false);

  // 展开时加载运行记录（低频诊断功能，默认收起）。
  useEffect(() => {
    if (open) {
      void loadRuns();
    }
  }, [open, loadRuns]);

  return (
    <section className="wc-cloud-section">
      <button
        type="button"
        className="wc-cloud-toggle"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
      >
        <h2 className="wc-section-title">
          云端签到任务
          {runs[0] ? (
            <span
              className={
                runTone(runs[0]) === 'ok'
                  ? 'wc-cloud-summary wc-cloud-summary--ok'
                  : runTone(runs[0]) === 'error'
                    ? 'wc-cloud-summary wc-cloud-summary--error'
                    : 'wc-cloud-summary'
              }
            >
              最近：{runTone(runs[0]) === 'ok' ? '成功' : runTone(runs[0]) === 'error' ? '失败' : '运行中'}
              {' · '}
              {formatRelative(runs[0].createdAt)}
            </span>
          ) : (
            <span className="wc-cloud-summary">每天 04:00 自动执行</span>
          )}
        </h2>
        <ChevronDown size={16} className={open ? 'wc-cloud-chevron wc-cloud-chevron--open' : 'wc-cloud-chevron'} />
      </button>
      {open ? (
        <div className="wc-cloud-body">
          <div className="wc-cloud-actions">
            <button
              type="button"
              className="wc-btn"
              onClick={() => void loadRuns()}
              disabled={runsLoading}
            >
              {runsLoading ? '查询中…' : '刷新运行记录'}
            </button>
            <button
              type="button"
              className="wc-btn wc-btn-primary"
              onClick={() => void triggerRun()}
              disabled={triggering || !canTrigger}
              title="手动触发云端签到（用于凭证修复后的验证/补签，日常由 Actions 凌晨 4 点自动执行）"
            >
              {triggering ? '触发中…' : '验证云端任务'}
            </button>
          </div>
          {triggerMessage ? (
            <div
              className={
                triggerMessage.includes('失败') || triggerMessage.includes('中止')
                  ? 'ck-banner ck-banner--error'
                  : 'ck-banner ck-banner--running'
              }
            >
              {triggerMessage}
            </div>
          ) : null}
          <div className="ck-runs">
            {runsError ? <div className="ck-runs-error">{runsError}</div> : null}
            {!runsError && runs.length === 0 && !runsLoading ? (
              <div className="ck-runs-empty">暂无运行记录（每天北京时间 04:00 自动执行）</div>
            ) : null}
            {runs.map((run) => {
              const tone = runTone(run);
              const dotClass =
                tone === 'ok'
                  ? 'ck-run-dot ck-run-dot--ok'
                  : tone === 'error'
                    ? 'ck-run-dot ck-run-dot--error'
                    : tone === 'running'
                      ? 'ck-run-dot ck-run-dot--running'
                      : 'ck-run-dot';
              const label =
                tone === 'ok'
                  ? '成功'
                  : tone === 'error'
                    ? `失败（${run.conclusion}）`
                    : tone === 'running'
                      ? '运行中'
                      : run.conclusion ?? run.status;
              return (
                <div
                  key={run.databaseId}
                  className="ck-run-row"
                  role="link"
                  tabIndex={0}
                  onClick={() => void openUrl(run.url)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      void openUrl(run.url);
                    }
                  }}
                >
                  <span className={dotClass} />
                  <span className="ck-run-title">{run.displayTitle || 'Daily Checkin'}</span>
                  <span className="ck-run-meta">{label}</span>
                  <span className="ck-run-meta">
                    {run.event === 'workflow_dispatch' ? '手动' : '定时'} · {formatRelative(run.createdAt)}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      ) : null}
    </section>
  );
}
