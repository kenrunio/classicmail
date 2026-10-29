// Compose / Outbox / signatures / send-as / auto-complete DTOs and calls (mail-compose domain).
import { rpc, RpcError } from "./rpc";

export interface Addr {
  name: string;
  email: string;
}

export interface AttachmentSpec {
  path?: string;
  attachmentId?: number;
  dataUrl?: string;
  cid?: string;
  name?: string;
  size?: number;
  mimeType?: string;
}

export type Importance = "low" | "normal" | "high";
export type ComposeMode = "new" | "reply" | "replyAll" | "forward" | "draft";

export interface Compose {
  accountId: number;
  from?: string | null;
  to: Addr[];
  cc: Addr[];
  bcc: Addr[];
  subject: string;
  html: string;
  text?: string | null;
  plainText?: boolean;
  attachments: AttachmentSpec[];
  importance: Importance;
  requestReadReceipt: boolean;
  requestDeliveryReceipt: boolean;
  deferUntil?: number | null;
  replyTo: Addr[];
  flagForRecipients?: string | null;
  mode: ComposeMode;
  sourceMessageId?: number | null;
  draftId?: string | null;
}

export interface FromChoice {
  accountId: number;
  email: string;
  name: string;
  primary: boolean;
}

export interface Prepared {
  compose: Compose;
  bodyHtml: string;
  quotedHtml: string;
  fromChoices: FromChoice[];
  draftMessageId?: number | null;
  /** replies: recipients of a plain Reply and of a Reply All (the Reply to One toggle) */
  replyTargets?: { one: { to: Addr[]; cc: Addr[] }; all: { to: Addr[]; cc: Addr[] } };
}

export interface FileInfo {
  path: string;
  name: string;
  size: number;
  mimeType: string;
  exists: boolean;
}

export interface OutboxItem {
  id: number;
  accountId: number;
  subject: string;
  to: Addr[];
  from?: string | null;
  status: "queued" | "sending" | "failed";
  attempts: number;
  lastError?: string | null;
  sendAt: number;
  createdAt: number;
  size: number;
  hasAttachments: boolean;
  importance: Importance;
}

export interface Signature {
  id: number;
  name: string;
  html: string;
}

export interface SignatureDefaults {
  accountId: number;
  newSignatureId: number | null;
  replySignatureId: number | null;
}

export interface SendAsRow {
  email: string;
  displayName: string;
  isDefault: boolean;
  signature: string;
}

export interface Suggestion {
  name: string;
  email: string;
  source: "recent" | "contact" | "other";
}

/** `mail.newMail` payload (contracts.md). */
export interface NewMailEvent {
  accountId: number;
  messages: { id: number; from: string; subject: string; preview: string }[];
}

export const composeApi = {
  prepare: (mode: ComposeMode, id?: number, accountId?: number) => rpc<Prepared>("mail.compose.prepare", { mode, id, accountId }),
  send: (compose: Compose) => rpc<{ outboxId: number; sendAt: number }>("mail.compose.send", { compose }),
  saveDraft: (compose: Compose) => rpc<{ draftId: string; messageId: number }>("mail.compose.saveDraft", { compose }),
  discardDraft: (accountId: number, draftId: string) => rpc<void>("mail.compose.discardDraft", { accountId, draftId }),
  fileInfo: (paths: string[]) => rpc<FileInfo[]>("mail.compose.fileInfo", { paths }),
  /** a message another program (Word's "Send as Attachment") handed over (src-tauri/src/mapi.rs) */
  readMapi: (dir: string) => rpc<{ subject: string; body: string; to: Addr[]; cc: Addr[]; bcc: Addr[]; attachments: FileInfo[] }>("mail.compose.readMapi", { dir }),
  readImage: (path: string) => rpc<{ name: string; dataUrl: string }>("mail.compose.readImage", { path }),
  openAttachment: (a: { path?: string; attachmentId?: number; confirmed?: boolean }) => rpc<void>("mail.compose.openAttachment", a),
  saveAs: (path: string, content: string) => rpc<void>("mail.compose.saveAs", { path, content }),
  outboxList: (accountId?: number) => rpc<OutboxItem[]>("mail.outbox.list", { accountId }),
  outboxCancel: (id: number) => rpc<{ draftId: string; messageId: number }>("mail.outbox.cancel", { id }),
  outboxRetry: (id: number) => rpc<void>("mail.outbox.retry", { id }),
  signatures: () => rpc<{ signatures: Signature[]; defaults: SignatureDefaults[] }>("mail.signatures.list", {}),
  saveSignature: (s: { id?: number | null; name: string; html: string }) => rpc<Signature>("mail.signatures.save", s),
  deleteSignature: (id: number) => rpc<void>("mail.signatures.delete", { id }),
  setSignatureDefaults: (d: SignatureDefaults) => rpc<void>("mail.signatures.setDefaults", d),
  sendAs: (accountId: number) => rpc<SendAsRow[]>("mail.sendAs.list", { accountId }),
  forgetRecipient: (email: string) => rpc<void>("mail.recipients.forget", { email }),
};

/** Auto-complete: our nickname cache first, then contacts (if that domain is there yet). */
export async function suggestRecipients(query: string, limit = 10): Promise<Suggestion[]> {
  const q = query.trim();
  if (!q) return [];
  const [mine, contacts] = await Promise.all([
    rpc<Suggestion[]>("mail.recipients.suggest", { query: q, limit }).catch(() => [] as Suggestion[]),
    rpc<Suggestion[]>("contacts.autocomplete", { query: q, limit }).catch((e: unknown) => {
      if (!(e instanceof RpcError) || e.code !== "no_method") console.warn("contacts.autocomplete", e);
      return [] as Suggestion[];
    }),
  ]);
  const seen = new Set<string>();
  const out: Suggestion[] = [];
  for (const s of [...mine, ...contacts]) {
    const k = s.email.toLowerCase();
    if (!s.email || seen.has(k)) continue;
    seen.add(k);
    out.push(s);
    if (out.length >= limit) break;
  }
  return out;
}

export function formatAddr(a: Addr): string {
  return a.name && a.name.toLowerCase() !== a.email.toLowerCase() ? `${a.name} <${a.email}>` : a.email;
}

export function isValidEmail(s: string): boolean {
  return /^[^\s@<>()[\],;:"]+@[^\s@<>()[\],;:"]+\.[^\s@<>()[\],;:".]+$/.test(s.trim());
}

/** "Name <a@b>; c@d, \"Last, First\" <e@f>" → addresses (unparseable bits keep name only). */
export function parseAddresses(s: string): Addr[] {
  const out: Addr[] = [];
  let cur = "";
  let q = false;
  let angle = false;
  const flush = () => {
    const t = cur.trim();
    cur = "";
    if (!t) return;
    const m = /^(.*)<([^>]*)>\s*$/.exec(t);
    if (m) out.push({ name: m[1].trim().replace(/^"|"$/g, "").trim(), email: m[2].trim() });
    else out.push({ name: "", email: t.replace(/^"|"$/g, "") });
  };
  for (const ch of s) {
    if (ch === '"') q = !q;
    if (!q && ch === "<") angle = true;
    if (!q && ch === ">") angle = false;
    if ((ch === ";" || ch === ",") && !q && !angle) flush();
    else cur += ch;
  }
  flush();
  return out;
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / 1024 / 1024).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB`;
}
