// 账号槽位命名工具：卡片显示名 + GitHub secret 名主干。
// secret 主干规则与后端 modules/work_cn_github.rs 的 slot_secret_stem 保持一致，
// 两边任何一边改动都必须同步另一边。

import type { WorkCnAccountView } from '../types/workCn';

export function accountDisplayName(account: WorkCnAccountView): string {
  return account.tags?.length
    ? account.tags[0]
    : account.nickname ?? account.email ?? account.userId ?? account.id;
}

// 标签 → 昵称 → 邮箱前缀 → user_id → 账号 id 依次回退；纯非 ASCII 名清洗为空。
export function accountSecretStem(account: WorkCnAccountView): string {
  const emailLocal = account.email?.split('@')[0] ?? '';
  const candidates = [
    account.tags?.[0] ?? '',
    account.nickname ?? '',
    emailLocal,
    account.userId ?? '',
    account.id,
  ];
  for (const candidate of candidates) {
    const stem = sanitizeSecretStem(candidate);
    if (stem) {
      return stem;
    }
  }
  return 'ACCOUNT';
}

// 大写化、非法字符折叠为单个 _、去首尾 _；GitHub 禁止数字或 GITHUB_ 开头，补 A_。
function sanitizeSecretStem(raw: string): string {
  const s = raw
    .trim()
    .replace(/[^A-Za-z0-9]+/g, '_')
    .toUpperCase()
    .replace(/^_+|_+$/g, '');
  if (!s) {
    return '';
  }
  return /^[0-9]/.test(s) || s.startsWith('GITHUB_') ? `A_${s}` : s;
}
