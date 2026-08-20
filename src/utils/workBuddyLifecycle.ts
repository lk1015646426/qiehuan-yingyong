export function shouldLoadWorkBuddyAccounts(accountsLoaded: boolean, loading: boolean): boolean {
  return !accountsLoaded && !loading;
}
