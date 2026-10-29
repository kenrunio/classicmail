// Small building blocks for the compose ribbon: buttons that take local or shared icons.
import type { ReactNode } from "react";
import { showMenu, type MenuItem } from "../../../ui";
import { CIcon } from "./icons";

function openMenu(e: React.MouseEvent, menu: MenuItem[] | (() => MenuItem[])) {
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
  showMenu(r.left, r.bottom, typeof menu === "function" ? menu() : menu, { minWidth: 180 });
}

export function Big({
  icon,
  label,
  onClick,
  disabled,
  pressed,
  title,
  menu,
}: {
  icon: string;
  label: ReactNode;
  onClick?: (e: React.MouseEvent) => void;
  disabled?: boolean;
  pressed?: boolean;
  title?: string;
  menu?: MenuItem[] | (() => MenuItem[]);
}) {
  return (
    <button
      className={`cm-rb-big${pressed ? " pressed" : ""}`}
      disabled={disabled}
      title={title ?? (typeof label === "string" ? label : undefined)}
      onMouseDown={(e) => e.preventDefault()}
      onClick={(e) => (menu ? openMenu(e, menu) : onClick?.(e))}
    >
      <CIcon name={icon} size={32} />
      <span>
        {label}
        {menu ? " ▾" : ""}
      </span>
    </button>
  );
}

export function Small({
  icon,
  label,
  onClick,
  disabled,
  pressed,
  title,
  menu,
}: {
  icon: string;
  label?: ReactNode;
  onClick?: (e: React.MouseEvent) => void;
  disabled?: boolean;
  pressed?: boolean;
  title?: string;
  menu?: MenuItem[] | (() => MenuItem[]);
}) {
  return (
    <button
      className={`cm-rb-small${pressed ? " pressed" : ""}`}
      disabled={disabled}
      title={title ?? (typeof label === "string" ? label : undefined)}
      onMouseDown={(e) => e.preventDefault()}
      onClick={(e) => (menu ? openMenu(e, menu) : onClick?.(e))}
    >
      <CIcon name={icon} />
      {label && <span>{label}</span>}
      {menu && <span style={{ fontSize: 9 }}>▾</span>}
    </button>
  );
}

/** Split button: icon applies the last value, ▾ opens the picker (font colour, highlight). */
export function SmallSplit({
  icon,
  title,
  onClick,
  onArrow,
  disabled,
  swatch,
}: {
  icon: string;
  title: string;
  /** gets the button's box, for a picker opened from the icon too */
  onClick: (r: DOMRect) => void;
  onArrow: (r: DOMRect) => void;
  disabled?: boolean;
  swatch?: string | null;
}) {
  return (
    <span className="cm-compose-split">
      <button
        className="cm-rb-small"
        title={title}
        disabled={disabled}
        onMouseDown={(e) => e.preventDefault()}
        onClick={(e) => onClick((e.currentTarget.parentElement as HTMLElement).getBoundingClientRect())}
      >
        <span className="cm-compose-swatched">
          <CIcon name={icon} />
          {swatch !== undefined && <span className="cm-compose-swatchbar" style={{ background: swatch ?? "transparent" }} />}
        </span>
      </button>
      <button
        className="cm-rb-small cm-compose-split-arrow"
        title={title}
        disabled={disabled}
        onMouseDown={(e) => e.preventDefault()}
        onClick={(e) => onArrow((e.currentTarget.parentElement as HTMLElement).getBoundingClientRect())}
      >
        <span style={{ fontSize: 9 }}>▾</span>
      </button>
    </span>
  );
}

export function Row({ children }: { children: ReactNode }) {
  return <div className="cm-rb-row">{children}</div>;
}

export function Stack({ children }: { children: ReactNode }) {
  return <div className="cm-rb-stack">{children}</div>;
}

export function Check({ label, checked, onChange, disabled }: { label: string; checked: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <label className="cm-compose-rbcheck" onMouseDown={(e) => e.preventDefault()}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      {label}
    </label>
  );
}
