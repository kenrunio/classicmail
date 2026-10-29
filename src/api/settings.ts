// Typed access to the settings table (rpc settings.*). Query key ["settings", ...] so the
// `settings.changed` event refreshes every window.
import { useQuery } from "@tanstack/react-query";
import { rpc } from "./rpc";
import { queryClient } from "./query";

/** Defaults for every known setting (the back end stores only what the user changed). */
export const SETTING_DEFAULTS = {
  "mail.pollSeconds": 60,
  "mail.markReadMode": "delay",
  "mail.markReadDelaySeconds": 1,
  "mail.prefetchDays": 30,
  /** keep every message body on this computer (background download, newest first) */
  "mail.downloadAllBodies": false,
  /** also download the attachments of the messages whose bodies are downloaded */
  "mail.downloadAttachments": false,
  "mail.fastInitialSync": true,
  "mail.blockRemoteImages": true,
  /** show Gmail's All Mail (every message) as a folder */
  "mail.showAllMail": false,
  /** load pictures automatically from contacts and senders whose pictures you downloaded before */
  "mail.autoPicturesKnownSenders": true,
  "mail.confirmPermanentDelete": true,
  /** Save As opens a company's usual folder once over this share (%) of its attachments went there */
  "mail.saveFolderPercent": 75,
  /** warning boxes answered with "Don't ask me again" (src/ui/dialog.tsx) */
  "ui.dontAsk": [] as string[],
  "mail.desktopAlert": true,
  "mail.composeFont": "Calibri, 11pt",
  /** message zoom in % (Ctrl + mouse wheel, View › Increase / Decrease Font Size) */
  "mail.bodyZoom": 100,
  /** Translate in a new message (or a reply to English mail) translates your text into this */
  "ai.myTranslateLanguage": "Hebrew",
  "calendar.workDays": [0, 1, 2, 3, 4],
  "calendar.workDayStart": 480,
  "calendar.workDayEnd": 1080,
  /** AI Review's meeting suggestions (src-tauri/src/ai/tools.rs): usual length and the gap kept around meetings */
  "calendar.meetingMinutes": 45,
  "calendar.meetingGapMinutes": 15,
  "calendar.weekStartsOn": 0,
  "calendar.defaultReminderMinutes": 15,
  "calendar.timeScale": 30,
  "app.closeToTray": false,
  /** how many actions Ctrl+Z can undo (1–100); the history is not kept across restarts */
  "app.undoSteps": 20,
  /** custom UI font (empty = Segoe UI Variable), size in px (0 = 13), weight 300/400/600 */
  "app.fontFamily": "",
  "app.fontSize": 0,
  "app.fontWeight": 400,
} as const;

export type SettingKey = keyof typeof SETTING_DEFAULTS;
type Widen<T> = T extends string ? string : T extends number ? number : T extends boolean ? boolean : T extends readonly number[] ? number[] : T;
export type SettingValues = { -readonly [K in SettingKey]: Widen<(typeof SETTING_DEFAULTS)[K]> };

export function useSettings() {
  const keys = Object.keys(SETTING_DEFAULTS) as SettingKey[];
  const q = useQuery({
    queryKey: ["settings", "all"],
    queryFn: () => rpc<Record<string, unknown>>("settings.getMany", { keys }),
  });
  const values = { ...SETTING_DEFAULTS } as unknown as SettingValues;
  for (const k of keys) {
    const v = q.data?.[k];
    if (v !== null && v !== undefined) (values as Record<string, unknown>)[k] = v;
  }
  return { values, loaded: q.isSuccess };
}

export async function setSetting<K extends SettingKey>(key: K, value: SettingValues[K]) {
  // optimistic: controlled checkboxes / number boxes show the change at once (no flicker, no lost keystrokes)
  const before = queryClient.getQueryData<Record<string, unknown>>(["settings", "all"])?.[key];
  queryClient.setQueryData<Record<string, unknown>>(["settings", "all"], (d) => ({ ...(d ?? {}), [key]: value }));
  try {
    await rpc("settings.set", { key, value });
  } catch (e) {
    // not saved: show the stored value again (then check it with the back end)
    queryClient.setQueryData<Record<string, unknown>>(["settings", "all"], (d) => ({ ...(d ?? {}), [key]: before }));
    queryClient.invalidateQueries({ queryKey: ["settings", "all"] });
    throw e;
  }
  // on success the back end's settings.changed event refreshes ["settings"] in every window
}
