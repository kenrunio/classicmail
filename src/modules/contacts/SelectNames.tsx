// "Select Names: Contacts" (the To… / Cc… dialog compose opens) and the address-list picker it
// shares with the Address Book window.
import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Dialog, Icon, keyOf } from "../../ui";
import { useRpc } from "../../api/query";
import { emailDisplayAs, type Contact, type ContactFolder } from "../../api/contacts";
import { folderLabel } from "./actions";
import "./contacts.css";

export interface Addr {
  name: string;
  email: string;
}

export interface AddrRow {
  key: string;
  contact: Contact;
  name: string;
  display: string;
  email: string;
}

/** One row per e-mail address of every contact in the account's Contacts folder. */
export function useAddressRows(accountId: number | null): { rows: AddrRow[]; loading: boolean } {
  const list = useRpc<Contact[]>(["contacts", "list", accountId, null, ""], "contacts.list", { accountId, groupId: null }, { enabled: accountId != null });
  const rows = useMemo(() => {
    const out: AddrRow[] = [];
    for (const c of list.data ?? []) {
      c.emails.forEach((e, i) => {
        out.push({ key: `${c.id}:${i}`, contact: c, name: c.displayName || e.value, display: e.displayName || emailDisplayAs(c.displayName, e.value), email: e.value });
      });
    }
    out.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
    return out;
  }, [list.data]);
  return { rows, loading: list.isLoading };
}

export function useContactFolders() {
  return useRpc<ContactFolder[]>(["contacts", "folders"], "contacts.folders");
}

/**
 * Search box + address list selector + list (Name / Display Name / E-mail Address / E-mail Type).
 * "Name only" type-down selects the first match like Outlook; "More columns" filters on Go.
 */
export function AddressPicker({
  accountId,
  setAccountId,
  selected,
  setSelected,
  onActivate,
  onContextMenu,
  autoFocus,
}: {
  accountId: number | null;
  setAccountId: (id: number) => void;
  selected: string[];
  setSelected: (keys: string[]) => void;
  onActivate: (rows: AddrRow[]) => void;
  onContextMenu?: (e: React.MouseEvent, rows: AddrRow[]) => void;
  autoFocus?: boolean;
}) {
  const folders = useContactFolders().data ?? [];
  const { rows, loading } = useAddressRows(accountId);
  const [mode, setMode] = useState<"name" | "more">("name");
  const [text, setText] = useState("");
  const [filter, setFilter] = useState("");
  const anchor = useRef<number | null>(null);
  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    if (mode !== "more" || !f) return rows;
    return rows.filter((r) => [r.name, r.display, r.email, r.contact.company].some((x) => x.toLowerCase().includes(f)));
  }, [rows, filter, mode]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const virt = useVirtualizer({ count: shown.length, getScrollElement: () => scrollRef.current, estimateSize: () => 19, overscan: 10 });
  const sel = new Set(selected);
  const cols = "minmax(150px, 1.1fr) minmax(170px, 1.4fr) minmax(170px, 1.3fr) 70px";

  useEffect(() => {
    if (mode !== "name") return;
    const t = text.trim().toLowerCase();
    if (!t) return;
    const i = shown.findIndex((r) => r.name.toLowerCase().startsWith(t) || r.email.toLowerCase().startsWith(t));
    if (i >= 0) {
      setSelected([shown[i].key]);
      anchor.current = i;
      virt.scrollToIndex(i, { align: "start" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, mode]);

  const rowsFor = (keys: string[]) => shown.filter((r) => keys.includes(r.key));
  const click = (i: number, e: React.MouseEvent) => {
    const r = shown[i];
    if (e.shiftKey && anchor.current != null) {
      const [a, b] = [Math.min(anchor.current, i), Math.max(anchor.current, i)];
      setSelected(shown.slice(a, b + 1).map((x) => x.key));
    } else if (e.ctrlKey) {
      setSelected(sel.has(r.key) ? selected.filter((k) => k !== r.key) : [...selected, r.key]);
      anchor.current = i;
    } else if (!(e.button === 2 && sel.has(r.key))) {
      setSelected([r.key]);
      anchor.current = i;
    }
  };
  const move = (d: number, extend: boolean) => {
    if (!shown.length) return;
    const cur = shown.findIndex((r) => r.key === selected[selected.length - 1]);
    const i = Math.max(0, Math.min(shown.length - 1, (cur < 0 ? -1 : cur) + d));
    if (extend && anchor.current != null) {
      const [a, b] = [Math.min(anchor.current, i), Math.max(anchor.current, i)];
      const keys = shown
        .slice(a, b + 1)
        .map((x) => x.key)
        .filter((k) => k !== shown[i].key);
      setSelected([...keys, shown[i].key]);
    } else {
      setSelected([shown[i].key]);
      anchor.current = i;
    }
    virt.scrollToIndex(i, { align: "auto" });
  };

  return (
    <div className="cm-contacts-ab">
      <div className="cm-contacts-ab-top">
        <div style={{ display: "flex", gap: 4, gridColumn: "1 / 3", gridRow: 2 }}>
          <input
            className="cm-input"
            style={{ flex: 1 }}
            autoFocus={autoFocus}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                if (mode === "more") setFilter(text);
                else if (selected.length) onActivate(rowsFor(selected));
              } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                move(e.key === "ArrowDown" ? 1 : -1, e.shiftKey);
              }
            }}
          />
          <button className="cm-button" style={{ minWidth: 40 }} onClick={() => setFilter(mode === "more" ? text : "")}>
            Go
          </button>
        </div>
        <span style={{ gridRow: 1, gridColumn: 1 }}>Search:</span>
        <div className="cm-contacts-ab-radios" style={{ gridRow: 1, gridColumn: 2 }}>
          <label>
            <input type="radio" checked={mode === "name"} onChange={() => setMode("name")} /> Name only
          </label>
          <label>
            <input type="radio" checked={mode === "more"} onChange={() => setMode("more")} /> More columns
          </label>
        </div>
        <span style={{ gridRow: 1, gridColumn: 3 }}>Address Book</span>
        <select className="cm-input" style={{ gridRow: 2, gridColumn: 3 }} value={accountId ?? ""} onChange={(e) => setAccountId(Number(e.target.value))} aria-label="Address Book">
          {folders.map((f, i) => (
            <option key={f.accountId} value={f.accountId}>
              {folderLabel(f, i)}
            </option>
          ))}
        </select>
        <span className="cm-link" style={{ gridRow: 2, gridColumn: 4, color: "var(--text-disabled)", textDecoration: "none" }} title="Not available">
          Advanced Find
        </span>
      </div>
      <div
        ref={scrollRef}
        className="cm-contacts-ab-list"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            move(e.key === "ArrowDown" ? 1 : -1, e.shiftKey);
          } else if (e.key === "Enter") {
            e.preventDefault();
            onActivate(rowsFor(selected));
          } else if (e.ctrlKey && keyOf(e) === "a") {
            e.preventDefault();
            setSelected(shown.map((r) => r.key));
          }
        }}
      >
        <div className="cm-contacts-ab-head" style={{ gridTemplateColumns: cols }}>
          <div>Name</div>
          <div>Display Name</div>
          <div>E-mail Address</div>
          <div>E-mail Type</div>
        </div>
        <div style={{ height: virt.getTotalSize(), position: "relative" }}>
          {virt.getVirtualItems().map((v) => {
            const r = shown[v.index];
            return (
              <div
                key={r.key}
                className={`cm-contacts-ab-row${sel.has(r.key) ? " selected" : ""}`}
                style={{ position: "absolute", top: v.start, left: 0, right: 0, gridTemplateColumns: cols }}
                onMouseDown={(e) => click(v.index, e)}
                onDoubleClick={() => onActivate([r])}
                onContextMenu={(e) => {
                  e.preventDefault();
                  onContextMenu?.(e, sel.has(r.key) ? rowsFor(selected) : [r]);
                }}
              >
                <div dir="auto">
                  <Icon name="contact" size={14} />
                  <span className="cm-ellipsis">{r.name}</span>
                </div>
                <div dir="auto">{r.display}</div>
                <div>{r.email}</div>
                <div>SMTP</div>
              </div>
            );
          })}
        </div>
        {!loading && shown.length === 0 && <div className="cm-empty">No entries.</div>}
      </div>
    </div>
  );
}

const tokenOf = (a: Addr) => a.name || a.email;

/** Parse a well's text ("Sarah Cohen; bob@x.com; Name <a@b>") back into addresses. */
function parseWell(text: string, known: Map<string, Addr>, rows: AddrRow[]): Addr[] {
  return text
    .split(";")
    .map((t) => t.trim())
    .filter(Boolean)
    .map((t) => {
      const k = known.get(t);
      if (k) return k;
      const m = t.match(/^"?([^"<]*?)"?\s*<([^>]+)>$/);
      if (m) return { name: m[1].trim(), email: m[2].trim() };
      if (t.includes("@")) return { name: "", email: t };
      const hit = rows.find((r) => r.name.toLowerCase() === t.toLowerCase() || r.display.toLowerCase() === t.toLowerCase());
      return hit ? { name: hit.name, email: hit.email } : { name: t, email: "" };
    });
}

export function SelectNamesDialog(props: { initial: { to: Addr[]; cc: Addr[]; bcc: Addr[] }; onOk: (v: { to: Addr[]; cc: Addr[]; bcc: Addr[] }) => void; onClose: () => void }) {
  const folders = useContactFolders().data ?? [];
  const [accountId, setAccountId] = useState<number | null>(null);
  const acct = accountId ?? folders[0]?.accountId ?? null;
  const { rows } = useAddressRows(acct);
  const [selected, setSelected] = useState<string[]>([]);
  const known = useRef(new Map<string, Addr>());
  const fmt = (list: Addr[]) => {
    for (const a of list) known.current.set(tokenOf(a), a);
    return list.map(tokenOf).join("; ");
  };
  const [wells, setWells] = useState(() => ({ to: fmt(props.initial.to), cc: fmt(props.initial.cc), bcc: fmt(props.initial.bcc) }));
  const [focusWell, setFocusWell] = useState<"to" | "cc" | "bcc">("to");

  const add = (well: "to" | "cc" | "bcc", list: AddrRow[]) => {
    if (!list.length) return;
    const addrs = list.map((r) => ({ name: r.name, email: r.email }));
    setWells((w) => {
      const existing = w[well].trim();
      const have = new Set(parseWell(existing, known.current, rows).map((a) => a.email.toLowerCase()));
      const fresh = addrs.filter((a) => !have.has(a.email.toLowerCase()));
      const text = fmt(fresh);
      return { ...w, [well]: [existing.replace(/;\s*$/, ""), text].filter(Boolean).join("; ") };
    });
  };
  const ok = () => {
    props.onOk({
      to: parseWell(wells.to, known.current, rows),
      cc: parseWell(wells.cc, known.current, rows),
      bcc: parseWell(wells.bcc, known.current, rows),
    });
    props.onClose();
  };
  const selRows = () => rows.filter((r) => selected.includes(r.key));
  const title = `Select Names: ${
    acct != null && folders.length
      ? folderLabel(
          folders.find((f) => f.accountId === acct) ?? folders[0],
          Math.max(
            0,
            folders.findIndex((f) => f.accountId === acct),
          ),
        )
      : "Contacts"
  }`;

  return (
    <Dialog
      title={title}
      width={680}
      height={560}
      onClose={props.onClose}
      footer={
        <>
          <button className="cm-button default" onClick={ok}>
            OK
          </button>
          <button className="cm-button" onClick={props.onClose}>
            Cancel
          </button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 8, height: "100%" }}>
        <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
          <AddressPicker accountId={acct} setAccountId={setAccountId} selected={selected} setSelected={setSelected} onActivate={(r) => add(focusWell, r)} autoFocus />
        </div>
        <div className="cm-contacts-sn-wells">
          {(["to", "cc", "bcc"] as const).map((w) => (
            <WellRow key={w} well={w} value={wells[w]} onAdd={() => add(w, selRows())} onChange={(v) => setWells((x) => ({ ...x, [w]: v }))} onFocus={() => setFocusWell(w)} />
          ))}
        </div>
      </div>
    </Dialog>
  );
}

function WellRow({ well, value, onAdd, onChange, onFocus }: { well: "to" | "cc" | "bcc"; value: string; onAdd: () => void; onChange: (v: string) => void; onFocus: () => void }) {
  const label = well === "to" ? "To ->" : well === "cc" ? "Cc ->" : "Bcc ->";
  return (
    <>
      <button className="cm-button" onClick={onAdd} onFocus={onFocus}>
        {label}
      </button>
      <input className="cm-input" dir="auto" value={value} onChange={(e) => onChange(e.target.value)} onFocus={onFocus} aria-label={label} />
    </>
  );
}
