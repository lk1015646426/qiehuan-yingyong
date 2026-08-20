import assert from 'node:assert/strict';
import test from 'node:test';

import { filterCheckinProducts, groupCheckinProducts, checkinProductLabel } from './checkinProducts.ts';
import type { CheckinAccount } from '../types/checkin.ts';

const items = [
  { product: 'trae', account: { id: 'trae-1' } },
  { product: 'workbuddy', account: { id: 'wb-1' } },
] as unknown as CheckinAccount[];

test('签到产品筛选保留原始顺序', () => {
  assert.deepEqual(filterCheckinProducts(items, 'all'), items);
  assert.equal(filterCheckinProducts(items, 'trae')[0].product, 'trae');
  assert.equal(filterCheckinProducts(items, 'workbuddy')[0].product, 'workbuddy');
  assert.deepEqual(filterCheckinProducts([], 'all'), []);
});

test('签到产品标签明确区分产品', () => {
  assert.equal(checkinProductLabel('all'), '全部');
  assert.equal(checkinProductLabel('trae'), 'TRAE');
  assert.equal(checkinProductLabel('workbuddy'), 'WorkBuddy');
});

test('全部视图按 TRAE 和 WorkBuddy 分组并保持账号顺序', () => {
  const mixed = [items[1], items[0], { product: 'trae', account: { id: 'trae-2' } }] as unknown as CheckinAccount[];
  const groups = groupCheckinProducts(mixed, 'all');

  assert.deepEqual(groups.map((group) => group.product), ['trae', 'workbuddy']);
  assert.deepEqual(groups[0].items.map((item) => item.account.id), ['trae-1', 'trae-2']);
  assert.deepEqual(groups[1].items.map((item) => item.account.id), ['wb-1']);
  assert.deepEqual(groupCheckinProducts(mixed, 'workbuddy').map((group) => group.product), ['workbuddy']);
  assert.deepEqual(groupCheckinProducts([], 'all'), []);
});
