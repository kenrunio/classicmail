// Mail dialogs: Move/Copy Items, Create New Folder, Rename, Custom flag, Message Options.
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { create } from "zustand";
import { Dialog, TreeView, errorBox, type TreeNode } from "../../ui";
import { mailApi, type MailFolder, type Mailbox } from "../../api/mail";
import { useRpc } from "../../api/query";
import { undoable } from "../../api/undo";
import { canReceive, mailboxNode, type NodeData } from "./folders";
import { todayYmd } from "./format";

// ---- host --------------------------------------------------------------------------------------

const useDialogs = create<{ stack: { key: number; node: ReactNode }[] }>(() => ({ stack: [] }));
let dlgKey = 0;

/** Show a dialog; `render(close)` returns the dialog element. */
export function showMailDialog(render: (close: () => void) => ReactNode) {
  const key = ++dlgKey;
  const close = () => useDialogs.setState((s) => ({ stack: s.stack.filter((d) => d.key !== key) }));
  useDialogs.setState((s) => ({ stack: [...s.stack, { key, node: render(close) }] }));
}

export function MailDialogHost() {
  const stack = useDialogs((s) => s.stack);
  return (
    <>
      {stack.map((d) => (
        <div key={d.key}>{d.node}</div>
      ))}
    </>
  );
}

// ---- folder picker -----------------------------------------------------------------------------

function FolderPicker({
  mailboxes,
  selected,
  onSelect,
  onDouble,
  accountId,
  autoFocus,
}: {
  mailboxes: Mailbox[];
  selected: number | null;
  onSelect: (f: MailFolder) => void;
  onDouble?: () => void;
  accountId?: number;
  autoFocus?: boolean;
}) {
  const nodes = useMemo(() => mailboxes.filter((m) => accountId === undefined || m.accountId === accountId).map((m) => mailboxNode(m)), [mailboxes, accountId]);
  const [expanded, setExpanded] = useState<Set<string>>(() => {
    const s = new Set<string>();
    const walk = (n: TreeNode) => {
      s.add(n.id);
      n.children?.forEach(walk);
    };
    nodes.forEach(walk);
    return s;
  });
  return (
    <div className="cm-mail-picker" onDoubleClick={onDouble}>
      <TreeView
        nodes={nodes}
        autoFocus={autoFocus}
        onActivate={() => onDouble?.()}
        selectedId={selected ? `f${selected}` : null}
        expanded={expanded}
        onToggle={(id, open) =>
          setExpanded((s) => {
            const n = new Set(s);
            if (open) n.add(id);
            else n.delete(id);
            return n;
          })
        }
        onSelect={(n) => {
          const d = n.data as NodeData | undefined;
          if (d?.kind === "folder") onSelect(d.folder);
        }}
      />
    </div>
  );
}

/** "Move Items" / "Copy Items": resolves with the chosen folder (or null). */
export function pickFolder(opts: { title: string; prompt: string; accountId?: number; initial?: number }): Promise<MailFolder | null> {
  return new Promise((resolve) => {
    showMailDialog((close) => (
      <PickFolderDialog
        {...opts}
        onDone={(f) => {
          close();
          resolve(f);
        }}
      />
    ));
  });
}

function PickFolderDialog({
  title,
  prompt,
  accountId,
  initial,
  onDone,
}: {
  title: string;
  prompt: string;
  accountId?: number;
  initial?: number;
  onDone: (f: MailFolder | null) => void;
}) {
  const data = useRpc<Mailbox[]>(["mail", "folders"], "mail.listFolders").data;
  const mbs = useMemo(() => data ?? [], [data]);
  const [sel, setSel] = useState<MailFolder | null>(null);
  useEffect(() => {
    if (!sel && initial) {
      for (const m of mbs) {
        const f = m.folders.find((x) => x.id === initial);
        if (f) setSel(f);
      }
    }
  }, [mbs, initial, sel]);
  const ok = sel && canReceive(sel);
  return (
    <Dialog
      title={title}
      width={360}
      onClose={() => onDone(null)}
      footer={
        <>
          <button className="cm-button default" disabled={!ok} onClick={() => ok && onDone(sel)}>
            OK
          </button>
          <button className="cm-button" onClick={() => onDone(null)}>
            Cancel
          </button>
          <button
            className="cm-button"
            disabled={!sel}
            onClick={async () => {
              const id = await newFolderDialog(sel ?? undefined, accountId);
              if (id) {
                for (const m of mbs) {
                  const f = m.folders.find((x) => x.id === id);
                  if (f) setSel(f);
                }
              }
            }}
          >
            New…
          </button>
        </>
      }
    >
      <div style={{ marginBottom: 6 }}>{prompt}</div>
      <FolderPicker mailboxes={mbs} selected={sel?.id ?? null} onSelect={setSel} onDouble={() => ok && onDone(sel)} accountId={accountId} autoFocus />
    </Dialog>
  );
}

/** "Create New Folder": name + where to place it. Resolves with the new folder id. */
export function newFolderDialog(parent?: MailFolder, accountId?: number): Promise<number | null> {
  return new Promise((resolve) => {
    showMailDialog((close) => (
      <NewFolderDialog
        parent={parent}
        accountId={accountId ?? parent?.accountId}
        onDone={(id) => {
          close();
          resolve(id);
        }}
      />
    ));
  });
}

function NewFolderDialog({ parent, accountId, onDone }: { parent?: MailFolder; accountId?: number; onDone: (id: number | null) => void }) {
  const mbs = useRpc<Mailbox[]>(["mail", "folders"], "mail.listFolders").data ?? [];
  const [name, setName] = useState("");
  const [sel, setSel] = useState<MailFolder | null>(parent ?? null);
  const [busy, setBusy] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);
  const submit = async () => {
    if (!name.trim()) return;
    setBusy(true);
    try {
      // the folder picked is the parent (Inbox too: its subfolders are "Inbox/<name>")
      const parentId = sel?.id;
      const acct = sel?.accountId ?? accountId ?? mbs[0]?.accountId;
      onDone(await undoable<number>(`New Folder "${name.trim()}"`, "mail.createFolder", { name: name.trim(), parentId, accountId: acct }));
    } catch (e) {
      setBusy(false);
      await errorBox(e, "Create New Folder");
      // ready to type another name
      nameRef.current?.focus();
      nameRef.current?.select();
    }
  };
  return (
    <Dialog
      title="Create New Folder"
      width={340}
      onClose={() => onDone(null)}
      footer={
        <>
          <button className="cm-button default" disabled={!name.trim() || busy} onClick={submit}>
            OK
          </button>
          <button className="cm-button" onClick={() => onDone(null)}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-mail-form">
        <label>Name:</label>
        <input
          ref={nameRef}
          className="cm-input"
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              submit();
            }
          }}
        />
        <label>Folder contains:</label>
        <select className="cm-input" disabled>
          <option>Mail and Post Items</option>
        </select>
        <label>Select where to place the folder:</label>
      </div>
      <FolderPicker mailboxes={mbs} selected={sel?.id ?? null} onSelect={setSel} accountId={accountId} />
    </Dialog>
  );
}

/** Simple one-line prompt (Rename). */
export function promptDialog(title: string, label: string, initial: string, allowEmpty = false): Promise<string | null> {
  return new Promise((resolve) => {
    showMailDialog((close) => {
      const done = (v: string | null) => {
        close();
        resolve(v);
      };
      return <PromptDialog title={title} label={label} initial={initial} allowEmpty={allowEmpty} onDone={done} />;
    });
  });
}

function PromptDialog({ title, label, initial, allowEmpty, onDone }: { title: string; label: string; initial: string; allowEmpty?: boolean; onDone: (v: string | null) => void }) {
  const [v, setV] = useState(initial);
  const ok = allowEmpty || !!v.trim();
  return (
    <Dialog
      title={title}
      width={340}
      onClose={() => onDone(null)}
      footer={
        <>
          <button className="cm-button default" disabled={!ok} onClick={() => onDone(v.trim())}>
            OK
          </button>
          <button className="cm-button" onClick={() => onDone(null)}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-mail-form">
        <label>{label}</label>
        <input
          className="cm-input"
          autoFocus
          value={v}
          onFocus={(e) => e.currentTarget.select()}
          onChange={(e) => setV(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && ok) {
              e.preventDefault();
              onDone(v.trim());
            }
          }}
        />
      </div>
    </Dialog>
  );
}

/** Follow Up › Custom…: due date. */
export function customFlagDialog(): Promise<string | null> {
  return new Promise((resolve) => {
    showMailDialog((close) => (
      <CustomFlagDialog
        onDone={(v) => {
          close();
          resolve(v);
        }}
      />
    ));
  });
}

function CustomFlagDialog({ onDone }: { onDone: (v: string | null) => void }) {
  const [due, setDue] = useState(todayYmd());
  return (
    <Dialog
      title="Custom"
      width={360}
      onClose={() => onDone(null)}
      footer={
        <>
          <button className="cm-button default" onClick={() => onDone(due)}>
            OK
          </button>
          <button className="cm-button" onClick={() => onDone(null)}>
            Cancel
          </button>
        </>
      }
    >
      <div style={{ marginBottom: 8 }}>Flagging a message creates a to-do item that reminds you to follow up. After you follow up, you can mark the to-do item complete.</div>
      <div className="cm-mail-form">
        <label>Flag to:</label>
        <select className="cm-input" disabled>
          <option>Follow up</option>
        </select>
        <label>Due date:</label>
        <input className="cm-input" type="date" value={due} onChange={(e) => setDue(e.target.value)} />
      </div>
    </Dialog>
  );
}

/** Message Options (Internet headers). */
export function messageOptionsDialog(id: number, importance: number) {
  showMailDialog((close) => <MessageOptions id={id} importance={importance} onClose={close} />);
}

function MessageOptions({ id, importance, onClose }: { id: number; importance: number; onClose: () => void }) {
  const [headers, setHeaders] = useState<string>("Loading…");
  useEffect(() => {
    mailApi.headers(id).then(setHeaders, (e) => setHeaders(String(e?.message ?? e)));
  }, [id]);
  return (
    <Dialog
      title="Message Options"
      width={560}
      onClose={onClose}
      footer={
        <button className="cm-button default" onClick={onClose}>
          Close
        </button>
      }
    >
      <fieldset className="cm-mail-fieldset">
        <legend>Message settings</legend>
        <div className="cm-mail-form">
          <label>Importance:</label>
          <select className="cm-input" disabled value={importance}>
            <option value={0}>Low</option>
            <option value={1}>Normal</option>
            <option value={2}>High</option>
          </select>
          <label>Sensitivity:</label>
          <select className="cm-input" disabled>
            <option>Normal</option>
          </select>
        </div>
      </fieldset>
      <div style={{ margin: "8px 0 3px" }}>Internet headers:</div>
      <textarea className="cm-input cm-mail-headers" readOnly value={headers} />
    </Dialog>
  );
}
