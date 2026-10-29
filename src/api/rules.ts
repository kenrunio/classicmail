// Rules DTOs (mirror src-tauri/src/rules/model.rs) and rpc wrappers.
import { rpc } from "./rpc";

export interface Person {
  name: string;
  email: string;
}

export type ImportanceLevel = "low" | "normal" | "high";

export type Condition =
  | { type: "from"; people: Person[] }
  | { type: "sentTo"; people: Person[] }
  | { type: "subjectContains"; words: string[] }
  | { type: "bodyContains"; words: string[] }
  | { type: "subjectOrBodyContains"; words: string[] }
  | { type: "headerContains"; words: string[] }
  | { type: "hasAttachment" }
  | { type: "importance"; level: ImportanceLevel }
  | { type: "sentOnlyToMe" }
  | { type: "myNameInTo" }
  | { type: "myNameInToOrCc" }
  | { type: "myNameNotInTo" }
  | { type: "throughAccount"; accountId: number }
  | { type: "fromDomain"; domains: string[] };

export type ConditionType = Condition["type"];

export type FlagValue = "today" | "tomorrow" | "thisWeek" | "nextWeek" | "noDate" | "complete" | "none";

export type Action =
  | { type: "moveToFolder"; folderId: number; folderPath: string }
  | { type: "copyToFolder"; folderId: number; folderPath: string }
  | { type: "delete" }
  | { type: "markRead" }
  | { type: "flag"; flag: FlagValue }
  | { type: "setImportance"; level: ImportanceLevel }
  | { type: "forwardTo"; people: Person[] }
  | { type: "displayDesktopAlert" }
  | { type: "playSound"; sound: string }
  | { type: "stopProcessing" };

export type ActionType = Action["type"];

export interface Rule {
  id: number;
  name: string;
  enabled: boolean;
  position: number;
  /** null = every mailbox */
  accountId: number | null;
  conditions: Condition[];
  exceptions: Condition[];
  actions: Action[];
  createdAt?: number;
  updatedAt?: number;
}

export interface RunStats {
  processed: number;
  matched: number;
  actions: number;
  failed: number;
  errors: string[];
}

export interface MessageInfo {
  id: number;
  accountId: number;
  from: Person;
  to: Person[];
  cc: Person[];
  subject: string;
  folderId: number | null;
  folderName: string | null;
}

/** mail.listFolders entry (provided by the mail module). */
export interface MailFolder {
  id: number;
  accountId: number;
  name: string;
  path: string;
  role: string;
  parentId: number | null;
  unreadCount?: number;
  totalCount?: number;
}

export const rulesApi = {
  list: (accountId?: number | null) => rpc<Rule[]>("rules.list", accountId == null ? {} : { accountId }),
  get: (id: number) => rpc<Rule>("rules.get", { id }),
  save: (rule: Rule) => rpc<{ id: number }>("rules.save", { rule }),
  delete: (ids: number[]) => rpc<number>("rules.delete", { ids }),
  reorder: (ids: number[]) => rpc<void>("rules.reorder", { ids }),
  setEnabled: (id: number, enabled: boolean) => rpc<void>("rules.setEnabled", { id, enabled }),
  runNow: (p: { ruleIds: number[]; folderId: number; includeSubfolders?: boolean; onlyUnread?: boolean }) => rpc<RunStats>("rules.runNow", p),
  test: (rule: Rule, messageId: number) => rpc<{ matches: boolean; conditions: boolean[]; exceptions: boolean[] }>("rules.test", { rule, messageId }),
  messageInfo: (messageId: number) => rpc<MessageInfo>("rules.messageInfo", { messageId }),
  export: (path: string, ruleIds?: number[]) => rpc<number>("rules.export", { path, ruleIds }),
  import: (path: string) => rpc<{ imported: number; disabled: string[]; forwards: string[]; skipped: string[] }>("rules.import", { path }),
};
