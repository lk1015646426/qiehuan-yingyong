// TRAE Work CN — GitHub Secrets 同步设置对话框（开发指南 §8.5 / 阶段 6）。
//
// 只保存仓库 owner/repo 与「账号 → 槽位」映射，绝不保存 GitHub PAT；secret 名
// 默认自动生成为 TRAE{N}_TOKEN / TRAE{N}_DEVICE_ID。真实同步走已登录的 gh CLI。

import { useEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import { useWorkCnStore } from '../../stores/useWorkCnStore';
import type { WorkCnGitHubSlot } from '../../types/workCn';

interface Props {
  open: boolean;
  onClose: () => void;
}

const overlayStyle: CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'rgba(15, 23, 42, 0.35)',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 1000,
};

const dialogStyle: CSSProperties = {
  width: 520,
  maxWidth: '92vw',
  maxHeight: '88vh',
  overflowY: 'auto',
  background: '#ffffff',
  borderRadius: 12,
  padding: '22px 24px',
  boxShadow: '0 12px 40px rgba(15, 23, 42, 0.25)',
  boxSizing: 'border-box',
};

const inputStyle: CSSProperties = {
  width: '100%',
  height: 34,
  padding: '0 10px',
  fontSize: 13,
  border: '1px solid #d7dae0',
  borderRadius: 6,
  boxSizing: 'border-box',
  marginBottom: 4,
};

const actionsStyle: CSSProperties = {
  display: 'flex',
  justifyContent: 'flex-end',
  gap: 8,
  marginTop: 16,
};

const buttonStyle: CSSProperties = {
  height: 34,
  padding: '0 16px',
  fontSize: 13,
  color: '#1f2430',
  background: '#f1f3f6',
  border: '1px solid #d7dae0',
  borderRadius: 6,
  cursor: 'pointer',
};

const primaryStyle: CSSProperties = {
  color: '#ffffff',
  background: '#2563eb',
  border: '1px solid #2563eb',
};

const errorStyle: CSSProperties = {
  marginTop: 12,
  fontSize: 12,
  color: '#ef4444',
  background: '#fef2f2',
  border: '1px solid #fecaca',
  borderRadius: 6,
  padding: '8px 10px',
};

const noteStyle: CSSProperties = {
  fontSize: 12,
  color: '#8a909c',
  lineHeight: 1.6,
  marginTop: 4,
};

export function WorkCnSettingsDialog({ open, onClose }: Props) {
  const accounts = useWorkCnStore((s) => s.accounts);
  const githubConfig = useWorkCnStore((s) => s.githubConfig);
  const githubCliStatus = useWorkCnStore((s) => s.githubCliStatus);
  const loadGitHubConfig = useWorkCnStore((s) => s.loadGitHubConfig);
  const saveGitHubConfig = useWorkCnStore((s) => s.saveGitHubConfig);
  const refreshGitHubCliStatus = useWorkCnStore((s) => s.refreshGitHubCliStatus);

  const [enabled, setEnabled] = useState(false);
  const [repository, setRepository] = useState('');
  const [slotByAccount, setSlotByAccount] = useState<Record<string, number>>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    void loadGitHubConfig();
    void refreshGitHubCliStatus();
  }, [open, loadGitHubConfig, refreshGitHubCliStatus]);

  useEffect(() => {
    if (!open) return;
    setEnabled(githubConfig.enabled);
    setRepository(githubConfig.repository);
    const map: Record<string, number> = {};
    for (const slot of githubConfig.slots) {
      map[slot.accountId] = slot.slot;
    }
    setSlotByAccount(map);
    setError(null);
  }, [open, githubConfig]);

  if (!open) {
    return null;
  }

  const buildConfig = (): { config: import('../../types/workCn').WorkCnGitHubConfig; dup: boolean } => {
    const slots: WorkCnGitHubSlot[] = [];
    const seenSlots = new Set<number>();
    for (const account of accounts) {
      const slot = slotByAccount[account.id] ?? 0;
      if (slot >= 1 && slot <= 4) {
        if (seenSlots.has(slot)) return { config: githubConfig, dup: true };
        seenSlots.add(slot);
        slots.push({
          slot,
          accountId: account.id,
          tokenSecret: '',
          deviceSecret: '',
        });
      }
    }
    return {
      config: { enabled, repository: repository.trim(), slots },
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

  const cliText = !githubCliStatus
    ? '检测中…'
    : !githubCliStatus.available
      ? '未检测到 GitHub CLI（gh）'
      : githubCliStatus.authed
        ? 'gh 已登录'
        : `gh 未登录${githubCliStatus.detail ? `：${githubCliStatus.detail}` : ''}`;

  return (
    <div style={overlayStyle} onClick={onClose}>
      <div style={dialogStyle} onClick={(e) => e.stopPropagation()}>
        <h3 style={{ margin: '0 0 12px', fontSize: 16 }}>GitHub Secrets 同步设置</h3>

        <label style={rowLabelStyle}>
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
            style={{ marginRight: 8 }}
          />
          启用 GitHub Secrets 同步
        </label>
        <p style={noteStyle}>
          把每个账号的最新签到凭证（access token + 设备 ID）同步到 GitHub Actions 仓库的 Secrets。
          本软件只同步凭证、绝不在本地签到。
        </p>

        <label style={rowLabelStyle}>GitHub 仓库（owner/repo）</label>
        <input
          value={repository}
          onChange={(e) => setRepository(e.target.value)}
          placeholder="lk1015646426/daily-checkin"
          disabled={!enabled}
          style={inputStyle}
        />

        <div style={{ marginTop: 12, fontSize: 13, color: '#1f2430' }}>
          GitHub CLI 状态：<b>{cliText}</b>
        </div>

        <div style={{ marginTop: 12 }}>
          <div style={rowLabelStyle}>账号 → 槽位绑定（secret 名自动为 TRAE{'{N}'}_TOKEN / TRAE{'{N}'}_DEVICE_ID）</div>
          {accounts.length === 0 ? (
            <div style={noteStyle}>还没有已导入的账号。请先在主页导入账号。</div>
          ) : (
            accounts.map((account) => (
              <div key={account.id} style={slotRowStyle}>
                <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {account.tags?.[0] ?? account.nickname ?? account.email ?? account.userId ?? account.id}
                </span>
                <select
                  value={slotByAccount[account.id] ?? 0}
                  disabled={!enabled}
                  onChange={(e) =>
                    setSlotByAccount((prev) => ({
                      ...prev,
                      [account.id]: Number(e.target.value),
                    }))
                  }
                  style={{ height: 30, fontSize: 13 }}
                >
                  <option value={0}>未绑定</option>
                  <option value={1}>槽位 1</option>
                  <option value={2}>槽位 2</option>
                  <option value={3}>槽位 3</option>
                  <option value={4}>槽位 4</option>
                </select>
              </div>
            ))
          )}
        </div>

        {error ? <div style={errorStyle}>{error}</div> : null}
        <div style={actionsStyle}>
          <button type="button" style={buttonStyle} onClick={onClose} disabled={saving}>
            取消
          </button>
          <button
            type="button"
            style={{ ...buttonStyle, ...primaryStyle }}
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

const rowLabelStyle: CSSProperties = {
  fontSize: 13,
  color: '#1f2430',
  display: 'block',
  marginBottom: 6,
  marginTop: 10,
};

const slotRowStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 10,
  padding: '6px 0',
  borderBottom: '1px solid #f0f1f4',
};
