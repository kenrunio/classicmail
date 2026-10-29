// Main window menu bar: File Edit View Go Tools Actions Help. Modules contribute Edit / View /
// Actions items via ModuleDef.useMenus.
import type { MenuBarMenu, MenuItem } from "../ui";
import { rpc } from "../api/rpc";
import { openWindow } from "../api/windows";
import { fetchAppInfo } from "../api/query";
import { useShell, type ReadingPane } from "./store";
import type { ModuleDef, ModuleMenus } from "./types";
import { newItemMenu } from "./newItems";
import { showAbout, showPrivacyPolicy, showShortcuts } from "./help";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { redo, undo, undoState } from "../api/undo";
import { startMailMerge } from "../modules/mail/merge";
import { changeZoom, currentZoom } from "../ui/zoom";

/** Edit › Undo <last action> / Redo <action> (the app-wide history, src/api/undo.ts). */
export function undoMenuItems(): MenuItem[] {
  const s = undoState();
  return [
    { label: s.undoLabel ? `Undo ${s.undoLabel}` : "Can't Undo", icon: "undo", shortcut: "Ctrl+Z", disabled: !s.undoLabel, onClick: () => void undo() },
    { label: s.redoLabel ? `Redo ${s.redoLabel}` : "Can't Redo", icon: "redo", shortcut: "Ctrl+Y", disabled: !s.redoLabel, onClick: () => void redo() },
    { type: "separator" },
  ];
}

export function buildMainMenus(mod: ModuleDef, m: ModuleMenus): MenuBarMenu[] {
  const shell = useShell.getState();
  const rp = (p: ReadingPane): MenuItem => ({
    label: p === "off" ? "Off" : p === "right" ? "Right" : "Bottom",
    radio: true,
    checked: shell.readingPane === p,
    onClick: () => useShell.getState().set({ readingPane: p }),
  });
  return [
    {
      label: "&File",
      items: () => [
        { label: "New", icon: "new", submenu: newItemMenu() },
        {
          label: "Open",
          icon: "folder-open",
          submenu: [{ label: "Outlook Data File…", onClick: () => void import("../modules/archive/actions").then((a) => a.openArchiveDialog()) }],
        },
        { type: "separator" },
        { label: "Print…", icon: "print", shortcut: "Ctrl+P", onClick: () => window.dispatchEvent(new CustomEvent("cm-print")) },
        { type: "separator" },
        { label: "Account Settings…", icon: "accounts", onClick: () => openWindow("accounts", {}, { label: "accounts", title: "Account Settings" }) },
        { type: "separator" },
        { label: "Exit", onClick: () => rpc("app.exit", {}).catch(() => getCurrentWebviewWindow().close()) },
      ],
    },
    {
      label: "&Edit",
      items: () => [
        ...undoMenuItems(),
        // a module's own (placeholder) Undo / Redo items give way to the real ones
        ...(m.edit?.filter((i) => !("label" in i && /^(Undo|Redo)\b/.test(i.label ?? ""))) ?? [
          { label: "Cut", icon: "cut", shortcut: "Ctrl+X", onClick: () => document.execCommand("cut") },
          { label: "Copy", icon: "copy", shortcut: "Ctrl+C", onClick: () => document.execCommand("copy") },
          { label: "Paste", icon: "paste", shortcut: "Ctrl+V", onClick: () => document.execCommand("paste") },
        ]),
      ],
    },
    {
      label: "&View",
      items: () => [
        ...(m.view ?? []),
        ...(m.view?.length ? [{ type: "separator" } as MenuItem] : []),
        {
          label: "Navigation Pane",
          icon: "nav-pane",
          shortcut: "Alt+F1",
          checked: !useShell.getState().navCollapsed,
          onClick: () => useShell.getState().set({ navCollapsed: !useShell.getState().navCollapsed }),
        },
        ...(mod.id === "mail" ? [{ label: "Reading Pane", icon: "reading-pane", submenu: [rp("right"), rp("bottom"), rp("off")] } as MenuItem] : []),
        {
          label: "To-Do Bar",
          icon: "todo-bar",
          submenu: [
            {
              label: "Normal",
              shortcut: "Alt+F2",
              radio: true,
              checked: shell.todoVisible && !shell.todoMinimized,
              onClick: () => useShell.getState().set({ todoVisible: true, todoMinimized: false }),
            },
            {
              label: "Minimized",
              radio: true,
              checked: shell.todoVisible && shell.todoMinimized,
              onClick: () => useShell.getState().set({ todoVisible: true, todoMinimized: true }),
            },
            { label: "Off", radio: true, checked: !shell.todoVisible, onClick: () => useShell.getState().set({ todoVisible: false }) },
          ],
        },
        ...(mod.id === "mail"
          ? ([
              { type: "separator" },
              // message zoom (also Ctrl + mouse wheel over a message)
              { label: "Increase Font Size", shortcut: "Ctrl+=", disabled: currentZoom() >= 300, onClick: () => changeZoom(1) },
              { label: "Decrease Font Size", shortcut: "Ctrl+-", disabled: currentZoom() <= 50, onClick: () => changeZoom(-1) },
              { label: `Normal Size${currentZoom() === 100 ? "" : ` (now ${currentZoom()}%)`}`, shortcut: "Ctrl+0", disabled: currentZoom() === 100, onClick: () => changeZoom(0) },
            ] as MenuItem[])
          : []),
        { type: "separator" },
        { label: "Refresh", icon: "refresh", shortcut: "F9", onClick: () => rpc("sync.now", {}) },
      ],
    },
    {
      label: "&Go",
      items: () => [
        { label: "Mail", icon: "mail", shortcut: "Ctrl+1", checked: mod.id === "mail", radio: true, onClick: () => useShell.getState().setModule("mail") },
        { label: "Calendar", icon: "calendar", shortcut: "Ctrl+2", checked: mod.id === "calendar", radio: true, onClick: () => useShell.getState().setModule("calendar") },
        { label: "Contacts", icon: "contacts", shortcut: "Ctrl+3", checked: mod.id === "contacts", radio: true, onClick: () => useShell.getState().setModule("contacts") },
        { label: "Tasks", icon: "tasks", shortcut: "Ctrl+4", checked: mod.id === "tasks", radio: true, onClick: () => useShell.getState().setModule("tasks") },
      ],
    },
    {
      label: "&Tools",
      items: () => [
        {
          label: "Send/Receive",
          icon: "send-receive",
          submenu: [{ label: "Send/Receive All", icon: "send-receive", shortcut: "F9", onClick: () => rpc("sync.now", {}) }],
        },
        { label: "Instant Search", icon: "find", shortcut: "Ctrl+E", onClick: () => window.dispatchEvent(new CustomEvent("cm-focus-search")) },
        { label: "Address Book…", icon: "address-book", shortcut: "Ctrl+Shift+B", onClick: () => openWindow("address-book", {}, { label: "address-book", title: "Address Book" }) },
        { label: "Mail Merge…", icon: "mail-merge", onClick: () => startMailMerge() },
        { type: "separator" },
        { label: "Rules and Alerts…", icon: "rules", onClick: () => openWindow("rules", {}, { label: "rules", title: "Rules and Alerts" }) },
        { type: "separator" },
        { label: "Account Settings…", icon: "accounts", onClick: () => openWindow("accounts", {}, { label: "accounts", title: "Account Settings" }) },
        { label: "Options…", icon: "options", onClick: () => openWindow("options", {}, { label: "options", title: "Options" }) },
      ],
    },
    { label: "&Actions", items: () => m.actions ?? [{ label: "(nothing here)", disabled: true }] },
    {
      label: "&Help",
      items: () => [
        { label: "Keyboard Shortcuts", icon: "help", onClick: () => showShortcuts() },
        { label: "Privacy Policy", onClick: () => showPrivacyPolicy() },
        { type: "separator" },
        {
          label: "About ClassicMail",
          onClick: async () => showAbout(await fetchAppInfo()),
        },
      ],
    },
  ];
}
