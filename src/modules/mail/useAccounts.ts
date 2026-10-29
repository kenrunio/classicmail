import { useRpc } from "../../api/query";
import type { Account } from "../../api/types";

export function useAccounts(): Account[] {
  return useRpc<Account[]>(["accounts"], "accounts.list").data ?? [];
}
