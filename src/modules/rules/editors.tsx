// The small dialogs behind the underlined values of a rule description (People, Search Text,
// folder picker, importance, account, follow-up flag, sound), opened with a promise:
//   const next = await editValue(kind, item, { accountId });   // null = cancelled
import { useEffect, useMemo, useRef, useState } from "react";
import { create } from "zustand";
import { rpc, RpcError } from "../../api/rpc";
import type { Account } from "../../api/types";
import type { Action, Condition, FlagValue, ImportanceLevel, MailFolder, Person } from "../../api/rules";
import { Dialog, TreeView, type TreeNode } from "../../ui";
import { FLAG_LABELS, IMPORTANCE_LABELS, personLabel, type ValueKind } from "./describe";

type Item = Condition | Action;

interface EditReq {
  kind: ValueKind;
  item: Item;
  accountId: number | null;
  /** "subject" | "body" ... for the Search Text caption */
  resolve: (v: Item | null) => void;
}

const useEditor = create<{ req: EditReq | null; set: (r: EditReq | null) => void }>((set) => ({ req: null, set: (req) => set({ req }) }));

export function editValue(kind: ValueKind, item: Item, opts: { accountId?: number | null } = {}): Promise<Item | null> {
  return new Promise((resolve) => useEditor.getState().set({ kind, item, accountId: opts.accountId ?? null, resolve }));
}

/** Mount once in a window that edits rules. */
export function EditorHost({ accounts }: { accounts: Account[] }) {
  const req = useEditor((s) => s.req);
  if (!req) return null;
  const done = (v: Item | null) => {
    useEditor.getState().set(null);
    req.resolve(v);
  };
  const it = req.item as Record<string, unknown> & Item;
  switch (req.kind) {
    case "people":
      return (
        <PeopleDialog
          title={it.type === "forwardTo" ? "Rule Address — Forward to" : "Rule Address"}
          listCaption={it.type === "from" ? "From:" : "To:"}
          initial={(it as { people: Person[] }).people}
          onOk={(people) => done({ ...it, people } as Item)}
          onCancel={() => done(null)}
        />
      );
    case "words":
    case "domains": {
      const key = req.kind === "words" ? "words" : "domains";
      const where: Record<string, string> = {
        subjectContains: "the subject",
        bodyContains: "the body",
        subjectOrBodyContains: "the subject or body",
        headerContains: "the message header",
      };
      const caption =
        req.kind === "domains" ? "Specify the sender domains (for example harborcap.com):" : `Specify words or phrases to search for in ${where[it.type] ?? "the message"}:`;
      return (
        <SearchTextDialog
          title={req.kind === "domains" ? "Sender Domains" : "Search Text"}
          caption={caption}
          initial={(it as unknown as Record<string, string[]>)[key]}
          onOk={(v) => done({ ...it, [key]: v } as Item)}
          onCancel={() => done(null)}
        />
      );
    }
    case "importance":
      return (
        <ChoiceDialog
          title="Importance"
          caption="Specify the importance:"
          options={(Object.keys(IMPORTANCE_LABELS) as ImportanceLevel[]).map((k) => [k, IMPORTANCE_LABELS[k]])}
          initial={(it as { level: ImportanceLevel }).level}
          onOk={(level) => done({ ...it, level } as Item)}
          onCancel={() => done(null)}
        />
      );
    case "flag":
      return (
        <ChoiceDialog
          title="Flag for Follow Up"
          caption="Flag to: Follow up. Due:"
          options={(Object.keys(FLAG_LABELS) as FlagValue[]).filter((k) => k !== "none").map((k) => [k, FLAG_LABELS[k]])}
          initial={(it as { flag: FlagValue }).flag}
          onOk={(flag) => done({ ...it, flag } as Item)}
          onCancel={() => done(null)}
        />
      );
    case "account":
      return (
        <ChoiceDialog
          title="Account"
          caption="Select the account:"
          options={accounts.map((a) => [String(a.id), `${a.displayName} (${a.email})`])}
          initial={String((it as { accountId: number }).accountId || accounts[0]?.id || "")}
          onOk={(id) => done({ ...it, accountId: Number(id) } as Item)}
          onCancel={() => done(null)}
        />
      );
    case "sound":
      return (
        <ChoiceDialog
          title="Play a Sound"
          caption="Sound:"
          options={[
            ["Notify", "Notify"],
            ["Chimes", "Chimes"],
            ["Ding", "Ding"],
          ]}
          initial={(it as { sound: string }).sound || "Notify"}
          onOk={(sound) => done({ ...it, sound } as Item)}
          onCancel={() => done(null)}
        />
      );
    case "folder":
      return (
        <FolderDialog
          accountId={req.accountId}
          accounts={accounts}
          initial={(it as { folderId: number }).folderId}
          onOk={(f) => done({ ...it, folderId: f.id, folderPath: f.path } as Item)}
          onCancel={() => done(null)}
        />
      );
  }
}

// ---- people --------------------------------------------------------------------------------

/** "Sarah Cohen <sarah@x.com>; david@y.com; Maya" → people */
export function parsePeople(text: string): Person[] {
  const out: Person[] = [];
  // split on ; , and newlines, but not inside "quotes" or <brackets>
  const parts: string[] = [];
  let cur = "";
  let quote = false;
  let angle = false;
  for (const ch of text) {
    if (ch === '"') quote = !quote;
    else if (ch === "<") angle = true;
    else if (ch === ">") angle = false;
    if (!quote && !angle && (ch === ";" || ch === "," || ch === "\n")) {
      parts.push(cur);
      cur = "";
    } else cur += ch;
  }
  parts.push(cur);
  for (const raw of parts) {
    const s = raw.trim();
    if (!s) continue;
    const m = s.match(/^"?([^"<]*?)"?\s*<([^>]+)>$/);
    if (m) out.push({ name: m[1].trim(), email: m[2].trim() });
    else if (s.includes("@")) out.push({ name: "", email: s });
    else out.push({ name: s, email: "" });
  }
  return out;
}

export function formatPerson(p: Person) {
  if (p.name && p.email) return `${p.name} <${p.email}>`;
  return p.email || p.name;
}

type Suggestion = { name: string; email: string };

function PeopleDialog({
  title,
  listCaption,
  initial,
  onOk,
  onCancel,
}: {
  title: string;
  listCaption: string;
  initial: Person[];
  onOk: (p: Person[]) => void;
  onCancel: () => void;
}) {
  const [list, setList] = useState<Person[]>(initial);
  const [text, setText] = useState("");
  const [sel, setSel] = useState(-1);
  const [sugg, setSugg] = useState<Suggestion[]>([]);
  const noAutocomplete = useRef(false);
  useEffect(() => {
    const q = text.split(/[;,]/).pop()?.trim() ?? "";
    if (q.length < 2 || noAutocomplete.current) {
      setSugg([]);
      return;
    }
    const t = setTimeout(() => {
      rpc<Suggestion[]>("contacts.autocomplete", { query: q, limit: 8 })
        .then((r) => setSugg(Array.isArray(r) ? r : []))
        .catch((e) => {
          if (e instanceof RpcError && e.code === "no_method") noAutocomplete.current = true;
          setSugg([]);
        });
    }, 150);
    return () => clearTimeout(t);
  }, [text]);
  // a typed name that is exactly a suggested contact's name resolves to that contact (like Check Names)
  const resolve = (people: Person[]) =>
    people.map((p) => {
      if (p.email || !p.name) return p;
      const hits = sugg.filter((s) => s.name.trim().toLowerCase() === p.name.trim().toLowerCase());
      return hits.length === 1 ? { name: hits[0].name, email: hits[0].email } : p;
    });
  const add = (people: Person[]) => {
    people = resolve(people);
    const merged = [...list];
    for (const p of people) if (!merged.some((x) => x.email.toLowerCase() === p.email.toLowerCase() && x.name === p.name)) merged.push(p);
    setList(merged);
    setText("");
    setSugg([]);
  };
  return (
    <Dialog
      title={title}
      onClose={onCancel}
      width={440}
      footer={
        <>
          <button className="cm-button default" onClick={() => onOk(text.trim() ? [...list, ...resolve(parsePeople(text))] : list)}>
            OK
          </button>
          <button className="cm-button" onClick={onCancel}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-rules-field-caption">Type names or e-mail addresses (separate several with semicolons):</div>
      <div style={{ display: "flex", gap: 6, position: "relative" }}>
        <input
          className="cm-input cm-fill"
          value={text}
          autoFocus
          placeholder="e.g. Sarah Cohen <sarah.cohen@harborcap.example>; @meridian.example"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && text.trim()) {
              e.preventDefault();
              add(parsePeople(text));
            }
          }}
        />
        <button className="cm-button" disabled={!text.trim()} onClick={() => add(parsePeople(text))}>
          Add -&gt;
        </button>
        {sugg.length > 0 && (
          <div className="cm-rules-suggest">
            {sugg.map((s) => (
              <div key={s.email} className="cm-rules-suggest-item" onMouseDown={(e) => (e.preventDefault(), add([{ name: s.name, email: s.email }]))}>
                {s.name ? `${s.name} <${s.email}>` : s.email}
              </div>
            ))}
          </div>
        )}
      </div>
      <div className="cm-rules-field-caption" style={{ marginTop: 10 }}>
        {listCaption}
      </div>
      <div className="cm-rules-listbox" style={{ height: 120 }} tabIndex={0}>
        {list.map((p, i) => (
          <div key={i} className={`cm-rules-listbox-item${i === sel ? " sel" : ""}`} onMouseDown={() => setSel(i)} title={formatPerson(p)}>
            {formatPerson(p)}
          </div>
        ))}
        {!list.length && (
          <div className="cm-muted" style={{ padding: 4 }}>
            (nobody yet)
          </div>
        )}
      </div>
      <div style={{ marginTop: 6 }}>
        <button className="cm-button" disabled={sel < 0} onClick={() => (setList(list.filter((_, i) => i !== sel)), setSel(-1))}>
          Remove
        </button>
        <span className="cm-muted" style={{ marginLeft: 10 }}>
          Tip: <i>@domain.com</i> matches everyone at that domain.
        </span>
      </div>
    </Dialog>
  );
}

// ---- search text ---------------------------------------------------------------------------

function SearchTextDialog({ title, caption, initial, onOk, onCancel }: { title: string; caption: string; initial: string[]; onOk: (w: string[]) => void; onCancel: () => void }) {
  const [list, setList] = useState<string[]>(initial.filter((w) => w.trim()));
  const [text, setText] = useState("");
  const [sel, setSel] = useState(-1);
  const add = () => {
    const w = text.trim();
    if (w && !list.includes(w)) setList([...list, w]);
    setText("");
  };
  return (
    <Dialog
      title={title}
      onClose={onCancel}
      width={400}
      footer={
        <>
          <button className="cm-button default" onClick={() => onOk(text.trim() && !list.includes(text.trim()) ? [...list, text.trim()] : list)}>
            OK
          </button>
          <button className="cm-button" onClick={onCancel}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-rules-field-caption">{caption}</div>
      <div style={{ display: "flex", gap: 6 }}>
        <input
          className="cm-input cm-fill"
          value={text}
          autoFocus
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && text.trim()) {
              e.preventDefault();
              e.stopPropagation();
              add();
            }
          }}
        />
        <button className="cm-button" disabled={!text.trim()} onClick={add}>
          Add
        </button>
      </div>
      <div className="cm-rules-field-caption" style={{ marginTop: 10 }}>
        Search list:
      </div>
      <div className="cm-rules-listbox" style={{ height: 110 }}>
        {list.map((w, i) => (
          <div key={w} className={`cm-rules-listbox-item${i === sel ? " sel" : ""}`} onMouseDown={() => setSel(i)}>
            "{w}"
          </div>
        ))}
      </div>
      <div style={{ marginTop: 6, display: "flex", justifyContent: "flex-end" }}>
        <button className="cm-button" disabled={sel < 0} onClick={() => (setList(list.filter((_, i) => i !== sel)), setSel(-1))}>
          Remove
        </button>
      </div>
    </Dialog>
  );
}

// ---- simple choice -------------------------------------------------------------------------

function ChoiceDialog<T extends string>({
  title,
  caption,
  options,
  initial,
  onOk,
  onCancel,
}: {
  title: string;
  caption: string;
  options: [T, string][];
  initial: T;
  onOk: (v: T) => void;
  onCancel: () => void;
}) {
  const [v, setV] = useState<T>(initial);
  return (
    <Dialog
      title={title}
      onClose={onCancel}
      width={320}
      footer={
        <>
          <button className="cm-button default" onClick={() => onOk(v)}>
            OK
          </button>
          <button className="cm-button" onClick={onCancel}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-rules-field-caption">{caption}</div>
      <select className="cm-input" style={{ width: "100%" }} value={v} onChange={(e) => setV(e.target.value as T)} autoFocus>
        {options.map(([k, label]) => (
          <option key={k} value={k}>
            {label}
          </option>
        ))}
      </select>
    </Dialog>
  );
}

/** Generic one-line text prompt ("Rename Rule"). */
export function PromptDialog({ title, caption, initial, onOk, onCancel }: { title: string; caption: string; initial: string; onOk: (v: string) => void; onCancel: () => void }) {
  const [v, setV] = useState(initial);
  return (
    <Dialog
      title={title}
      onClose={onCancel}
      width={360}
      footer={
        <>
          <button className="cm-button default" disabled={!v.trim()} onClick={() => onOk(v.trim())}>
            OK
          </button>
          <button className="cm-button" onClick={onCancel}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-rules-field-caption">{caption}</div>
      <input
        className="cm-input"
        style={{ width: "100%", boxSizing: "border-box" }}
        value={v}
        autoFocus
        onFocus={(e) => e.currentTarget.select()}
        onChange={(e) => setV(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && v.trim()) {
            e.preventDefault();
            onOk(v.trim());
          }
        }}
      />
    </Dialog>
  );
}

// ---- folders -------------------------------------------------------------------------------

/** Accepts the folder list in any reasonable shape (flat, per-account groups, nested children). */
export function normalizeFolders(raw: unknown): MailFolder[] {
  const out: MailFolder[] = [];
  const walk = (v: unknown, accountId?: number, parentId?: number | null) => {
    if (Array.isArray(v)) {
      v.forEach((x) => walk(x, accountId, parentId));
      return;
    }
    if (!v || typeof v !== "object") return;
    const o = v as Record<string, unknown>;
    const acct = typeof o.accountId === "number" ? o.accountId : accountId;
    const isFolder = typeof o.id === "number" && typeof o.name === "string" && ("path" in o || "role" in o || "parentId" in o);
    if (isFolder && acct !== undefined) {
      out.push({
        id: o.id as number,
        accountId: acct,
        name: o.name as string,
        path: (o.path as string) ?? (o.name as string),
        role: (o.role as string) ?? "user",
        parentId: (o.parentId as number | null | undefined) ?? parentId ?? null,
      });
    }
    for (const k of ["children", "folders", "subfolders"]) if (k in o) walk(o[k], acct, isFolder ? (o.id as number) : parentId);
  };
  walk(raw);
  return out.filter((f) => !(f as unknown as { hidden?: boolean }).hidden);
}

let folderCache: { at: number; data: { folders: MailFolder[]; note?: string } } | null = null;

/** Folders from mail.listFolders (falls back to reading the mail tables via rules.folders). */
export async function loadFolders(force = false): Promise<{ folders: MailFolder[]; note?: string }> {
  if (!force && folderCache && Date.now() - folderCache.at < 30_000) return folderCache.data;
  let data: { folders: MailFolder[]; note?: string };
  try {
    data = { folders: normalizeFolders(await rpc("mail.listFolders", {})) };
  } catch (e) {
    if (!(e instanceof RpcError && e.code === "no_method")) throw e;
    try {
      data = { folders: normalizeFolders(await rpc("rules.folders", {})) };
    } catch {
      data = { folders: [] };
    }
    if (!data.folders.length) data.note = "The folder list isn't available yet (mail folders haven't been loaded).";
  }
  folderCache = { at: Date.now(), data };
  return data;
}

const ROLE_ORDER = ["inbox", "drafts", "sent", "trash", "spam", "outbox"];
const ROLE_ICON: Record<string, string> = { inbox: "inbox", drafts: "drafts", sent: "sent", trash: "deleted", spam: "junk", outbox: "outbox" };

export function folderTree(folders: MailFolder[], accounts: Account[], accountId: number | null): TreeNode[] {
  const accts = accounts.filter((a) => accountId == null || a.id === accountId);
  return accts
    .map((a) => {
      const mine = folders.filter((f) => f.accountId === a.id && !["all", "starred", "important"].includes(f.role));
      const kids = (parent: number | null): TreeNode[] =>
        mine
          .filter((f) => (f.parentId ?? null) === parent || (parent === null && f.parentId != null && !mine.some((p) => p.id === f.parentId)))
          .sort((x, y) => {
            const rx = ROLE_ORDER.indexOf(x.role);
            const ry = ROLE_ORDER.indexOf(y.role);
            if (rx !== ry) return (rx < 0 ? 99 : rx) - (ry < 0 ? 99 : ry);
            return x.name.localeCompare(y.name);
          })
          .map((f) => ({ id: `f${f.id}`, label: f.name, icon: ROLE_ICON[f.role] ?? "folder", data: f, children: kids(f.id) }));
      return { id: `a${a.id}`, label: `Mailbox - ${a.displayName || a.email}`, icon: "mailbox", bold: true, children: kids(null) } as TreeNode;
    })
    .filter((n) => n.children?.length);
}

export function useFolders() {
  const [state, setState] = useState<{ folders: MailFolder[]; note?: string; loading: boolean; error?: string }>({ folders: [], loading: true });
  useEffect(() => {
    let alive = true;
    loadFolders()
      .then((d) => alive && setState({ ...d, loading: false }))
      .catch((e) => alive && setState({ folders: [], loading: false, error: String(e?.message ?? e) }));
    return () => {
      alive = false;
    };
  }, []);
  return state;
}

function FolderDialog({
  accountId,
  accounts,
  initial,
  onOk,
  onCancel,
}: {
  accountId: number | null;
  accounts: Account[];
  initial: number;
  onOk: (f: MailFolder) => void;
  onCancel: () => void;
}) {
  const { folders, note, loading, error } = useFolders();
  const [sel, setSel] = useState<MailFolder | null>(null);
  const nodes = useMemo(() => folderTree(folders, accounts, accountId), [folders, accounts, accountId]);
  useEffect(() => {
    if (!sel && initial > 0) setSel(folders.find((f) => f.id === initial) ?? null);
  }, [folders, initial, sel]);
  return (
    <Dialog
      title="Rules and Alerts"
      onClose={onCancel}
      width={360}
      footer={
        <>
          <button className="cm-button default" disabled={!sel} onClick={() => sel && onOk(sel)}>
            OK
          </button>
          <button className="cm-button" onClick={onCancel}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-rules-field-caption">Choose a folder:</div>
      <div className="cm-rules-listbox" style={{ height: 230 }} onDoubleClick={() => sel && onOk(sel)}>
        {loading && (
          <div className="cm-muted" style={{ padding: 6 }}>
            Loading folders…
          </div>
        )}
        {error && <div className="cm-error-bar">{error}</div>}
        {!loading && !error && !nodes.length && (
          <div className="cm-muted" style={{ padding: 6 }}>
            {note ?? "No folders."}
          </div>
        )}
        <TreeView nodes={nodes} autoFocus selectedId={sel ? `f${sel.id}` : null} onSelect={(n) => n.data && setSel(n.data as MailFolder)} onActivate={() => sel && onOk(sel)} />
      </div>
      {sel && (
        <div className="cm-muted" style={{ marginTop: 6 }}>
          Folder: {sel.path}
        </div>
      )}
    </Dialog>
  );
}

export { personLabel };
