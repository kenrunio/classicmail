// Services modules run while the main window is open (integrator-owned list).
import { onCmEvent } from "../api/events";
import { startReminderService } from "../modules/calendar/reminders";
import { showDesktopAlert, startDesktopAlerts } from "../modules/mail/compose/alerts";
import { startFindService } from "../modules/mail/findAll";
import type { NewMailEvent } from "../api/mailCompose";

/** Rules' "display a Desktop Alert" action → the same alert window as new mail. */
function startRuleAlerts(): () => void {
  return onCmEvent((e) => {
    if (e.kind !== "rules.alert") return;
    const messages = (e.messages as NewMailEvent["messages"]) ?? [];
    if (messages.length) showDesktopAlert({ accountId: Number((messages[0] as { accountId?: number }).accountId ?? 0), messages });
  });
}

export const MODULE_SERVICES: (() => () => void)[] = [startReminderService, startDesktopAlerts, startRuleAlerts, startFindService];
