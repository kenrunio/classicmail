// Core DTOs shared by every module. Domain DTOs live in src/api/<domain>.ts.
export interface Account {
  id: number;
  email: string;
  displayName: string;
  provider: "google" | "demo";
  color: string;
  position: number;
  enabled: boolean;
}

export interface AppInfo {
  version: string;
  dataDir: string;
  googleClientConfigured: boolean;
  googleClientEmbedded: boolean;
}

export interface SyncStatus {
  accountId: number;
  phase: "idle" | "sync" | "error" | string;
  text: string;
  progress?: number | null;
}
