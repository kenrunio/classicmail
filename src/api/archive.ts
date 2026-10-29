// Outlook archives (.pst), read-only. Rows of `archive.listMessages` have the mail list shape
// (`MessageRow`, `ListResult`) so the mail MessageList shows them; `accountId` is the archive id.
import { rpc } from "./rpc";
import type { Addr, Arrange, ListResult } from "./mail";
import type { Prepared } from "./mailCompose";

export type ArchiveStatus = "pending" | "indexing" | "ready" | "error" | "missing" | "locked";

export interface ArchiveFolder {
  id: number;
  archiveId: number;
  parentId: number | null;
  name: string;
  /** "mail" folders are shown; calendar / contacts / tasks / notes / journal are not (v1) */
  kind: string;
  contentCount: number;
  total: number;
  unread: number;
  done: boolean;
}

export interface Archive {
  id: number;
  path: string;
  displayName: string;
  storeName: string;
  format: string;
  fileSize: number;
  status: ArchiveStatus;
  error: string | null;
  itemCount: number;
  indexed: number;
  expected: number;
  indexText: boolean;
  textDone: boolean;
  textIndexed: number;
  indexedAt: number | null;
  indexMs: number | null;
  folders: ArchiveFolder[];
}

export interface ArchiveListParams {
  source: { folderId?: number; archiveId?: number };
  arrange: Arrange;
  desc: boolean;
  search?: string;
  /** "folder" | "archive" (this archive) | "all" (Search All Archives) */
  scope?: "folder" | "archive" | "all";
  offset: number;
  limit: number;
  withGroups?: boolean;
}

export interface ArchiveAttachment {
  index: number;
  filename: string;
  mimeType: string;
  size: number;
  isInline: boolean;
  contentId: string | null;
}

export interface ArchiveMessageView {
  id: number;
  archiveId: number;
  archiveName: string;
  folderId: number;
  folderName: string;
  messageClass: string;
  subject: string;
  fromName: string;
  fromEmail: string;
  to: Addr[];
  cc: Addr[];
  bcc: Addr[];
  sentAt: number | null;
  receivedAt: number;
  importance: number;
  flagStatus: "flagged" | "complete" | null;
  isRead: boolean;
  lastVerb: string | null;
  size: number;
  html: string;
  isPlain: boolean;
  text: string;
  remoteBlocked: boolean;
  hasRemote: boolean;
  attachments: ArchiveAttachment[];
  bodySource: string;
  messageIdHeader: string | null;
}

export const archiveApi = {
  open: (paths: string[]) => rpc<{ ids: number[]; errors: { path: string; message: string }[] }>("archive.open", { paths }),
  close: (id: number) => rpc<void>("archive.close", { id }),
  list: () => rpc<Archive[]>("archive.list", {}),
  listMessages: (p: ArchiveListParams) => rpc<ListResult>("archive.listMessages", p),
  search: (query: string, archiveId?: number) => rpc<ListResult>("archive.search", { query, archiveId }),
  get: (id: number, allowRemote = false) => rpc<ArchiveMessageView>("archive.getMessage", { id, allowRemote }),
  headers: (id: number) => rpc<string>("archive.getHeaders", { id }),
  adjacent: (id: number, direction: "next" | "previous") => rpc<number | null>("archive.adjacent", { id, direction }),
  saveAttachment: (id: number, index: number, path: string) => rpc<string>("archive.saveAttachment", { id, index, path }),
  saveAllAttachments: (id: number, dir: string) => rpc<string[]>("archive.saveAllAttachments", { id, dir }),
  openAttachment: (id: number, index: number, confirmed = false, choose = false) => rpc<string>("archive.openAttachment", { id, index, confirmed, choose }),
  setIndexText: (id: number, on: boolean) => rpc<void>("archive.setIndexText", { id, on }),
  reindex: (id: number) => rpc<void>("archive.reindex", { id }),
  properties: (id: number) => rpc<Archive>("archive.properties", { id }),
  prepareCompose: (id: number, mode: "reply" | "replyAll" | "forward", accountId?: number) => rpc<Prepared>("archive.prepareCompose", { id, mode, accountId }),
};
