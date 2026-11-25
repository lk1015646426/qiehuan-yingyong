import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const page = (name: string) => readFileSync(new URL(`../pages/${name}`, import.meta.url), 'utf8');
const style = (name: string) => readFileSync(new URL(`../styles/pages/${name}`, import.meta.url), 'utf8');

const layout = readFileSync(new URL('../components/AccountManagementLayout.tsx', import.meta.url), 'utf8');
const providers = [
  page('WorkCnSwitcherPage.tsx'),
  page('WorkBuddyPage.tsx'),
  readFileSync(new URL('../components/doubao/DoubaoPoolPanel.tsx', import.meta.url), 'utf8'),
];

test('三个账号页都由公共组件渲染顶栏、卡片字段、操作和详情', () => {
  for (const source of providers) {
    for (const component of ['AccountManagementLayout', 'AccountStatusBanner', 'AccountList', 'AccountCardFrame', 'AccountCardActions']) {
      assert.match(source, new RegExp('<' + component + '\\b'), component + ' 应由页面直接渲染');
    }
    assert.doesNotMatch(source, /className="[^"]*account-card__(?:head|identity|actions|metrics)\b/);
  }
  assert.deepEqual([...layout.matchAll(/<dt>([^<]+)<\/dt>/g)].map(match => match[1]), [
    '可用积分', '凭证剩余', '今日奖励', '连续签到',
  ]);
  assert.match(layout, /UID \{uid \? compactUid\(uid\) : '未提供'\}/);
  assert.match(providers[0], /uid=\{account\.userId\}/);
  assert.match(providers[1], /uid=\{account\.uid\}/);
  assert.match(layout, /tokenExpiresAt === undefined \? '未提供'/);
  assert.match(layout, /<summary>账号详情<\/summary>/);
  assert.match(layout, /<span>令牌到期 <strong>\{expiry\}<\/strong>/);
});

test('顶栏和卡片操作按 WorkBuddy 的位置和顺序渲染', () => {
  const toolbar = layout.slice(layout.indexOf('<header'), layout.indexOf('</header>'));
  assert.deepEqual([...toolbar.matchAll(/<AccountButton[^>]* label="([^"]+)"/g)].map(match => match[1]), [
    '导入当前账号', '同步 GitHub', '刷新全部积分', '本地一键签到', '路径设置', '重新检测',
  ]);
  const actions = layout.slice(layout.indexOf('export function AccountCardActions'), layout.indexOf('export type AccountCheckinResult'));
  assert.match(actions, /className="uac-primary"[\s\S]*'切换并打开'[\s\S]*label="客户端签到"[\s\S]*className="uac-management"[\s\S]*label="刷新积分"[\s\S]*label="备注"[\s\S]*'删除'/);
  assert.match(layout, /action=\{recovery\} label="恢复认证"[\s\S]*action=\{cloudCheckin\} label="云端签到"/);
});

test('三页的指标和按钮网格共用同一紧凑样式', () => {
  const css = style('workbuddy.css');
  assert.match(layout, /account-card--compact/);
  assert.match(layout, /className="wc-slots wb-slots"/);
  assert.match(layout, /className="wc-page workbuddy-page"/);
  assert.match(style('work-cn.css'), /\.account-card\.account-card--compact\s*\{[^}]*display:\s*grid/s);
  for (const name of ['uac-metrics', 'uac-quick-status']) {
    assert.match(css, new RegExp('\\.' + name + '\\s*\\{[^}]*grid-template-columns:\\s*repeat\\(2,'));
  }
  assert.match(css, /\.uac-actions\s*\{[^}]*grid-template-columns:\s*minmax\(0, 1fr\);/s);
  assert.doesNotMatch(css, /:has\(\.uac-checkin\)/);
});

test('账号卡片的高度由内容决定', () => {
  const css = [style('work-cn.css'), style('workbuddy.css'), style('checkin.css')].join('\n');
  assert.doesNotMatch(css, /(?:account-card|wb-slot)[^{]*\{[^}]*min-height:\s*250px/s);
  assert.doesNotMatch(css, /(?:account-card__actions|wc-slot-actions|ck-card-actions)[^{]*\{[^}]*margin-top:\s*auto/s);
});

test('本地签到、积分查询和云端签到各自接到实际的入口', () => {
  const [trae, workBuddy, doubao] = providers;
  assert.match(workBuddy, /invoke\('open_workbuddy_checkin_client', \{ accountId: account\.id \}\)/);
  assert.match(workBuddy, /checkin=\{\{[\s\S]*onClick: \(\) => void openClientCheckin\(\), disabled: busy \|\| openingCheckin \|\| !active/);
  assert.match(workBuddy, /cloudCheckin=\{\{[\s\S]*onClick: \(\) => void triggerCheckin\(account\.id\)/);
  assert.match(trae, /await localCheckin\(account\.id\)/);
  assert.match(trae, /await useCheckinStore\.getState\(\)\.localCheckin\(account\.id\)/);
  assert.match(trae, /patch_general_config.*updates/);
  assert.match(layout, /disabled=\{!action\.onClick \|\| action\.disabled\}/);
  assert.match(doubao, /switchAccount=\{\{ title: '[^']+' \}\}/);
  assert.match(doubao, /checkin=\{\{ title: '[^']+' \}\}/);
  assert.match(doubao, /invoke<DoubaoQuota>\('get_doubao_account_quota', \{ accountId \}\)/);
  assert.match(doubao, /credits=\{<span[^>]*>[\s\S]*quotaCaption\(quota\)/);
});
