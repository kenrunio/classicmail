// Mail DTOs and rpc wrappers (mail-core). Compose-related calls live in src/api/mailCompose.ts.
import { rpc } from "./rpc";

export interface Addr {
  name: string;
  email: string;
}

export type FolderRole = "inbox" | "sent" | "drafts" | "trash" | "spam" | "outbox" | "all" | "user" | "placeholder" | string;

export interface MailFolder {
  id: number;
  accountId: number;
  gmailLabelId: string | null;
  name: string;
  path: string;
  parentId: number | null;
  role: FolderRole;
  hidden: boolean;
  unread: number;
  total: number;
}

export interface Mailbox {
  accountId: number;
  email: string;
  displayName: string;
  folders: MailFolder[];
  unreadMail: number;
  followUp: number;
  largeMail: number;
}

export type SearchFolder = "unread" | "followUp" | "large" | "archive";

export interface ListSource {
  folderId?: number;
  searchFolder?: SearchFolder;
  accountId?: number;
  ids?: number[];
}

export type Arrange = "date" | "conversation" | "from" | "to" | "subject" | "size" | "flag" | "attachments" | "importance";

export interface ListParams {
  source: ListSource;
  arrange: Arrange;
  desc: boolean;
  search?: string;
  /** with `search`: Gmail's own matches, shown too (mail.searchServer) */
  alsoIds?: number[];
  /** only these messages, within the view */
  onlyIds?: number[];
  scope?: "folder" | "all";
  offset: number;
  limit: number;
  withGroups?: boolean;
  /** Show as Conversations: one row per thread (`MessageRow.conv`) */
  conversations?: boolean;
}

/** A conversation header row's aggregates (Show as Conversations). */
export interface ConvInfo {
  /** "<accountId>:<threadId>" */
  key: string;
  /** messages of the conversation in this view */
  count: number;
  /** messages in all folders (what expanding shows) */
  total: number;
  unread: number;
  size: number;
  hasAttachments: boolean;
  importance: number;
  /** 2 flagged, 1 completed, 0 none */
  flag: number;
  /** senders, newest first */
  participants: string;
  /** the conversation's messages in this view, newest first (what actions apply to) */
  ids: number[];
}

export interface MessageRow {
  id: number;
  accountId: number;
  threadId: string;
  subject: string;
  fromName: string;
  fromEmail: string;
  toDisplay: string;
  date: number;
  size: number;
  isRead: boolean;
  isFlagged: boolean;
  flagStatus: "flagged" | "complete" | null;
  flagDue: string | null;
  hasAttachments: boolean;
  importance: number;
  lastVerb: "replied" | "repliedAll" | "forwarded" | null;
  isDraft: boolean;
  isInvite: boolean;
  preview: string;
  folder: string | null;
  outboxId: number | null;
  outboxStatus: string | null;
  /** conversation header (the other fields are its newest message's) */
  conv?: ConvInfo;
  /** `listThread`: the message is in the current view (else another folder, see `folder`) */
  inView?: boolean;
}

export interface ListGroup {
  key: string;
  label: string;
  count: number;
}

export interface ListResult {
  total: number;
  unread: number;
  groups: ListGroup[] | null;
  rows: MessageRow[];
  crossFolder: boolean;
  /** messages in the view (`total` counts conversations when they're shown) */
  messages: number;
}

/** html: shown in the sandboxed body frame; pdf: the WebView's PDF viewer at `url`; none: `note` why not */
export type AttachmentPreview = { kind: "html"; html: string } | { kind: "pdf"; url: string } | { kind: "none"; note: string };

export interface Attachment {
  id: number;
  filename: string;
  mimeType: string;
  size: number;
  isInline: boolean;
  contentId: string | null;
}

export interface Invite {
  method: string;
  uid: string;
  summary: string;
  location: string;
  start: number | null;
  end: number | null;
  allDay: boolean;
  organizer: string;
  organizerEmail: string;
  /** METHOD:REPLY: who answered, and ACCEPTED / TENTATIVE / DECLINED */
  replyFrom?: string;
  replyStatus?: string;
}

export interface MessageView {
  id: number;
  accountId: number;
  accountEmail: string;
  gmailId: string;
  threadId: string;
  subject: string;
  fromName: string;
  fromEmail: string;
  to: Addr[];
  cc: Addr[];
  bcc: Addr[];
  replyTo: Addr[];
  sentAt: number | null;
  receivedAt: number;
  isRead: boolean;
  isFlagged: boolean;
  flagStatus: "flagged" | "complete" | null;
  flagDue: string | null;
  importance: number;
  lastVerb: string | null;
  lastVerbAt: number | null;
  isDraft: boolean;
  size: number;
  folders: { id: number; name: string; role: FolderRole }[];
  html: string;
  isPlain: boolean;
  text: string;
  remoteBlocked: boolean;
  hasRemote: boolean;
  safeSender: boolean;
  attachments: Attachment[];
  invite: Invite | null;
  bodyError: string | null;
  messageIdHeader: string | null;
}

export type FlagKind = "none" | "today" | "tomorrow" | "thisWeek" | "nextWeek" | "noDate" | "custom" | "complete";

/** Whose domain picks the folder: a message (by one of its attachments), an archived message, or
 *  the recipients of a message being written. */
export type FolderFor = { attachmentId: number } | { messageId: number } | { archiveMessageId: number } | { to: string[]; cc: string[] };

/** The folder part of a file path. */
export const folderOf = (path: string) => path.replace(/[\\/][^\\/]*$/, "");

/** A save dialog's default path: the file name in the suggested folder. */
export const inFolder = (folder: string | null, name: string) => (folder ? `${folder.replace(/[\\/]+$/, "")}\\${name}` : name);

export const mailApi = {
  folders: () => rpc<Mailbox[]>("mail.listFolders"),
  list: (p: ListParams) => rpc<ListResult>("mail.listMessages", p),
  /** every message of a conversation across folders, newest first; `view` marks `inView` */
  listThread: (accountId: number, threadId: string, view?: Pick<ListParams, "source" | "search" | "scope">) => rpc<MessageRow[]>("mail.listThread", { accountId, threadId, view }),
  get: (id: number, allowRemote = false) => rpc<MessageView>("mail.getMessage", { id, allowRemote }),
  headers: (id: number) => rpc<string>("mail.getHeaders", { id }),
  adjacent: (id: number, direction: "next" | "previous", folderId?: number) => rpc<number | null>("mail.adjacent", { id, direction, folderId }),
  markRead: (ids: number[], read: boolean) => rpc("mail.markRead", { ids, read }),
  markAllRead: (folderId: number) => rpc<number>("mail.markAllRead", { folderId }),
  setFlag: (ids: number[], flag: FlagKind, due?: string) => rpc("mail.setFlag", { ids, flag, due }),
  move: (ids: number[], folderId: number, fromFolderId?: number) => rpc("mail.move", { ids, folderId, fromFolderId }),
  copy: (ids: number[], folderId: number) => rpc("mail.copy", { ids, folderId }),
  archive: (ids: number[], fromFolderId?: number) => rpc("mail.archive", { ids, fromFolderId }),
  delete: (ids: number[]) => rpc<{ permanent: number }>("mail.delete", { ids }),
  /** how many of `ids` Delete would erase for good (those in Deleted Items) */
  deleteInfo: (ids: number[]) => rpc<{ permanent: number }>("mail.deleteInfo", { ids }),
  restore: (ids: number[]) => rpc("mail.restore", { ids }),
  emptyFolder: (folderId: number) => rpc<number>("mail.emptyFolder", { folderId }),
  markJunk: (ids: number[]) => rpc("mail.markJunk", { ids }),
  notJunk: (ids: number[]) => rpc("mail.notJunk", { ids }),
  createFolder: (name: string, parentId?: number, accountId?: number) => rpc<number>("mail.createFolder", { name, parentId, accountId }),
  renameFolder: (folderId: number, name: string) => rpc("mail.renameFolder", { folderId, name }),
  deleteFolder: (folderId: number) => rpc("mail.deleteFolder", { folderId }),
  saveAttachment: (attachmentId: number, path: string) => rpc<string>("mail.saveAttachment", { attachmentId, path }),
  saveAllAttachments: (messageId: number, dir: string) => rpc<string[]>("mail.saveAllAttachments", { messageId, dir }),
  /** the folder to open for saving or attaching, learnt per correspondent's domain (src-tauri/src/mail/save_folders.rs) */
  suggestFolder: (f: FolderFor) => rpc<string | null>("mail.saveFolder.suggest", f),
  /** an attachment was saved in `folder`: count it for the message's domain */
  rememberFolder: (f: FolderFor, folder: string) => rpc<boolean>("mail.saveFolder.remember", { ...f, folder }),
  /** `confirmed`: the user said "Open" after a `confirm_open` error ("Opening Mail Attachment"). */
  openAttachment: (attachmentId: number, confirmed = false, choose = false) => rpc<string>("mail.openAttachment", { attachmentId, confirmed, choose }),
  addSafeSender: (address: string) => rpc("mail.addSafeSender", { address }),
  /** Reading Pane preview of an attachment (mail/preview.rs) */
  previewAttachment: (id: number) => rpc<AttachmentPreview>("mail.previewAttachment", { id }),
  /** Gmail's own search: every match (`ids`) and the ones the local search didn't find (`extraIds`) */
  searchServer: (query: string, accountId?: number) => rpc<{ ids: number[]; query: string; extra: number; extraIds: number[] }>("mail.searchServer", { query, accountId }),
  deliverDemo: (accountId?: number) => rpc("demo.deliverMail", { accountId }),
};
