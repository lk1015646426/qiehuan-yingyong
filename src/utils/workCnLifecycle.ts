export function shouldLoadWorkCnAccounts(accountsLoaded: boolean, loading: boolean): boolean {
  return !accountsLoaded && !loading;
}
