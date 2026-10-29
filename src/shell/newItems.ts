// "New ▾" items available everywhere (File › New, the New split button's menu).
import { useHotkeys, type MenuItem } from "../ui";
import { openWindow } from "../api/windows";

export const newMail = () => openWindow("compose", {}, { title: "Untitled - Message" });
export const newAppointment = (start?: number, end?: number, allDay?: boolean) =>
  openWindow("appointment", { start, end, allDay: allDay ? 1 : undefined }, { title: "Untitled - Appointment" });
export const newMeeting = () => openWindow("appointment", { meeting: 1 }, { title: "Untitled - Meeting" });
export const newContact = () => openWindow("contact", {}, { title: "Untitled - Contact" });
export const newTask = () => openWindow("task", {}, { title: "Untitled - Task" });

export function newItemMenu(): MenuItem[] {
  return [
    { label: "Mail Message", icon: "new-mail", shortcut: "Ctrl+Shift+M", onClick: newMail },
    { type: "separator" },
    { label: "Appointment", icon: "appointment", shortcut: "Ctrl+Shift+A", onClick: () => newAppointment() },
    { label: "Meeting Request", icon: "meeting", shortcut: "Ctrl+Shift+Q", onClick: newMeeting },
    { label: "Contact", icon: "contact", shortcut: "Ctrl+Shift+C", onClick: newContact },
    { label: "Task", icon: "task", shortcut: "Ctrl+Shift+K", onClick: newTask },
  ];
}

// Ctrl+N in an item window makes another item of its kind (Outlook)
const NEW_OF_KIND: Record<string, () => unknown> = { compose: newMail, message: newMail, appointment: () => newAppointment(), contact: newContact, task: newTask };

/** The New shortcuts in an item window too (the main window has its own, MainWindow.tsx). */
export function NewItemKeys({ kind }: { kind: string }) {
  const ofKind = NEW_OF_KIND[kind];
  useHotkeys({
    ...(ofKind ? { "ctrl+n": () => void ofKind() } : {}),
    "ctrl+shift+m": () => void newMail(),
    "ctrl+shift+a": () => void newAppointment(),
    "ctrl+shift+q": () => void newMeeting(),
    "ctrl+shift+c": () => void newContact(),
    "ctrl+shift+k": () => void newTask(),
  });
  return null;
}
