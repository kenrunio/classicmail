// The tabs above a message's text once View Source is used (or a Markdown message is written):
// Message / HTML Source (/ Original Message), or Markdown / Preview, to flip between them.
import type { ReactNode } from "react";

export interface BodyTab {
  key: string;
  label: string;
}

export function BodyTabs({
  tabs,
  active,
  onPick,
  onClose,
  children,
}: {
  tabs: BodyTab[];
  active: string;
  onPick: (key: string) => void;
  /** closes the source tabs (none: they stay, as for a Markdown message) */
  onClose?: () => void;
  /** buttons at the right (Copy, Save As…) */
  children?: ReactNode;
}) {
  return (
    <div className="cm-body-tabs" role="tablist">
      {tabs.map((t) => (
        <button key={t.key} role="tab" aria-selected={t.key === active} className={t.key === active ? "on" : ""} onClick={() => onPick(t.key)}>
          {t.label}
        </button>
      ))}
      <span className="cm-fill" />
      {children}
      {onClose && (
        <button className="cm-body-tabs-close" title="Close the source" onClick={onClose}>
          ×
        </button>
      )}
    </div>
  );
}
