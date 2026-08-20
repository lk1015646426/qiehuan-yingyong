import type { CheckinAccount, CheckinProduct } from '../types/checkin';

export function filterCheckinProducts(items: CheckinAccount[], product: CheckinProduct): CheckinAccount[] {
  if (product === 'all') return items;
  return items.filter((item) => item.product === product);
}

export interface CheckinProductGroup {
  product: Exclude<CheckinProduct, 'all'>;
  items: CheckinAccount[];
}

export function groupCheckinProducts(items: CheckinAccount[], product: CheckinProduct): CheckinProductGroup[] {
  const products: Array<Exclude<CheckinProduct, 'all'>> = product === 'all'
    ? ['trae', 'workbuddy']
    : [product];
  return products
    .map((groupProduct) => ({
      product: groupProduct,
      items: items.filter((item) => item.product === groupProduct),
    }))
    .filter((group) => group.items.length > 0);
}

export function checkinProductLabel(product: CheckinProduct): string {
  if (product === 'trae') return 'TRAE';
  if (product === 'workbuddy') return 'WorkBuddy';
  return '全部';
}

export function checkinProductIconAlt(product: Exclude<CheckinProduct, 'all'>): string {
  return product === 'trae' ? 'TRAE' : 'WorkBuddy';
}
