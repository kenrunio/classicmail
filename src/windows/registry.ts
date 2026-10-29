// window kind (from ?window=) → component. Each window is code-split.
// Domain teams add their windows by creating the file named here; the integrator owns this list.
import { lazy, type ComponentType } from "react";

const WINDOWS: Record<string, () => Promise<{ default: ComponentType }>> = {
  main: () => import("../shell/MainWindow"),
  // Open in New Window: another main window on one module (MainWindow, isExplorer)
  explorer: () => import("../shell/MainWindow"),
  compose: () => import("../modules/mail/windows/ComposeWindow"),
  message: () => import("../modules/mail/windows/MessageWindow"),
  "desktop-alert": () => import("../modules/mail/windows/DesktopAlertWindow"),
  appointment: () => import("../modules/calendar/windows/AppointmentWindow"),
  recurrence: () => import("../modules/calendar/windows/RecurrenceWindow"),
  reminders: () => import("../modules/calendar/windows/RemindersWindow"),
  contact: () => import("../modules/contacts/windows/ContactWindow"),
  "address-book": () => import("../modules/contacts/windows/AddressBookWindow"),
  task: () => import("../modules/tasks/windows/TaskWindow"),
  rules: () => import("../modules/rules/RulesWindow"),
  options: () => import("./OptionsWindow"),
  accounts: () => import("./AccountsWindow"),
};

/**
 * The window's component, loaded before the first render. (Rendering a lazy() component instead
 * shows a fallback first, and React then holds the real content back ~300 ms to avoid flicker.)
 * Falls back to lazy() if the chunk can't be loaded up front.
 */
export function loadWindow(kind: string): Promise<ComponentType> {
  const loader = WINDOWS[kind] ?? WINDOWS.main;
  return loader().then(
    (m) => m.default,
    () => lazy(loader),
  );
}
