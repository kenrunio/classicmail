// Mail merge (src-tauri/src/mail/merge): one personalised message per row of an Excel / CSV list.
import { rpc } from "./rpc";
import type { Compose } from "./mailCompose";

export interface MergeSummary {
  fileName: string;
  headers: string[];
  /** index into headers */
  emailColumn: number;
  rowCount: number;
}

export interface RowIssue {
  /** row number in the file */
  row: number;
  email: string;
  fields?: string[];
}

export interface Preflight {
  messages: number;
  skipped: RowIssue[];
  duplicates: RowIssue[];
  /** "{Field}" spellings that match no column */
  unknownFields: string[];
  blankFields: RowIssue[];
  overLimit: boolean;
}

export type MergeMode = "test" | "drafts" | "send";

export interface MergeResult {
  created: number;
  testRow?: number;
  testEmail?: string;
}

export const mergeApi = {
  load: (path: string) => rpc<MergeSummary>("mail.merge.load", { path }),
  check: (path: string, compose: Compose) => rpc<Preflight>("mail.merge.check", { path, compose }),
  run: (path: string, compose: Compose, mode: MergeMode) => rpc<MergeResult>("mail.merge.run", { path, compose, mode }),
};
