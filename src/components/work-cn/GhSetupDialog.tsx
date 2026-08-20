// GitHub 上传条件引导弹窗：自动检测三项前置条件并就地补全。
//
// 三项条件：
//   1. gh CLI 已安装 —— 缺失时「自动下载并安装」（官方 MSI，进度实时推送）；
//   2. gh 已登录 —— 缺失时填写 PAT（只走 stdin，绝不落盘）；
//   3. 同步配置（启用 + 仓库 + 槽位绑定）—— 就地填写保存。
// 每完成一步自动重新检测，全部通过后即可一键同步 GitHub Secrets。

import { useEffect, useState } from 'react';
import { useWorkCnStore } from '../../stores/useWorkCnStore';
import { openExternalUrl } from '../../services/workCnService';
import { accountSecretStem } from '../../utils/accountNaming';
import type { WorkCnGitHubSlot } from '../../types/workCn';

interface Props {
  open: boolean;
  onClose: () => void;
}

// PAT 创建页：预勾选 repo（Secrets 读写必需）与 workflow（触发签到用）。
const PAT_URL =
  'https://github.com/settings/tokens/new?scopes=repo,workflow&description=TRAE-checkin-sync';

function formatBytes(n: number): string {
  if (!n) return '';
  const mb = n / (1024 * 1024);
  return mb >= 1 ? `${mb.toFixed(1)} MB` : `${(n / 1024).toFixed(0)} KB`;
}

export function GhSetupDialog({ open, onClose }: Props) {
  const accounts = useWorkCnStore((s) => s.accounts);
  const githubConfig = useWorkCnStore((s) => s.githubConfig);
  const githubCliStatus = useWorkCnStore((s) => s.githubCliStatus);
  const loadGitHubConfig = useWorkCnStore((s) => s.loadGitHubConfig);
  const saveGitHubConfig = useWorkCnStore((s) => s.saveGitHubConfig);
  const ghSetup = useWorkCnStore((s) => s.ghSetup);
  const ghSetupError = useWorkCnStore((s) => s.ghSetupError);
  const setupGh = useWorkCnStore((s) => s.setupGh);
  const ghLogin = useWorkCnStore((s) => s.ghLogin);

  const [pat, setPat] = useState('');
  const [loggingIn, setLoggingIn] = useState(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [repository, setRepository] = useState('');
  const [boundIds, setBoundIds] = useState<Set<string>>(new Set());
  const [savingConfig, setSavingConfig] = useState(false);
  const [configError, setConfigError] = useState<string | null>(null);

  // 打开时只拉取最新配置；CLI 状态由页面挂载时检测（避免重复 spawn gh 进程），
  // 安装/登录完成后 store 会自动刷新。
  useEffect(() => {
    if (!open) return;
    void loadGitHubConfig();
  }, [open, loadGitHubConfig]);

  useEffect(() => {
    if (!open) return;
    setEnabled(githubConfig.enabled);
    setRepository(githubConfig.repository);
    setBoundIds(new Set(githubConfig.slots.map((slot) => slot.accountId)));
    setConfigError(null);
  }, [open, githubConfig]);

  // 任意一步成功后自动复检（PAT 登录 / 安装完成后 store 已刷新 CLI 状态）。
  useEffect(() => {
    if (!open) return;
    const allReady =
      githubCliStatus?.available &&
      githubCliStatus.authed &&
      githubConfig.enabled &&
      /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(githubConfig.repository) &&
      githubConfig.slots.length > 0;
    if (allReady) {
      setPat('');
      setLoginError(null);
      setConfigError(null);
    }
  }, [open, githubCliStatus, githubConfig]);

  if (!open) {
    return null;
  }

  const installing =
    ghSetup?.phase === 'downloading' || ghSetup?.phase === 'installing';
  const installPercent =
    ghSetup?.phase === 'downloading' && ghSetup.total > 0
      ? Math.min(100, Math.round((ghSetup.received / ghSetup.total) * 100))
      : null;

  const handleLogin = async () => {
    setLoginError(null);
    setLoggingIn(true);
    try {
      await ghLogin(pat);
      setPat('');
    } catch (err) {
      setLoginError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoggingIn(false);
    }
  };

  // 槽位号按账号列表顺序自动分配（1,2,3…），勾选即绑定，无数量上限。
  const slotNumbers = new Map<string, number>();
  {
    let n = 1;
    for (const account of accounts) {
      if (boundIds.has(account.id)) slotNumbers.set(account.id, n++);
    }
  }

  const handleSaveConfig = async () => {
    setConfigError(null);
    const slots: WorkCnGitHubSlot[] = [];
    for (const account of accounts) {
      const slot = slotNumbers.get(account.id);
      if (slot !== undefined) {
        slots.push({ slot, accountId: account.id, tokenSecret: '', deviceSecret: '' });
      }
    }
    if (enabled && !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository.trim())) {
      setConfigError('仓库需为 owner/repo 形式');
      return;
    }
    if (enabled && slots.length === 0) {
      setConfigError('启用同步时至少要绑定一个账号到槽位');
      return;
    }
    setSavingConfig(true);
    try {
      await saveGitHubConfig({
        enabled,
        repository: repository.trim(),
        slots,
        workflowFile: githubConfig.workflowFile || 'daily-checkin.yml',
      });
    } catch (err) {
      setConfigError(err instanceof Error ? err.message : String(err));
    } finally {
      setSavingConfig(false);
    }
  };

  const configReady =
    githubConfig.enabled &&
    /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(githubConfig.repository) &&
    githubConfig.slots.length > 0;

  const rowDone = (ok: boolean) =>
    ok ? 'gh-row gh-row--ok' : 'gh-row gh-row--todo';

  return (
    <div className="wc-overlay" onClick={installing ? undefined : onClose}>
      <div className="wc-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="wc-dialog-title">GitHub 上传准备</h3>
        <p className="wc-dialog-note">
          同步凭证到 GitHub Secrets 需要满足以下条件，缺少的项目会在这里就地补全。
        </p>

        {/* 条件 1：gh CLI 安装 */}
        <div className={rowDone(!!githubCliStatus?.available)}>
          <div className="gh-row-main">
            <span className="gh-row-title">① 安装 GitHub CLI（gh）</span>
            <span className="gh-row-state">
              {githubCliStatus
                ? githubCliStatus.available
                  ? '已安装'
                  : '未安装'
                : '检测中…'}
            </span>
          </div>
          {!githubCliStatus?.available ? (
            <div className="gh-row-body">
              {installing ? (
                <div className="gh-progress">
                  <div className="gh-progress-text">
                    {ghSetup?.phase === 'installing'
                      ? '正在安装（请在系统弹出的 UAC 窗口点「是」）…'
                      : installPercent !== null
                        ? `下载中 ${installPercent}%（${formatBytes(ghSetup!.received)} / ${formatBytes(ghSetup!.total)}）`
                        : '正在下载…'}
                  </div>
                  <div className="gh-progress-bar">
                    <div
                      className="gh-progress-fill"
                      style={{ width: `${ghSetup?.phase === 'installing' ? 100 : installPercent ?? 4}%` }}
                    />
                  </div>
                </div>
              ) : (
                <div className="gh-row-actions">
                  <button
                    type="button"
                    className="wc-btn wc-btn-primary"
                    onClick={() => void setupGh()}
                    disabled={loggingIn || savingConfig}
                  >
                    自动下载并安装
                  </button>
                  <button
                    type="button"
                    className="gh-link gh-link--btn"
                    onClick={() => void openExternalUrl('https://cli.github.com/')}
                  >
                    打开官方下载页
                  </button>
                </div>
              )}
              {ghSetupError ? <div className="gh-error">{ghSetupError}</div> : null}
            </div>
          ) : null}
        </div>

        {/* 条件 2：gh 登录 */}
        <div className={rowDone(!!githubCliStatus?.authed)}>
          <div className="gh-row-main">
            <span className="gh-row-title">② 登录 GitHub 账号（gh）</span>
            <span className="gh-row-state">
              {githubCliStatus
                ? githubCliStatus.authed
                  ? '已登录'
                  : '未登录'
                : '检测中…'}
            </span>
          </div>
          {githubCliStatus?.available && !githubCliStatus.authed ? (
            <div className="gh-row-body">
              <div className="gh-row-actions">
                <input
                  className="wc-input gh-pat-input"
                  type="password"
                  value={pat}
                  onChange={(e) => setPat(e.target.value)}
                  placeholder="粘贴 GitHub Token（ghp_… 或 github_pat_…）"
                  disabled={loggingIn || installing}
                />
                <button
                  type="button"
                  className="wc-btn wc-btn-primary"
                  onClick={() => void handleLogin()}
                  disabled={loggingIn || installing || !pat.trim()}
                >
                  {loggingIn ? '登录中…' : '登录'}
                </button>
              </div>
              <p className="wc-dialog-note">
                还没有 Token？
                <button
                  type="button"
                  className="gh-link gh-link--btn"
                  onClick={() => void openExternalUrl(PAT_URL)}
                >
                  点此创建
                </button>
                （已预勾选 repo + workflow 权限，生成后粘贴到上面）。
              </p>
              {loginError ? <div className="gh-error">{loginError}</div> : null}
            </div>
          ) : null}
        </div>

        {/* 条件 3：同步配置 */}
        <div className={rowDone(configReady)}>
          <div className="gh-row-main">
            <span className="gh-row-title">③ 同步配置（仓库 + 槽位绑定）</span>
            <span className="gh-row-state">{configReady ? '已配置' : '未配置'}</span>
          </div>
          {!configReady ? (
            <div className="gh-row-body">
              <label className="gh-row-actions" style={{ marginTop: 0 }}>
                <input
                  type="checkbox"
                  checked={enabled}
                  onChange={(e) => setEnabled(e.target.checked)}
                />
                <span>启用 GitHub Secrets 同步</span>
              </label>
              <input
                className="wc-input"
                value={repository}
                onChange={(e) => setRepository(e.target.value)}
                placeholder="GitHub 仓库（owner/repo）"
                disabled={!enabled || savingConfig}
              />
              {accounts.length === 0 ? (
                <p className="wc-dialog-note">还没有已导入的账号，请先到「账号切换」页导入。</p>
              ) : (
                accounts.map((account) => {
                  const slot = slotNumbers.get(account.id);
                  return (
                    <div key={account.id} className="wc-slot-row">
                      <label style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, minWidth: 0 }}>
                        <input
                          type="checkbox"
                          checked={slot !== undefined}
                          disabled={!enabled || savingConfig}
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
                        <span className="wc-slot-row-slot">{accountSecretStem(account)}_TOKEN</span>
                      ) : (
                        <span className="wc-slot-row-slot wc-slot-row-slot--unbound">未绑定</span>
                      )}
                    </div>
                  );
                })
              )}
              <div className="gh-row-actions" style={{ marginTop: 8 }}>
                <button
                  type="button"
                  className="wc-btn wc-btn-primary"
                  onClick={() => void handleSaveConfig()}
                  disabled={savingConfig || installing}
                >
                  {savingConfig ? '保存中…' : '保存配置'}
                </button>
              </div>
              {configError ? <div className="gh-error">{configError}</div> : null}
            </div>
          ) : null}
        </div>

        <div className="wc-dialog-actions">
          <button
            type="button"
            className="wc-btn"
            onClick={onClose}
            disabled={installing}
          >
            {installing ? '安装中…' : '关闭'}
          </button>
        </div>
      </div>
    </div>
  );
}
