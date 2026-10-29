// The Outlook main window: menu bar, Standard toolbar, Navigation Pane | module view | To-Do Bar,
// status bar.
import { useEffect, useMemo } from "react";
import { useRpc } from "../api/query";
import { useCmEvent } from "../api/events";
import { rpc } from "../api/rpc";
import type { Account, SyncStatus } from "../api/types";
import { MenuBar, Splitter, clamp, useHotkeys, Icon } from "../ui";
import { TodoBar } from "../modules/tasks/TodoBar";
import { moduleById, MODULES } from "./registry";
import { isExplorer, useShell, useStatus } from "./store";
import { NavigationPane } from "./NavigationPane";
import { StatusBar } from "./StatusBar";
import { buildMainMenus } from "./mainMenus";
import { FirstRun } from "./FirstRun";
import type { ModuleDef } from "./types";
import { startDesktopServices } from "./services";
import { showShortcuts } from "./help";
import { newAppointment, newContact, newMail, newMeeting, newTask } from "./newItems";
import { openWindow, setWindowTitle } from "../api/windows";

export default function MainWindow() {
  // field by field: a splitter drag changes navWidth / todoWidth on every mouse move
  const mod = moduleById(useShell((s) => s.module));
  const navCollapsed = useShell((s) => s.navCollapsed);
  const navWidth = useShell((s) => s.navWidth);
  const todoVisible = useShell((s) => s.todoVisible);
  const todoMinimized = useShell((s) => s.todoMinimized);
  const todoWidth = useShell((s) => s.todoWidth);
  const set = useShell((s) => s.set);
  const setModule = useShell((s) => s.setModule);
  const accounts = useRpc<Account[]>(["accounts"], "accounts.list");
  const setSync = useStatus((s) => s.setSync);
  useCmEvent("sync.status", (e) => setSync(e as unknown as SyncStatus));
  // statuses sent before this window was listening (else the status bar says "Connecting…" until the next poll)
  useEffect(() => {
    rpc<SyncStatus[]>("sync.status", {})
      .then((list) => list.forEach((s) => useStatus.getState().sync[s.accountId] || setSync(s)))
      .catch(() => {});
  }, [setSync]);

  useEffect(() => {
    void setWindowTitle(`${mod.label} - ClassicMail${import.meta.env.DEV ? (import.meta.env.VITE_CM_PREVIEW === "1" ? " [PREVIEW]" : " [TEST BUILD]") : ""}`);
  }, [mod.label]);

  // tray, desktop alerts, reminders and messages from other programs: once, in the main window
  useEffect(() => (isExplorer ? undefined : startDesktopServices()), []);

  useHotkeys({
    "ctrl+1": () => setModule("mail"),
    "ctrl+2": () => setModule("calendar"),
    "ctrl+3": () => setModule("contacts"),
    "ctrl+4": () => setModule("tasks"),
    "ctrl+n": () => mod.newItem(),
    f9: () => rpc("sync.now", {}),
    "ctrl+m": () => rpc("sync.now", {}),
    "alt+f1": () => set({ navCollapsed: !navCollapsed }),
    "alt+f2": () => set(todoVisible && !todoMinimized ? { todoVisible: false } : { todoVisible: true, todoMinimized: false }),
    // New items from anywhere (the Calendar / Contacts views handle their own variants)
    "ctrl+shift+m": () => newMail(),
    "ctrl+shift+a": () => mod.id !== "calendar" && newAppointment(),
    "ctrl+shift+q": () => mod.id !== "calendar" && newMeeting(),
    "ctrl+shift+c": () => mod.id !== "contacts" && newContact(),
    "ctrl+shift+k": () => (mod.id === "tasks" ? mod.newItem() : newTask()),
    "ctrl+shift+b": () => mod.id !== "contacts" && openWindow("address-book", {}, { label: "address-book", title: "Address Book" }),
  });

  // the same elements while only the pane widths change, so React skips their subtrees during a drag
  const navPane = useMemo(() => <NavigationPane module={mod} modules={MODULES} />, [mod]);
  const view = useMemo(() => <mod.View />, [mod]);
  const todoBar = useMemo(() => <TodoBar onClose={() => useShell.getState().set({ todoMinimized: true })} />, []);

  if (accounts.data && accounts.data.length === 0) return <FirstRun />;

  const navW = navCollapsed ? 28 : navWidth;
  return (
    <div className="cm-main">
      {/* keyed by module: each module's useMenus hook lives in its own component instance */}
      <ModuleMenuBar key={mod.id} mod={mod} />
      <div className="cm-toolbar-row">
        <mod.Toolbar />
      </div>
      <div className="cm-main-body">
        <div className="cm-navpane-wrap" style={{ width: navW }}>
          {navPane}
        </div>
        {!navCollapsed && <Splitter direction="vertical" onResize={(d) => set({ navWidth: clamp(useShell.getState().navWidth + d, 150, 480) })} />}
        <div className="cm-module-view">{view}</div>
        {todoVisible && todoMinimized && (
          <div className="cm-todobar-min">
            <button className="cm-navpane-expand" title="Expand the To-Do Bar (Alt+F2)" onClick={() => set({ todoMinimized: false })}>
              <Icon name="chevron-left-double" size={12} />
            </button>
            <div className="cm-navpane-vertical-title" onClick={() => set({ todoMinimized: false })}>
              To-Do Bar
            </div>
          </div>
        )}
        {todoVisible && !todoMinimized && (
          <>
            <Splitter direction="vertical" onResize={(d) => set({ todoWidth: clamp(useShell.getState().todoWidth - d, 160, 420) })} />
            <div className="cm-todobar-wrap" style={{ width: todoWidth }}>
              {todoBar}
            </div>
          </>
        )}
      </div>
      <StatusBar />
    </div>
  );
}

function ModuleMenuBar({ mod }: { mod: ModuleDef }) {
  const menus = mod.useMenus?.() ?? {};
  return <MenuBar menus={buildMainMenus(mod, menus)} right={<HelpBox />} />;
}

function HelpBox() {
  return (
    <div className="cm-helpbox" title="Type a question for help">
      <input
        className="cm-input"
        placeholder="Type a question for help"
        style={{ width: 170, height: 18, fontSize: 11 }}
        onKeyDown={(e) => {
          if (e.key !== "Enter") return;
          // keep this Enter from also "clicking" the OK button of the box that opens
          e.preventDefault();
          const q = e.currentTarget.value.trim();
          e.currentTarget.value = "";
          e.currentTarget.blur();
          setTimeout(() => showShortcuts(q || undefined), 0);
        }}
      />
      <Icon name="chevron-down" size={10} />
    </div>
  );
}
