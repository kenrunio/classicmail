// Office 2007 Ribbon, used by item windows (compose, appointment, contact, task, message).
import { useState, type ReactNode } from "react";
import { Icon } from "./icons";
import { showMenu, type MenuItem } from "./menu";

export interface RibbonTab {
  id: string;
  label: string;
  groups: { label: string; content: ReactNode }[];
}

export function Ribbon({ tabs, orbMenu, quickAccess }: { tabs: RibbonTab[]; orbMenu?: MenuItem[]; quickAccess?: ReactNode }) {
  const [active, setActive] = useState(tabs[0]?.id);
  const tab = tabs.find((t) => t.id === active) ?? tabs[0];
  return (
    <div className="cm-ribbon">
      <div className="cm-ribbon-tabs">
        <button
          className="cm-ribbon-orb"
          title="Menu"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            if (orbMenu) showMenu(r.left, r.bottom, orbMenu, { minWidth: 220 });
          }}
        >
          <Icon name="mail" size={18} />
        </button>
        {quickAccess && <div className="cm-ribbon-qat">{quickAccess}</div>}
        {tabs.map((t) => (
          <div key={t.id} className={`cm-ribbon-tab${t.id === tab?.id ? " active" : ""}`} onMouseDown={() => setActive(t.id)}>
            {t.label}
          </div>
        ))}
      </div>
      <div className="cm-ribbon-body">
        {tab?.groups.map((g, i) => (
          <div key={i} className="cm-ribbon-group">
            <div className="cm-ribbon-group-content">{g.content}</div>
            <div className="cm-ribbon-group-label">{g.label}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

export function RibbonBig({
  icon,
  label,
  onClick,
  disabled,
  menu,
  pressed,
  title,
}: {
  icon: string;
  label: string;
  onClick?: () => void;
  disabled?: boolean;
  menu?: MenuItem[] | (() => MenuItem[]);
  pressed?: boolean;
  title?: string;
}) {
  return (
    <button
      className={`cm-rb-big${pressed ? " pressed" : ""}`}
      disabled={disabled}
      title={title ?? label}
      onMouseDown={(e) => e.preventDefault()}
      onClick={(e) => {
        if (menu) {
          const r = e.currentTarget.getBoundingClientRect();
          showMenu(r.left, r.bottom, typeof menu === "function" ? menu() : menu);
        } else onClick?.();
      }}
    >
      <Icon name={icon} size={32} />
      <span>
        {label}
        {menu ? " ▾" : ""}
      </span>
    </button>
  );
}

export function RibbonSmall({
  icon,
  label,
  onClick,
  disabled,
  pressed,
  title,
  menu,
}: {
  icon: string;
  label?: string;
  onClick?: () => void;
  disabled?: boolean;
  pressed?: boolean;
  title?: string;
  menu?: MenuItem[] | (() => MenuItem[]);
}) {
  return (
    <button
      className={`cm-rb-small${pressed ? " pressed" : ""}`}
      disabled={disabled}
      title={title ?? label}
      onMouseDown={(e) => e.preventDefault()}
      onClick={(e) => {
        if (menu) {
          const r = e.currentTarget.getBoundingClientRect();
          showMenu(r.left, r.bottom, typeof menu === "function" ? menu() : menu);
        } else onClick?.();
      }}
    >
      <Icon name={icon} />
      {label && <span>{label}</span>}
      {menu && <span style={{ fontSize: 9 }}>▾</span>}
    </button>
  );
}

/** Stack of small buttons (up to 3 rows) inside a ribbon group. */
export function RibbonStack({ children }: { children: ReactNode }) {
  return <div className="cm-rb-stack">{children}</div>;
}

export function RibbonRow({ children }: { children: ReactNode }) {
  return <div className="cm-rb-row">{children}</div>;
}
