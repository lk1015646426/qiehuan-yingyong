import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const page = (name: string) => readFileSync(new URL(`../pages/${name}`, import.meta.url), 'utf8');
const style = (name: string) => readFileSync(new URL(`../styles/pages/${name}`, import.meta.url), 'utf8');

test('TRAE 与 WorkBuddy 使用相同卡片结构区域', () => {
  const sources = [
    page('WorkCnSwitcherPage.tsx'),
    page('WorkBuddyPage.tsx'),
  ];
  for (const source of sources) {
    for (const className of [
      'account-card',
      'account-card__head',
      'account-card__identity',
      'account-card__metrics',
      'account-card__actions',
    ]) {
      assert.equal(source.includes(className), true, `${className} 应出现在每个账号页面`);
    }
  }
});

test('本地 TRAE 与 WorkBuddy 卡片都展示 UID', () => {
  assert.match(page('WorkCnSwitcherPage.tsx'), /UID/);
  assert.match(page('WorkBuddyPage.tsx'), /UID/);
});

test('本地 TRAE 与 WorkBuddy 卡片使用同一个紧凑尺寸契约', () => {
  const sources = [
    page('WorkCnSwitcherPage.tsx'),
    page('WorkBuddyPage.tsx'),
  ];
  for (const source of sources) {
    assert.match(source, /account-card--compact/);
  }
});

test('账号卡片不再用固定最小高度或自动外边距制造空白', () => {
  const css = [style('work-cn.css'), style('workbuddy.css'), style('checkin.css')].join('\n');
  assert.doesNotMatch(css, /(?:account-card|wb-slot)[^{]*\{[^}]*min-height:\s*250px/s);
  assert.doesNotMatch(css, /(?:account-card__actions|wc-slot-actions|ck-card-actions)[^{]*\{[^}]*margin-top:\s*auto/s);
});

test('共享紧凑网格的优先级高于页面卡片基础样式', () => {
  assert.match(style('work-cn.css'), /\.account-card\.account-card--compact\s*\{[^}]*display:\s*grid/s);
});
