// Navigation Pane: module title, module-specific content, the big Mail / Calendar / Contacts /
// Tasks buttons at the bottom and the small-icon row. Right-click a module's button › Open in New
// Window: another main window on that module (e.g. Calendar on a second screen).
import { Icon, contextMenu } from "../ui";
import { openWindow } from "../api/windows";
import { useShell } from "./store";
import type { ModuleDef } from "./types";
import { usePersistentState, Splitter } from "../ui";
import { useRef } from "react";

/** A module button's right-click menu. */
const moduleMenu = (m: ModuleDef, open: () => void) =>
  contextMenu([
    { label: "Open", onClick: open },
    { label: "Open in New Window", onClick: () => void openWindow("explorer", { module: m.id }, { title: `${m.label} - ClassicMail` }) },
  ]);

/** height of one big module button (px) — dragging the splitter this far adds / removes one */
const BUTTON_H = 32;

export function NavigationPane({ module, modules }: { module: ModuleDef; modules: ModuleDef[] }) {
  // only what it shows (not the pane widths, which change on every move of a splitter drag)
  const navCollapsed = useShell((s) => s.navCollapsed);
  const set = useShell((s) => s.set);
  const setModule = useShell((s) => s.setModule);
  // how many big buttons are shown (dragging the splitter above them changes it, like Outlook)
  const [bigCount, setBigCount] = usePersistentState("navBigButtons", 4);
  const drag = useRef(0);

  if (navCollapsed) {
    return (
      <div className="cm-navpane collapsed">
        <button className="cm-navpane-expand" title="Expand the Navigation Pane" onClick={() => set({ navCollapsed: false })}>
          <Icon name="chevron-right-double" size={12} />
        </button>
        <div className="cm-navpane-vertical-title">{module.label}</div>
        <div className="cm-fill" />
        {modules.map((m) => (
          <button
            key={m.id}
            className={`cm-navpane-mini${m.id === module.id ? " selected" : ""}`}
            title={m.label}
            onClick={() => setModule(m.id)}
            onContextMenu={moduleMenu(m, () => setModule(m.id))}
          >
            <Icon name={m.icon} />
          </button>
        ))}
      </div>
    );
  }

  const big = modules.slice(0, bigCount);
  const small = modules.slice(bigCount);
  return (
    <div className="cm-navpane">
      <div className="cm-navpane-title">
        <span className="cm-fill">{module.label}</span>
        <button className="cm-navpane-collapse" title="Minimize the Navigation Pane" onClick={() => set({ navCollapsed: true })}>
          <Icon name="chevron-left-double" size={12} />
        </button>
      </div>
      <div className="cm-navpane-content">
        <module.NavPane />
      </div>
      <Splitter
        direction="horizontal"
        onResize={(d) => {
          // the splitter reports small per-move deltas: add them up, one button per button height
          drag.current += d;
          while (drag.current <= -BUTTON_H) {
            drag.current += BUTTON_H;
            setBigCount((n) => Math.min(modules.length, n + 1));
          }
          while (drag.current >= BUTTON_H) {
            drag.current -= BUTTON_H;
            setBigCount((n) => Math.max(0, n - 1));
          }
        }}
        onResizeEnd={() => (drag.current = 0)}
      />
      <div className="cm-navpane-buttons">
        {big.map((m, i) => (
          <button
            key={m.id}
            className={`cm-navpane-btn${m.id === module.id ? " selected" : ""}`}
            onClick={() => setModule(m.id)}
            onContextMenu={moduleMenu(m, () => setModule(m.id))}
            title={`${m.label} (Ctrl+${i + 1})`}
          >
            <Icon name={m.icon} size={24} />
            <span>{m.label}</span>
          </button>
        ))}
        <div className="cm-navpane-smallrow">
          {small.map((m) => (
            <button
              key={m.id}
              className={`cm-navpane-small${m.id === module.id ? " selected" : ""}`}
              title={m.label}
              onClick={() => setModule(m.id)}
              onContextMenu={moduleMenu(m, () => setModule(m.id))}
            >
              <Icon name={m.icon} />
            </button>
          ))}
          <div className="cm-fill" />
          <button className="cm-navpane-small" title="Show more buttons" onClick={() => setBigCount(modules.length)}>
            <Icon name="chevron-down" size={10} />
          </button>
        </div>
      </div>
    </div>
  );
}
