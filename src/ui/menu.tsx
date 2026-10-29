// Office 2007 menus: menu bar, drop-down and context menus (with submenus + keyboard).
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { create } from "zustand";
import { Icon } from "./icons";
import { KeyLabel, accessKeys, keyOf } from "./keys";

export type MenuItem =
  | {
      type?: "item";
      label: string;
      icon?: string;
      shortcut?: string;
      disabled?: boolean;
      checked?: boolean;
      /** radio-style dot instead of a check */
      radio?: boolean;
      bold?: boolean;
      onClick?: () => void;
      submenu?: MenuItem[];
    }
  | { type: "separator" }
  | { type: "header"; label: string };

type MenuOpts = {
  onClose?: () => void;
  minWidth?: number;
  /** opened from the keyboard: the first item starts highlighted */
  keyboard?: boolean;
  /** Left / Right past the menu's edge: the menu bar's next menu */
  nav?: (dir: -1 | 1) => void;
};
type OpenMenu = MenuOpts & { x: number; y: number; items: MenuItem[]; key: number };

const useMenuStore = create<{ open: OpenMenu | null; set: (m: OpenMenu | null) => void }>((set) => ({
  open: null,
  set: (open) => set({ open }),
}));

let menuKey = 0;

/** Show a popup menu at a screen point (context menus, drop-down buttons). */
export function showMenu(x: number, y: number, items: MenuItem[], opts: MenuOpts = {}) {
  const prev = useMenuStore.getState().open;
  prev?.onClose?.();
  useMenuStore.getState().set({ x, y, items, key: ++menuKey, ...opts });
}

export function closeMenu() {
  const prev = useMenuStore.getState().open;
  useMenuStore.getState().set(null);
  prev?.onClose?.();
}

/** onContextMenu handler factory. */
export function contextMenu(items: MenuItem[] | (() => MenuItem[])) {
  return (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    showMenu(e.clientX, e.clientY, typeof items === "function" ? items() : items);
  };
}

/** Mount once per window (WindowRoot does this). */
export function MenuHost() {
  const open = useMenuStore((s) => s.open);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest?.(".cm-menu")) closeMenu();
    };
    const onBlur = () => closeMenu();
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("blur", onBlur);
    };
  }, [open]);
  if (!open) return null;
  return createPortal(<PopupMenu key={open.key} x={open.x} y={open.y} items={open.items} minWidth={open.minWidth} keyboard={open.keyboard} nav={open.nav} />, document.body);
}

function PopupMenu({
  x,
  y,
  items,
  minWidth,
  keyboard,
  nav,
  onLeaveLeft,
}: {
  x: number;
  y: number;
  items: MenuItem[];
  minWidth?: number;
  keyboard?: boolean;
  nav?: (dir: -1 | 1) => void;
  onLeaveLeft?: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });
  const selectable = (i: number) => {
    const it = items[i];
    return it && it.type !== "separator" && it.type !== "header" && !it.disabled;
  };
  const [active, setActive] = useState(() => (keyboard ? items.findIndex((_, i) => selectable(i)) : -1));
  // each item's access key (the underlined letter), as in Windows menus
  const keys = accessKeys(items.map((it) => (it.type === "separator" || it.type === "header" ? "" : it.label)));
  const [subOpen, setSubOpen] = useState(-1);
  // the submenu was opened from the keyboard (its first item starts highlighted)
  const [subByKey, setSubByKey] = useState(false);
  const openSub = (i: number) => {
    setSubByKey(true);
    setSubOpen(i);
  };
  const timer = useRef<number | undefined>(undefined);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    let nx = x;
    let ny = y;
    if (nx + r.width > window.innerWidth - 2) nx = Math.max(2, window.innerWidth - r.width - 2);
    if (ny + r.height > window.innerHeight - 2) ny = Math.max(2, window.innerHeight - r.height - 2);
    setPos({ x: nx, y: ny });
  }, [x, y]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // only the innermost open menu handles keys
      if (subOpen !== -1) return;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const dir = e.key === "ArrowDown" ? 1 : -1;
        let i = active;
        for (let n = 0; n < items.length; n++) {
          i = (i + dir + items.length) % items.length;
          if (selectable(i)) break;
        }
        setActive(i);
      } else if (e.key === "Enter" || e.key === "ArrowRight") {
        const it = items[active];
        if (e.key === "ArrowRight" && nav && !(it && it.type !== "separator" && it.type !== "header" && it.submenu)) {
          e.preventDefault();
          nav(1);
        } else if (it && it.type !== "separator" && it.type !== "header" && !it.disabled) {
          e.preventDefault();
          if (it.submenu) openSub(active);
          else if (e.key === "Enter") {
            closeMenu();
            it.onClick?.();
          }
        }
      } else if (e.key === "ArrowLeft" && (onLeaveLeft || nav)) {
        e.preventDefault();
        if (onLeaveLeft) onLeaveLeft();
        else nav?.(-1);
      } else if (e.key === "Escape") {
        e.preventDefault();
        if (onLeaveLeft) onLeaveLeft();
        else closeMenu();
      } else if (e.key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) {
        // the item's letter chooses it (Alt+letter is left to the menu bar)
        const i = keys.indexOf(keyOf(e));
        if (i < 0) return;
        e.preventDefault();
        const it = items[i];
        if (!selectable(i) || it.type === "separator" || it.type === "header") return;
        setActive(i);
        if (it.submenu) openSub(i);
        else {
          closeMenu();
          it.onClick?.();
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  return (
    <div ref={ref} className="cm-menu" style={{ left: pos.x, top: pos.y, minWidth }} onContextMenu={(e) => e.preventDefault()}>
      <div className="cm-menu-gutter" />
      {items.map((it, i) => {
        if (it.type === "separator") return <div key={i} className="cm-menu-sep" />;
        if (it.type === "header")
          return (
            <div key={i} className="cm-menu-header">
              {it.label}
            </div>
          );
        return (
          <div
            key={i}
            className={`cm-menu-item${it.disabled ? " disabled" : ""}${active === i ? " active" : ""}${it.bold ? " bold" : ""}`}
            onMouseEnter={() => {
              setActive(i);
              window.clearTimeout(timer.current);
              timer.current = window.setTimeout(() => {
                setSubByKey(false);
                setSubOpen(it.submenu && !it.disabled ? i : -1);
              }, 250);
            }}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              if (it.disabled) return;
              if (it.submenu) {
                setSubByKey(false);
                setSubOpen(i);
                return;
              }
              closeMenu();
              it.onClick?.();
            }}
          >
            <span className="cm-menu-icon">
              {it.checked ? (
                <span className={`cm-menu-check${it.radio ? " radio" : ""}`}>{it.radio ? <Icon name="dot" size={14} /> : <Icon name="check" size={12} />}</span>
              ) : it.icon ? (
                <Icon name={it.icon} />
              ) : null}
            </span>
            <span className="cm-menu-label">
              <KeyLabel label={it.label} k={keys[i]} />
            </span>
            <span className="cm-menu-shortcut">{it.shortcut}</span>
            <span className="cm-menu-arrow">{it.submenu ? "▸" : ""}</span>
            {subOpen === i && it.submenu && <SubMenu parent={ref} index={i} items={it.submenu} keyboard={subByKey} onClose={() => setSubOpen(-1)} />}
          </div>
        );
      })}
    </div>
  );
}

function SubMenu({
  parent,
  index,
  items,
  keyboard,
  onClose,
}: {
  parent: React.RefObject<HTMLDivElement | null>;
  index: number;
  items: MenuItem[];
  keyboard: boolean;
  onClose: () => void;
}) {
  const p = parent.current;
  if (!p) return null;
  const row = p.children[index + 1] as HTMLElement | undefined; // +1 for the gutter div
  const pr = p.getBoundingClientRect();
  const rr = row?.getBoundingClientRect();
  return createPortal(<PopupMenu x={pr.right - 3} y={(rr?.top ?? pr.top) - 3} items={items} keyboard={keyboard} onLeaveLeft={onClose} />, document.body);
}

// ---- menu bar --------------------------------------------------------------------------------

export interface MenuBarMenu {
  /** "&File" — the letter after & is the Alt accelerator */
  label: string;
  items: MenuItem[] | (() => MenuItem[]);
}

function renderAccel(label: string): ReactNode {
  const i = label.indexOf("&");
  if (i < 0) return label;
  return (
    <>
      {label.slice(0, i)}
      <u>{label[i + 1]}</u>
      {label.slice(i + 2)}
    </>
  );
}

export function MenuBar({ menus, right }: { menus: MenuBarMenu[]; right?: ReactNode }) {
  const [openIdx, setOpenIdx] = useState(-1);
  const refs = useRef<(HTMLDivElement | null)[]>([]);
  const open = (i: number, keyboard = false) => {
    const el = refs.current[i];
    if (!el) return;
    const r = el.getBoundingClientRect();
    const m = menus[i];
    // showMenu first: it calls the previous menu's onClose (setOpenIdx(-1)), which must not win
    showMenu(r.left, r.bottom, typeof m.items === "function" ? m.items() : m.items, {
      onClose: () => setOpenIdx(-1),
      minWidth: 180,
      keyboard,
      // Left / Right go on to the next menu along the bar
      nav: (dir) => open((i + dir + menus.length) % menus.length, true),
    });
    setOpenIdx(i);
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // F10 opens the first menu, as in Windows
      if (e.key === "F10" && !e.altKey && !e.ctrlKey && !e.shiftKey) {
        if (document.querySelector(".cm-dialog-overlay")) return;
        e.preventDefault();
        if (openIdx >= 0) closeMenu();
        else open(0, true);
        return;
      }
      if (!e.altKey || e.ctrlKey || e.key.length !== 1) return;
      const k = keyOf(e);
      const idx = menus.findIndex((m) => {
        const i = m.label.indexOf("&");
        return i >= 0 && m.label[i + 1].toLowerCase() === k;
      });
      if (idx >= 0) {
        e.preventDefault();
        open(idx, true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
  return (
    <div className="cm-menubar">
      {menus.map((m, i) => (
        <div
          key={i}
          ref={(el) => {
            refs.current[i] = el;
          }}
          className={`cm-menubar-item${openIdx === i ? " open" : ""}`}
          onMouseDown={(e) => {
            e.preventDefault();
            if (openIdx === i) closeMenu();
            else open(i);
          }}
          onMouseEnter={() => {
            if (openIdx >= 0 && openIdx !== i) open(i);
          }}
        >
          {renderAccel(m.label)}
        </div>
      ))}
      <div className="cm-fill" />
      {right}
    </div>
  );
}
