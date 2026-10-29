import { QueryClient, useQuery, type UseQueryOptions } from "@tanstack/react-query";
import { rpc } from "./rpc";
import type { AppInfo } from "./types";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: { staleTime: 30_000, refetchOnWindowFocus: false, retry: 1 },
  },
});

/** useQuery around an rpc call. The key must start with the domain ("mail", "calendar", ...). */
export function useRpc<T>(key: unknown[], method: string, params: object = {}, opts: Omit<UseQueryOptions<T>, "queryKey" | "queryFn"> = {}) {
  return useQuery<T>({ queryKey: key, queryFn: () => rpc<T>(method, params), ...opts });
}

/** app.info (version, data folder, Google client file). One key for every window; under
 *  ["accounts"] so adding or removing an account refreshes it. */
const APP_INFO_KEY = ["accounts", "info"];

export function useAppInfo() {
  return useRpc<AppInfo>(APP_INFO_KEY, "app.info");
}

export function fetchAppInfo() {
  return queryClient.fetchQuery({ queryKey: APP_INFO_KEY, queryFn: () => rpc<AppInfo>("app.info") });
}
