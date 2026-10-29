// Table views: Phone List, By Company, By Location — sortable columns, group headers that
// collapse, "Click here to add a new Contact" row (Outlook 2007).
import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Icon, errorBox, usePersistentState } from "../../ui";
import { contactsApi, fullName, mainAddress, parseFullName, phone, phonesFromLabels, type Contact } from "../../api/contacts";
import { currentAccountId, currentGroupResource, openContact } from "./actions";
import { useSelection } from "./selection";
import { columnDate, compareText, isDateKey, isSortKey, shownFileAs, sortContacts, type NameOrder, type SortKey } from "./sort";
import { onViewBackgroundContextMenu } from "./sortMenu";
import { useContactsUi } from "./store";

type ListKind = "phone" | "company" | "location";

interface Col {
  id: string;
  label: string;
  width: number;
  get: (c: Contact, order: NameOrder) => string;
  /** View › Sort By order this column's header click selects (else the column sorts by its text) */
  sortKey?: SortKey;
  /** editable in the "Click here to add a new Contact" row */
  newField?: string;
}

const COLS: Record<string, Col> = {
  fullName: { id: "fullName", label: "Full Name", width: 170, get: (c) => fullName(c), sortKey: "firstName", newField: "fullName" },
  jobTitle: { id: "jobTitle", label: "Job Title", width: 150, get: (c) => c.jobTitle },
  company: { id: "company", label: "Company", width: 160, get: (c) => c.company, sortKey: "company", newField: "company" },
  fileAs: { id: "fileAs", label: "File As", width: 170, get: (c, o) => shownFileAs(c, o), sortKey: "fileAs" },
  department: { id: "department", label: "Department", width: 110, get: (c) => c.department },
  state: { id: "state", label: "State", width: 70, get: (c) => mainAddress(c)?.region ?? "" },
  country: { id: "country", label: "Country/Region", width: 130, get: (c) => mainAddress(c)?.country ?? "" },
  business: { id: "business", label: "Business Phone", width: 125, get: (c) => phone(c, "Business"), newField: "Business" },
  businessFax: { id: "businessFax", label: "Business Fax", width: 115, get: (c) => phone(c, "Business Fax"), newField: "Business Fax" },
  home: { id: "home", label: "Home Phone", width: 115, get: (c) => phone(c, "Home"), newField: "Home" },
  mobile: { id: "mobile", label: "Mobile Phone", width: 125, get: (c) => phone(c, "Mobile"), newField: "Mobile" },
  email: { id: "email", label: "E-mail", width: 210, get: (c) => c.emails[0]?.value ?? "", newField: "email" },
  added: { id: "added", label: "Added", width: 150, get: (c) => columnDate(c.firstSeenAt), sortKey: "added" },
  modified: { id: "modified", label: "Modified", width: 150, get: (c) => columnDate(c.updated), sortKey: "modified" },
};

const LAYOUTS: Record<ListKind, { cols: string[]; group?: { label: string; get: (c: Contact) => string } }> = {
  phone: { cols: ["fullName", "company", "fileAs", "business", "businessFax", "home", "mobile", "email", "added", "modified"] },
  company: {
    cols: ["fullName", "jobTitle", "company", "fileAs", "department", "business", "businessFax", "home", "mobile"],
    group: { label: "Company", get: (c) => c.company },
  },
  location: {
    cols: ["fullName", "company", "fileAs", "state", "country", "business", "businessFax", "home", "mobile"],
    group: { label: "Country/Region", get: (c) => mainAddress(c)?.country ?? "" },
  },
};

type Row = { type: "group"; key: string; count: number } | { type: "item"; index: number };

const ROW_H = 19;
const GROUP_H = 25;

export function ListView({ items, kind }: { items: Contact[]; kind: ListKind }) {
  const layout = LAYOUTS[kind];
  const [widths, setWidths] = usePersistentState<Record<string, number>>(`contacts.cols.${kind}`, {});
  // one sort for every view: View › Sort By, or a column header click here
  const sort = useContactsUi((s) => s.sort);
  const setSort = useContactsUi((s) => s.setSort);
  const nameOrder = useContactsUi((s) => s.nameOrder);
  const setItems = useContactsUi((s) => s.setItems);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const cols = layout.cols.map((id) => ({ ...COLS[id], width: widths[id] ?? COLS[id].width }));

  // sort, then group (items stay in display order so selection ranges follow what you see)
  const ordered = useMemo(() => {
    let arr: Contact[];
    if (isSortKey(sort.key)) arr = sortContacts(items, sort, nameOrder);
    else {
      // a table-only column (phones, e-mail, job title...)
      const col = COLS[sort.key] ?? COLS.fileAs;
      arr = [...items].sort((a, b) => {
        const r = compareText(col.get(a, nameOrder), col.get(b, nameOrder)) || compareText(shownFileAs(a, nameOrder), shownFileAs(b, nameOrder));
        return sort.desc ? -r : r;
      });
    }
    if (layout.group) {
      const g = layout.group.get;
      arr.sort((a, b) => {
        const ga = g(a);
        const gb = g(b);
        if (!ga !== !gb) return ga ? -1 : 1; // "(none)" last
        return compareText(ga, gb);
      });
    }
    return arr;
  }, [items, sort, nameOrder, layout]);
  useEffect(() => setItems(ordered), [ordered, setItems]);

  const rows: Row[] = useMemo(() => {
    if (!layout.group) return ordered.map((_, index) => ({ type: "item", index }));
    const out: Row[] = [];
    let cur: string | null = null;
    let header: { type: "group"; key: string; count: number } | null = null;
    ordered.forEach((c, index) => {
      const k = layout.group!.get(c);
      if (k !== cur || header === null) {
        cur = k;
        header = { type: "group", key: k, count: 0 };
        out.push(header);
      }
      header.count++;
      if (!collapsed.has(k)) out.push({ type: "item", index });
    });
    return out;
  }, [ordered, layout, collapsed]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const virt = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => (rows[i].type === "group" ? GROUP_H : ROW_H),
    overscan: 12,
  });
  const rowOfIndex = useMemo(() => {
    const m = new Map<number, number>();
    rows.forEach((r, ri) => r.type === "item" && m.set(r.index, ri));
    return m;
  }, [rows]);
  const sel = useSelection(ordered, (i) => {
    const ri = rowOfIndex.get(i);
    if (ri !== undefined) virt.scrollToIndex(ri, { align: "auto" });
  });
  const totalW = 24 + cols.reduce((s, c) => s + c.width, 0);

  const startResize = (id: string, e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX;
    const w0 = widths[id] ?? COLS[id].width;
    const move = (ev: MouseEvent) => setWidths((w) => ({ ...w, [id]: Math.max(40, w0 + ev.clientX - x0) }));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  return (
    <div
      ref={scrollRef}
      className="cm-contacts-grid"
      tabIndex={0}
      onContextMenu={onViewBackgroundContextMenu}
      onKeyDown={(e) => {
        if ((e.target as HTMLElement).tagName === "INPUT") return;
        sel.onKeyDown(e, { pageStep: Math.max(1, Math.floor((scrollRef.current?.clientHeight ?? 400) / ROW_H) - 2) });
      }}
    >
      <div style={{ width: Math.max(totalW, 100), minWidth: "100%" }}>
        <div className="cm-contacts-grid-head" style={{ width: totalW }}>
          <div className="cm-contacts-grid-hcell icon" style={{ width: 24 }}>
            <Icon name="contact" size={14} />
          </div>
          {cols.map((c) => {
            const key = c.sortKey ?? c.id;
            const sorted = sort.key === key;
            return (
              <div
                key={c.id}
                className={`cm-contacts-grid-hcell${sorted ? " sorted" : ""}`}
                style={{ width: c.width }}
                title={c.sortKey === "firstName" ? "Sort by First Name" : undefined}
                onClick={() => setSort(key, sorted ? !sort.desc : isDateKey(key))}
              >
                <span className="cm-ellipsis">{c.label}</span>
                {sorted && <span className="cm-contacts-sortarrow">{sort.desc ? "▼" : "▲"}</span>}
                <span className="cm-contacts-grid-resize" onMouseDown={(e) => startResize(c.id, e)} onClick={(e) => e.stopPropagation()} />
              </div>
            );
          })}
        </div>
        <NewContactRow cols={cols} totalW={totalW} />
        <div style={{ height: virt.getTotalSize(), position: "relative", width: totalW }}>
          {virt.getVirtualItems().map((v) => {
            const r = rows[v.index];
            if (r.type === "group") {
              const open = !collapsed.has(r.key);
              return (
                <div
                  key={v.key}
                  className="cm-contacts-grid-group"
                  style={{ top: v.start, height: GROUP_H }}
                  onClick={() =>
                    setCollapsed((s) => {
                      const n = new Set(s);
                      if (open) n.add(r.key);
                      else n.delete(r.key);
                      return n;
                    })
                  }
                >
                  <span className="cm-contacts-grid-toggle">{open ? "−" : "+"}</span>
                  <span dir="auto">
                    {layout.group!.label}: {r.key || "(none)"}
                  </span>
                  <span className="cm-contacts-grid-count">
                    ({r.count} {r.count === 1 ? "item" : "items"})
                  </span>
                </div>
              );
            }
            const c = ordered[r.index];
            const selected = sel.selSet.has(c.id);
            return (
              <div
                key={v.key}
                className={`cm-contacts-grid-row${selected ? " selected" : ""}`}
                data-id={c.id}
                draggable
                style={{ top: v.start, height: ROW_H }}
                onMouseDown={(e) => sel.onItemMouseDown(r.index, e)}
                onDoubleClick={() => openContact(c)}
                onContextMenu={(e) => sel.onItemContextMenu(r.index, e)}
              >
                <div className="cm-contacts-grid-cell icon" style={{ width: 24 }}>
                  <Icon name="contact" size={14} />
                </div>
                {cols.map((col) => (
                  <div key={col.id} className="cm-contacts-grid-cell cm-ellipsis" style={{ width: col.width }} dir="auto">
                    {col.get(c, nameOrder)}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
        {items.length === 0 && <div className="cm-empty">There are no items to show in this view.</div>}
      </div>
    </div>
  );
}

/** "Click here to add a new Contact" — type straight into the grid, Enter saves. */
function NewContactRow({ cols, totalW }: { cols: Col[]; totalW: number }) {
  const [editing, setEditing] = useState(false);
  const [vals, setVals] = useState<Record<string, string>>({});
  const reset = () => {
    setVals({});
    setEditing(false);
  };
  const save = async () => {
    if (!Object.values(vals).some((v) => v.trim())) return reset();
    const phones: Record<string, string> = {};
    for (const k of ["Business", "Business Fax", "Home", "Mobile"]) if (vals[k]) phones[k] = vals[k];
    try {
      await contactsApi.create(
        {
          ...parseFullName(vals.fullName ?? ""),
          company: vals.company ?? "",
          phones: phonesFromLabels(phones),
          emails: vals.email ? [{ value: vals.email.trim(), type: "work", displayName: "" }] : [],
          groups: currentGroupResource(),
        },
        currentAccountId(),
        `New Contact "${(vals.fullName ?? vals.company ?? vals.email ?? "").trim()}"`,
      );
      reset();
    } catch (e) {
      errorBox(e);
    }
  };
  if (!editing)
    return (
      <div className="cm-contacts-grid-new" style={{ width: totalW }} onClick={() => setEditing(true)}>
        <div className="cm-contacts-grid-cell icon" style={{ width: 24 }} />
        <span className="cm-muted">Click here to add a new Contact</span>
      </div>
    );
  return (
    <div className="cm-contacts-grid-new editing" style={{ width: totalW }}>
      <div className="cm-contacts-grid-cell icon" style={{ width: 24 }}>
        <Icon name="contact" size={14} />
      </div>
      {cols.map((c, i) =>
        c.newField ? (
          <input
            key={c.id}
            className="cm-contacts-grid-input"
            style={{ width: c.width }}
            autoFocus={i === 0}
            value={vals[c.newField] ?? ""}
            onChange={(e) => setVals((v) => ({ ...v, [c.newField!]: e.target.value }))}
            onKeyDown={(e) => {
              if (e.key === "Enter") save();
              else if (e.key === "Escape") reset();
            }}
          />
        ) : (
          <div key={c.id} className="cm-contacts-grid-cell" style={{ width: c.width }} />
        ),
      )}
    </div>
  );
}
