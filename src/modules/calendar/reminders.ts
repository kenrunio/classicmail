// Reminder service (contract): the main window calls startReminderService() once. When the back
// end emits `reminder.due` (or reminders are already due at start-up) the always-on-top
// "reminders" window is opened / brought to the front.
import { onCmEvent } from "../../api/events";
import { openWindow } from "../../api/windows";
import { cal, type ReminderItem } from "../../api/calendar";

export function openReminders(items: ReminderItem[]) {
  if (!items.length) return;
  openWindow("reminders", {}, { label: "reminders", alwaysOnTop: true, title: `${items.length} Reminder${items.length === 1 ? "" : "s"}`, width: 440, height: 340 }).catch(
    () => {},
  );
}

export function startReminderService(): () => void {
  const off = onCmEvent((e) => {
    if (e.kind === "reminder.due") openReminders((e.items as ReminderItem[]) ?? []);
  });
  // also starts the back-end reminder loop
  cal
    .dueReminders()
    .then(openReminders)
    .catch(() => {});
  return off;
}
