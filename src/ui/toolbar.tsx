// Office 2007 command bars (main window "Standard" toolbar and module toolbars).
import type { ReactNode } from "react";
import { Icon } from "./icons";
import { showMenu, type MenuItem } from "./menu";

export function Toolbar({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={`cm-toolbar ${className ?? ""}`}>{children}</div>;
}

export function TbSep() {
  return <div className="cm-tb-sep" />;
}

export interface TbButtonProps {
  icon?: string;
  label?: string;
  title?: string;
  onClick?: () => void;
  disabled?: boolean;
  pressed?: boolean;
  /** label under a 32px icon (ribbon-ish big button) */
  big?: boolean;
}

export function TbButton({ icon, label, title, onClick, disabled, pressed }: TbButtonProps) {
  return (
    <button className={`cm-tb-btn${pressed ? " pressed" : ""}`} title={title ?? label} disabled={disabled} onMouseDown={(e) => e.preventDefault()} onClick={onClick}>
      {icon && <Icon name={icon} />}
      {label && <span>{label}</span>}
    </button>
  );
}

function openBelow(e: React.MouseEvent, items: MenuItem[] | (() => MenuItem[])) {
  const r = (e.currentTarget as HTMLElement).closest(".cm-tb-split, .cm-tb-btn")!.getBoundingClientRect();
  showMenu(r.left, r.bottom, typeof items === "function" ? items() : items, { minWidth: 160 });
}

/** Button with a default action plus a ▾ drop-down (e.g. "New ▾", "Send/Receive ▾"). */
export function TbSplit({ icon, label, title, onClick, disabled, menu }: TbButtonProps & { menu: MenuItem[] | (() => MenuItem[]) }) {
  return (
    <div className={`cm-tb-split${disabled ? " disabled" : ""}`}>
      <button className="cm-tb-btn main" title={title ?? label} disabled={disabled} onMouseDown={(e) => e.preventDefault()} onClick={onClick}>
        {icon && <Icon name={icon} />}
        {label && <span>{label}</span>}
      </button>
      <button className="cm-tb-btn arrow" disabled={disabled} onMouseDown={(e) => e.preventDefault()} onClick={(e) => openBelow(e, menu)}>
        <Icon name="chevron-down" size={10} />
      </button>
    </div>
  );
}

/** Button that only opens a menu. */
export function TbDropdown({ icon, label, title, disabled, menu }: TbButtonProps & { menu: MenuItem[] | (() => MenuItem[]) }) {
  return (
    <button className="cm-tb-btn" title={title ?? label} disabled={disabled} onMouseDown={(e) => e.preventDefault()} onClick={(e) => openBelow(e, menu)}>
      {icon && <Icon name={icon} />}
      {label && <span>{label}</span>}
      <Icon name="chevron-down" size={10} />
    </button>
  );
}
