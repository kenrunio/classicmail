// Business Cards / Address Cards / Detailed Address Cards: cards flow top-to-bottom in columns
// that scroll sideways, with the A–Z index tabs down the right edge (Outlook 2007).
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { addressLabel, addressLines, fullName, INDEX_ORDER, mainAddress, phonesByLabel, formatDate, type Contact } from "../../api/contacts";
import { BusinessCard } from "./BusinessCard";
import { openContact } from "./actions";
import { useSelection } from "./selection";
import { indexKeyFor, isDateKey, rangeText as rangeOf, shownFileAs, sortText, type NameOrder, type SortKey } from "./sort";
import { onViewBackgroundContextMenu } from "./sortMenu";

type CardKind = "business" | "address" | "detailed";

interface Field {
  label?: string;
  value: string;
}

const PHONE_ORDER = [
  "Business",
  "Business 2",
  "Home",
  "Home 2",
  "Business Fax",
  "Mobile",
  "Company Main",
  "Assistant",
  "Pager",
  "Other",
  "Home Fax",
  "Other Fax",
  "Car",
  "Primary",
];

function phoneFields(c: Contact): Field[] {
  const map = phonesByLabel(c.phones);
  const labels = [...PHONE_ORDER, ...Object.keys(map).filter((l) => !PHONE_ORDER.includes(l))];
  return labels.filter((l) => map[l]).map((l) => ({ label: `${l}:`, value: map[l] }));
}

function emailFields(c: Contact): Field[] {
  return c.emails.map((e, i) => ({ label: i === 0 ? "E-mail:" : `E-mail ${i + 1}:`, value: e.value }));
}

/** Lines of an address card (the order Outlook uses). */
export function cardFields(c: Contact, kind: CardKind): Field[] {
  if (kind === "detailed") {
    const out: Field[] = [];
    const name = fullName(c);
    if (name) out.push({ label: "Full Name:", value: name });
    if (c.jobTitle) out.push({ label: "Job Title:", value: c.jobTitle });
    if (c.company) out.push({ label: "Company:", value: c.company });
    if (c.department) out.push({ label: "Department:", value: c.department });
    for (const a of c.addresses) {
      addressLines(a).forEach((l, i) => out.push({ label: i === 0 ? `${addressLabel(a.type)} Address:` : "", value: l }));
    }
    out.push(...phoneFields(c), ...emailFields(c));
    if (c.urls[0]) out.push({ label: "Web Page:", value: c.urls[0].value });
    if (c.nickname) out.push({ label: "Nickname:", value: c.nickname });
    if (c.birthday) out.push({ label: "Birthday:", value: formatDate(c.birthday) });
    return out;
  }
  const out: Field[] = addressLines(mainAddress(c)).map((value) => ({ value }));
  out.push(...phoneFields(c), ...emailFields(c));
  return out;
}

const LINE = 15;
const HEAD = 19;
const PAD = 7;
const GAP = 10;
const BCARD_H = 126;

function cardHeight(c: Contact, kind: CardKind): number {
  if (kind === "business") return BCARD_H;
  return HEAD + Math.max(1, cardFields(c, kind).length) * LINE + PAD;
}

function colWidth(kind: CardKind) {
  return kind === "business" ? 248 : kind === "detailed" ? 268 : 226;
}

export function CardView({
  items,
  kind,
  sortKey = "fileAs",
  sortDesc = false,
  nameOrder = "lastFirst",
  onRange,
}: {
  items: Contact[];
  kind: CardKind;
  sortKey?: SortKey;
  sortDesc?: boolean;
  nameOrder?: NameOrder;
  onRange?: (text: string) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(400);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    setHeight(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  // pack cards into columns
  const columns = useMemo(() => {
    const avail = Math.max(120, height - 2 * GAP);
    const cols: number[][] = [];
    let col: number[] = [];
    let h = 0;
    items.forEach((c, i) => {
      const ch = cardHeight(c, kind) + GAP;
      if (col.length && h + ch > avail + GAP) {
        cols.push(col);
        col = [];
        h = 0;
      }
      col.push(i);
      h += ch;
    });
    if (col.length) cols.push(col);
    return cols;
  }, [items, kind, height]);
  const colOf = useMemo(() => {
    const m = new Map<number, number>();
    columns.forEach((col, ci) => col.forEach((i) => m.set(i, ci)));
    return m;
  }, [columns]);

  const width = colWidth(kind);
  const virt = useVirtualizer({
    horizontal: true,
    count: columns.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => width + GAP,
    overscan: 3,
  });

  const scrollTo = (index: number) => {
    const ci = colOf.get(index);
    if (ci !== undefined) virt.scrollToIndex(ci, { align: "auto" });
  };
  const sel = useSelection(items, scrollTo, { textOf: isDateKey(sortKey) ? undefined : (c) => sortText(c, sortKey, nameOrder) ?? "", desc: sortDesc });
  const avgPerCol = columns.length ? Math.max(1, Math.round(items.length / columns.length)) : 1;

  // "Abramson – Cohen" range in the title bar
  const vitems = virt.getVirtualItems();
  const firstCol = vitems.find((v) => v.end > virt.scrollOffset! + 4);
  const lastCol = [...vitems].reverse().find((v) => v.start < (virt.scrollOffset ?? 0) + (scrollRef.current?.clientWidth ?? 0) - 4);
  const rangeText =
    firstCol && lastCol && columns[firstCol.index] && columns[lastCol.index]
      ? [items[columns[firstCol.index][0]], items[columns[lastCol.index][columns[lastCol.index].length - 1]]].map((c) => (c ? rangeOf(c, sortKey, nameOrder) : "")).join(" – ")
      : "";
  useEffect(() => onRange?.(rangeText), [rangeText, onRange]);

  // index tabs jump by the active sort key's first letter (company name for Company; none for dates)
  const keys = useMemo(() => items.map((c) => indexKeyFor(c, sortKey, nameOrder)), [items, sortKey, nameOrder]);
  const present = useMemo(() => new Set(keys.filter((k): k is string => k !== null)), [keys]);
  const showTabs = !isDateKey(sortKey);

  return (
    <div className="cm-contacts-cardwrap">
      <div
        ref={scrollRef}
        className={`cm-contacts-cards ${kind}`}
        tabIndex={0}
        onKeyDown={(e) => sel.onKeyDown(e, { horizontalStep: avgPerCol, pageStep: avgPerCol })}
        onContextMenu={onViewBackgroundContextMenu}
      >
        <div style={{ width: virt.getTotalSize() + GAP, height: "100%", position: "relative" }}>
          {vitems.map((v) => (
            <div key={v.key} className="cm-contacts-col" style={{ position: "absolute", top: 0, left: v.start + GAP, width, height: "100%" }}>
              {kind !== "business" && <div className="cm-contacts-colrule" style={{ left: width + GAP / 2 - 1 }} />}
              {columns[v.index].map((i) => {
                const c = items[i];
                const selected = sel.selSet.has(c.id);
                const handlers = {
                  onMouseDown: (e: React.MouseEvent) => sel.onItemMouseDown(i, e),
                  onDoubleClick: () => openContact(c),
                  onContextMenu: (e: React.MouseEvent) => sel.onItemContextMenu(i, e),
                };
                return kind === "business" ? (
                  <BusinessCard key={c.id} contact={c} selected={selected} focused={i === sel.focusIdx} {...handlers} />
                ) : (
                  <AddressCard key={c.id} contact={c} kind={kind} title={shownFileAs(c, nameOrder)} selected={selected} {...handlers} />
                );
              })}
            </div>
          ))}
        </div>
      </div>
      {showTabs && (
        <IndexTabs
          present={present}
          onJump={(k) => {
            const i = jumpToKey(keys, k, sortDesc);
            if (i >= 0) sel.moveTo(i, false);
            const ci = colOf.get(i);
            // Outlook brings the letter to the left edge
            if (ci !== undefined) requestAnimationFrame(() => virt.scrollToIndex(ci, { align: "start" }));
            scrollRef.current?.focus();
          }}
        />
      )}
    </div>
  );
}

/** First contact at or after index tab `key` in display order (before it, when descending). */
export function jumpToKey(keys: (string | null)[], key: string, desc: boolean): number {
  const order = INDEX_ORDER.indexOf(key);
  let i = keys.findIndex((k) => k !== null && (desc ? INDEX_ORDER.indexOf(k) <= order : INDEX_ORDER.indexOf(k) >= order));
  if (i < 0) {
    // past the end: the last contact that has a key
    for (i = keys.length - 1; i >= 0 && keys[i] === null; i--);
  }
  return i;
}

function AddressCard({
  contact: c,
  kind,
  title,
  selected,
  onMouseDown,
  onDoubleClick,
  onContextMenu,
}: {
  contact: Contact;
  kind: CardKind;
  title: string;
  selected: boolean;
  onMouseDown: (e: React.MouseEvent) => void;
  onDoubleClick: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
}) {
  const fields = cardFields(c, kind);
  return (
    <div
      className={`cm-contacts-acard${selected ? " selected" : ""}`}
      data-id={c.id}
      draggable
      onMouseDown={onMouseDown}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      <div className="cm-contacts-acard-head cm-ellipsis" dir="auto">
        {title || "(no name)"}
      </div>
      <div className="cm-contacts-acard-body">
        {fields.map((f, i) =>
          f.label === undefined ? (
            <div key={i} className="cm-contacts-acard-line cm-ellipsis" dir="auto">
              {f.value}
            </div>
          ) : (
            <div key={i} className="cm-contacts-acard-line labelled">
              <span className="cm-contacts-acard-label">{f.label}</span>
              <span className="cm-contacts-acard-value cm-ellipsis" dir="auto">
                {f.value}
              </span>
            </div>
          ),
        )}
      </div>
    </div>
  );
}

const LETTERS = INDEX_ORDER.slice(1, 27);

/** The index tabs down the right edge: "123 ab cd ef ..." (paired when the pane is short). */
function IndexTabs({ present, onJump }: { present: Set<string>; onJump: (key: string) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [h, setH] = useState(500);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setH(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const hebrew = present.has("א");
  const slots = Math.floor(h / 17);
  let tabs: { label: string; key: string }[];
  if (slots >= 28 + (hebrew ? 1 : 0)) tabs = LETTERS.map((l) => ({ label: l, key: l }));
  else if (slots >= 15) {
    tabs = [];
    for (let i = 0; i < LETTERS.length; i += 2) tabs.push({ label: LETTERS.slice(i, i + 2).join(""), key: LETTERS[i] });
  } else {
    tabs = [];
    for (let i = 0; i < LETTERS.length; i += 4) tabs.push({ label: `${LETTERS[i]}-${LETTERS[Math.min(i + 3, 25)]}`, key: LETTERS[i] });
  }
  tabs = [{ label: "123", key: "123" }, ...tabs];
  if (hebrew) tabs.push({ label: "א", key: "א" });
  return (
    <div ref={ref} className="cm-contacts-index" role="tablist" aria-label="Index">
      {tabs.map((t) => (
        <button key={t.label} className="cm-contacts-index-tab" onMouseDown={(e) => e.preventDefault()} onClick={() => onJump(t.key)} title={`Go to ${t.label}`}>
          {t.label}
        </button>
      ))}
    </div>
  );
}
