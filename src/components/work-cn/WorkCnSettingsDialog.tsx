// TRAE Work CN — GitHub Secrets 同步设置对话框（开发指南 §8.5 / 阶段 6）。
//
// 只保存仓库 owner/repo 与「账号 → 槽位」映射，绝不保存 GitHub PAT；secret 名
// 默认自动生成为 {账号槽位名}_TOKEN / {账号槽位名}_DEVICE_ID。真实同步走已登录的 gh CLI。

import { useEffect, useState } from 'react';
import { useWorkCnStore } from '../../stores/useWorkCnStore';
import { accountSecretStem } from '../../utils/accountNaming';
import type { WorkCnGitHubSlot } from '../../types/workCn';

interface Props {
  open: boolean;
  onClose: () => void;
}

export function WorkCnSettingsDialog({ open, onClose }: Props) {
  const accounts = useWorkCnStore((s) => s.accounts);
  const githubConfig = useWorkCnStore((s) => s.githubConfig);
  const githubCliStatus = useWorkCnStore((s) => s.githubCliStatus);
  const loadGitHubConfig = useWorkCnStore((s) => s.loadGitHubConfig);
  const saveGitHubConfig = useWorkCnStore((s) => s.saveGitHubConfig);
  const refreshGitHubCliStatus = useWorkCnStore((s) => s.refreshGitHubCliStatus);
  const clearCredentials = useWorkCnStore((s) => s.clearCredentials);

  const [enabled, setEnabled] = useState(false);
  const [repository, setRepository] = useState('');
  const [workflowFile, setWorkflowFile] = useState('daily-checkin.yml');
  const [boundIds, setBoundIds] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearing, setClearing] = useState(false);

  useEffect(() => {
    if (!open) return;
    void loadGitHubConfig();
    void refreshGitHubCliStatus();
  }, [open, loadGitHubConfig, refreshGitHubCliStatus]);

  useEffect(() => {
    if (!open) return;
    setEnabled(githubConfig.enabled);
    setRepository(githubConfig.repository);
    setWorkflowFile(githubConfig.workflowFile || 'daily-checkin.yml');
    setBoundIds(new Set(githubConfig.slots.map((slot) => slot.accountId)));
    setError(null);
  }, [open, githubConfig]);

  if (!open) {
    return null;
  }

  // 槽位号按账号列表顺序自动分配（1,2,3…），勾选即绑定，无数量上限。
  const slotNumbers = new Map<string, number>();
  {
    let n = 1;
    for (const account of accounts) {
      if (boundIds.has(account.id)) slotNumbers.set(account.id, n++);
    }
  }

  const buildConfig = (): { config: import('../../types/workCn').WorkCnGitHubConfig; dup: boolean } => {
    const slots: WorkCnGitHubSlot[] = [];
    for (const account of accounts) {
      const slot = slotNumbers.get(account.id);
      if (slot !== undefined) {
        slots.push({
          slot,
          accountId: account.id,
          tokenSecret: '',
          deviceSecret: '',
          // 保留已保存的自动签到开关：设置弹窗重建槽位列表，不读旧值会把
          // 用户关闭过的开关悄悄重置为开启。
          checkinEnabled:
            githubConfig.slots.find((s) => s.accountId === account.id)?.checkinEnabled ?? true,
        });
      }
    }
    return {
      config: {
        enabled,
        repository: repository.trim(),
        slots,
        workflowFile: workflowFile.trim() || 'daily-checkin.yml',
      },
      dup: false,
    };
  };

  const handleSave = async () => {
    setError(null);
    const { config, dup } = buildConfig();
    if (dup) {
      setError('同一个槽位不能绑定多个账号');
      return;
    }
    if (enabled && !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(config.repository)) {
      setError('仓库需为 owner/repo 形式');
      return;
    }
    setSaving(true);
    try {
      await saveGitHubConfig(config);
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  const handleClear = async () => {
    setError(null);
    setClearing(true);
    try {
      await clearCredentials();
      setConfirmClear(false);
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setClearing(false);
    }
  };

  const cliText = !githubCliStatus
    ? '检测中…'
    : !githubCliStatus.available
      ? '未检测到 GitHub CLI（gh）'
      : githubCliStatus.authed
        ? 'gh 已登录'
        : `gh 未登录${githubCliStatus.detail ? `：${githubCliStatus.detail}` : ''}`;

  return (
    <div className="wc-overlay" onClick={onClose}>
      <div className="wc-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="wc-dialog-title">GitHub Secrets 同步设置</h3>

        <label className="wc-row-label" style={{ marginTop: 0, display: 'flex', alignItems: 'center' }}>
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
            style={{ marginRight: 8 }}
          />
          启用 GitHub Secrets 同步
        </label>
        <p className="wc-dialog-note">
          把每个账号的最新签到凭证（access token + 设备 ID）同步到 GitHub Actions 仓库的 Secrets。
          本软件只同步凭证、绝不在本地签到。
        </p>

        <label className="wc-row-label">GitHub 仓库（owner/repo）</label>
        <input
          value={repository}
          onChange={(e) => setRepository(e.target.value)}
          placeholder="lk1015646426/daily-checkin"
          disabled={!enabled}
          className="wc-input"
        />

        <label className="wc-row-label">Workflow 文件名</label>
        <input
          value={workflowFile}
          onChange={(e) => setWorkflowFile(e.target.value)}
          placeholder="daily-checkin.yml"
          disabled={!enabled}
          className="wc-input"
        />
        <p className="wc-dialog-note">
          云端签到仓库中 workflow 文件名（.github/workflows/ 目录下），用于触发签到与查询运行状态。
        </p>

        <div style={{ marginTop: 12, fontSize: 13 }}>
          GitHub CLI 状态：<b>{cliText}</b>
        </div>

        <div style={{ marginTop: 12 }}>
          <div className="wc-row-label">账号 → 槽位绑定（按列表顺序自动编号，secret 名自动为「账号名_TOKEN / 账号名_DEVICE_ID」，数量不限）</div>
          {accounts.length === 0 ? (
            <div className="wc-dialog-note">还没有已导入的账号。请先在主页导入账号。</div>
          ) : (
            accounts.map((account) => {
              const slot = slotNumbers.get(account.id);
              return (
                <div key={account.id} className="wc-slot-row">
                  <label style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, minWidth: 0 }}>
                    <input
                      type="checkbox"
                      checked={slot !== undefined}
                      disabled={!enabled}
                      onChange={(e) =>
                        setBoundIds((prev) => {
                          const next = new Set(prev);
                          if (e.target.checked) next.add(account.id);
                          else next.delete(account.id);
                          return next;
                        })
                      }
                    />
                    <span className="wc-slot-row-name">
                      {account.tags?.[0] ?? account.nickname ?? account.email ?? account.userId ?? account.id}
                    </span>
                  </label>
                  {slot !== undefined ? (
                    <span className="wc-slot-row-slot">槽位 {slot} · {accountSecretStem(account)}_TOKEN</span>
                  ) : (
                    <span className="wc-slot-row-slot wc-slot-row-slot--unbound">未绑定</span>
                  )}
                </div>
              );
            })
          )}
        </div>

        <div className="wc-danger-zone">
          <div className="wc-danger-title">危险操作</div>
          <p className="wc-dialog-note">
            清除本地保存的全部账号凭证与 GitHub 同步配置。此操作不可撤销，清除后需重新导入账号。
          </p>
          {confirmClear ? (
            <div className="wc-danger-confirm">
              <div className="wc-danger-confirm-text">
                确认要清除全部本地凭证吗？此操作不可撤销。
              </div>
              <div className="wc-danger-confirm-actions">
                <button
                  type="button"
                  className="wc-btn"
                  onClick={() => setConfirmClear(false)}
                  disabled={clearing}
                >
                  取消
                </button>
                <button
                  type="button"
                  className="wc-btn wc-btn-danger"
                  onClick={handleClear}
                  disabled={clearing}
                >
                  {clearing ? '清除中…' : '确认清除'}
                </button>
              </div>
            </div>
          ) : (
            <button
              type="button"
              className="wc-btn wc-btn-danger"
              onClick={() => setConfirmClear(true)}
            >
              清除本地凭证
            </button>
          )}
        </div>

        {error ? <div className="wc-dialog-error">{error}</div> : null}
        <div className="wc-dialog-actions">
          <button type="button" className="wc-btn" onClick={onClose} disabled={saving}>
            取消
          </button>
          <button
            type="button"
            className="wc-btn wc-btn-primary"
            onClick={handleSave}
            disabled={saving}
          >
            {saving ? '保存中…' : '保存'}
          </button>
        </div>
      </div>
    </div>
  );
}
