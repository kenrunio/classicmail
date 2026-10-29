// Tools › Options — Outlook 2007 style tabbed dialog over the settings table.
import { useEffect, useState, type ReactNode } from "react";
import { rpc } from "../api/rpc";
import { keepPreviousData } from "@tanstack/react-query";
import { useAppInfo, useRpc } from "../api/query";
import { setSetting, useSettings, type SettingKey, type SettingValues } from "../api/settings";
import { closeCurrentWindow, setWindowTitle, windowParams } from "../api/windows";
import { Icon, errorBox } from "../ui";
import { OPTION_EXTRA_TABS } from "./optionTabs";
import { DEFAULT_SIZE, FONT_CHOICES, SIZE_CHOICES, WEIGHT_CHOICES } from "../ui/font";
import "./windows.css";

const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

function minutesToTime(m: number) {
  return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}
function timeToMinutes(t: string) {
  const [h, m] = t.split(":").map(Number);
  return (h || 0) * 60 + (m || 0);
}

export default function OptionsWindow() {
  useEffect(() => {
    setWindowTitle("Options");
  }, []);
  const { values, loaded } = useSettings();
  const [tab, setTab] = useState(() => windowParams().get("tab") ?? "prefs");
  // "Set up translation…" etc. while this window is already open: switch to the requested tab
  useEffect(() => {
    const un = import("@tauri-apps/api/event").then(({ listen }) => listen<string>("cm-options-tab", (e) => setTab(e.payload)));
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);
  const set = <K extends SettingKey>(k: K, v: SettingValues[K]) => setSetting(k, v).catch((e) => errorBox(e));

  const tabs: { id: string; label: string; body: ReactNode }[] = [
    { id: "prefs", label: "Preferences", body: <Preferences values={values} set={set} /> },
    { id: "setup", label: "Mail Setup", body: <MailSetup values={values} set={set} /> },
    ...OPTION_EXTRA_TABS.map((t) => ({ id: t.id, label: t.label, body: <t.Component /> })),
    { id: "other", label: "Other", body: <Other values={values} set={set} /> },
  ];
  const current = tabs.find((t) => t.id === tab) ?? tabs[0];

  return (
    <div className="cm-dlgwin">
      <div className="cm-dlgwin-tabs">
        {tabs.map((t) => (
          <div key={t.id} className={`cm-dlgwin-tab${t.id === current.id ? " active" : ""}`} onMouseDown={() => setTab(t.id)}>
            {t.label}
          </div>
        ))}
      </div>
      <div className="cm-dlgwin-page">{loaded ? current.body : <div className="cm-empty">Loading…</div>}</div>
      <div className="cm-dialog-footer">
        <span className="cm-muted cm-fill" style={{ alignSelf: "center" }}>
          Changes are saved as you make them.
        </span>
        <button className="cm-button default" onClick={() => closeCurrentWindow()}>
          OK
        </button>
      </div>
    </div>
  );
}

type Props = { values: SettingValues; set: <K extends SettingKey>(k: K, v: SettingValues[K]) => void };

function Section({ icon, title, children }: { icon: string; title: string; children: ReactNode }) {
  return (
    <div className="cm-opt-section">
      <div className="cm-opt-icon">
        <Icon name={icon} size={32} />
      </div>
      <fieldset className="cm-fieldset">
        <legend>{title}</legend>
        {children}
      </fieldset>
    </div>
  );
}

function Preferences({ values, set }: Props) {
  return (
    <>
      <Section icon="mail" title="E-mail">
        <div className="cm-opt-row">
          <span>Mark items as read:</span>
          <select className="cm-input" value={values["mail.markReadMode"]} onChange={(e) => set("mail.markReadMode", e.target.value)}>
            <option value="delay">after being viewed in the Reading Pane</option>
            <option value="selectionChange">when the selection changes</option>
            <option value="never">never (only when opened)</option>
          </select>
        </div>
        {values["mail.markReadMode"] === "delay" && (
          <div className="cm-opt-row">
            <span>Wait</span>
            <input
              className="cm-input"
              type="number"
              min={0}
              max={60}
              style={{ width: 60 }}
              value={values["mail.markReadDelaySeconds"]}
              onChange={(e) => set("mail.markReadDelaySeconds", Math.max(0, Number(e.target.value)))}
            />
            <span>seconds before marking an item as read</span>
          </div>
        )}
        <label className="cm-opt-row">
          <input type="checkbox" checked={values["mail.blockRemoteImages"]} onChange={(e) => set("mail.blockRemoteImages", e.target.checked)} />
          Don't download pictures automatically in HTML e-mail (except from Safe Senders)
        </label>
        <label className="cm-opt-row" style={{ marginLeft: 22 }}>
          <input
            type="checkbox"
            disabled={!values["mail.blockRemoteImages"]}
            checked={values["mail.autoPicturesKnownSenders"]}
            onChange={(e) => set("mail.autoPicturesKnownSenders", e.target.checked)}
          />
          …but download them automatically from my contacts and from senders I've downloaded pictures from before
        </label>
        <label className="cm-opt-row">
          <input type="checkbox" checked={values["mail.confirmPermanentDelete"]} onChange={(e) => set("mail.confirmPermanentDelete", e.target.checked)} />
          Warn before permanently deleting items
        </label>
        <label className="cm-opt-row">
          <input type="checkbox" checked={values["mail.desktopAlert"]} onChange={(e) => set("mail.desktopAlert", e.target.checked)} />
          Display a New Mail Desktop Alert
        </label>
        <div
          className="cm-opt-row"
          title="Save As, Save All Attachments and Attach File open in the folder used before for that company (its e-mail domain); with no usual folder, in the folder its files share."
        >
          <span>Attachments: open a company's usual folder above</span>
          <input
            className="cm-input"
            type="number"
            min={1}
            max={100}
            style={{ width: 52 }}
            value={values["mail.saveFolderPercent"]}
            onChange={(e) => set("mail.saveFolderPercent", Math.min(100, Math.max(1, Number(e.target.value) || 75)))}
          />
          <span>% of saves</span>
        </div>
        <label className="cm-opt-row">
          <input
            type="checkbox"
            checked={values["mail.showAllMail"]}
            onChange={(e) => {
              set("mail.showAllMail", e.target.checked);
              rpc("sync.now", {}).catch(() => {});
            }}
          />
          Show Gmail's "All Mail" folder (every message, including archived ones)
        </label>
      </Section>
      <Section icon="options" title="Appearance">
        <div className="cm-opt-row">
          <span style={{ width: 44 }}>Font:</span>
          <select
            className="cm-input"
            style={{ width: 190 }}
            value={FONT_CHOICES.includes(values["app.fontFamily"]) || !values["app.fontFamily"] ? values["app.fontFamily"] : "__other"}
            onChange={(e) => set("app.fontFamily", e.target.value === "__other" ? values["app.fontFamily"] || " " : e.target.value)}
          >
            <option value="">Default (Segoe UI)</option>
            {FONT_CHOICES.map((f) => (
              <option key={f} value={f} style={{ fontFamily: f }}>
                {f}
              </option>
            ))}
            <option value="__other">Other…</option>
          </select>
          {values["app.fontFamily"] && !FONT_CHOICES.includes(values["app.fontFamily"]) && (
            <input
              className="cm-input"
              style={{ width: 130 }}
              placeholder="font name"
              defaultValue={values["app.fontFamily"].trim()}
              onBlur={(e) => set("app.fontFamily", e.target.value.trim())}
            />
          )}
        </div>
        <div className="cm-opt-row">
          <span style={{ width: 44 }}>Size:</span>
          <select className="cm-input" value={values["app.fontSize"]} onChange={(e) => set("app.fontSize", Number(e.target.value))}>
            <option value={0}>Default ({DEFAULT_SIZE})</option>
            {SIZE_CHOICES.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
          <span style={{ marginLeft: 8 }}>Weight:</span>
          <select className="cm-input" value={values["app.fontWeight"]} onChange={(e) => set("app.fontWeight", Number(e.target.value))}>
            {WEIGHT_CHOICES.map((w) => (
              <option key={w.value} value={w.value}>
                {w.label}
              </option>
            ))}
          </select>
        </div>
        <div className="cm-opt-row cm-muted" style={{ border: "1px solid #c3d4ea", padding: "6px 8px", background: "#fff", color: "#000" }}>
          Preview — Inbox (12) · RE: Q3 earnings call — ישיבת דירקטוריון
        </div>
        <div className="cm-opt-row">
          <button
            className="cm-button"
            onClick={() => {
              set("app.fontFamily", "");
              set("app.fontSize", 0);
              set("app.fontWeight", 400);
            }}
          >
            Reset font
          </button>
        </div>
      </Section>
      <Section icon="calendar" title="Calendar">
        <div className="cm-opt-row">
          <span>Calendar work week:</span>
          {DAYS.map((d, i) => (
            <label key={d} style={{ display: "flex", alignItems: "center", gap: 2 }}>
              <input
                type="checkbox"
                checked={values["calendar.workDays"].includes(i)}
                onChange={(e) => set("calendar.workDays", e.target.checked ? [...values["calendar.workDays"], i].sort() : values["calendar.workDays"].filter((x) => x !== i))}
              />
              {d}
            </label>
          ))}
        </div>
        <div className="cm-opt-row">
          <span>Start time:</span>
          <input
            className="cm-input"
            type="time"
            value={minutesToTime(values["calendar.workDayStart"])}
            onChange={(e) => set("calendar.workDayStart", timeToMinutes(e.target.value))}
          />
          <span>End time:</span>
          <input
            className="cm-input"
            type="time"
            value={minutesToTime(values["calendar.workDayEnd"])}
            onChange={(e) => set("calendar.workDayEnd", timeToMinutes(e.target.value))}
          />
        </div>
        <div className="cm-opt-row" title="Used when AI Review suggests meeting times from your calendar">
          <span>Usual meeting length:</span>
          <input
            className="cm-input"
            type="number"
            min={5}
            max={600}
            step={5}
            style={{ width: 56 }}
            value={values["calendar.meetingMinutes"]}
            onChange={(e) => set("calendar.meetingMinutes", Math.max(5, Number(e.target.value) || 45))}
          />
          <span>minutes</span>
          <span style={{ marginLeft: 12 }}>Gap between meetings:</span>
          <input
            className="cm-input"
            type="number"
            min={0}
            max={240}
            step={5}
            style={{ width: 56 }}
            value={values["calendar.meetingGapMinutes"]}
            onChange={(e) => set("calendar.meetingGapMinutes", Math.max(0, Number(e.target.value) || 0))}
          />
          <span>minutes</span>
        </div>
        <div className="cm-opt-row">
          <span>First day of week:</span>
          <select className="cm-input" value={values["calendar.weekStartsOn"]} onChange={(e) => set("calendar.weekStartsOn", Number(e.target.value))}>
            <option value={0}>Sunday</option>
            <option value={1}>Monday</option>
            <option value={6}>Saturday</option>
          </select>
          <span style={{ marginLeft: 12 }}>Time scale:</span>
          <select className="cm-input" value={values["calendar.timeScale"]} onChange={(e) => set("calendar.timeScale", Number(e.target.value))}>
            {[5, 6, 10, 15, 30, 60].map((m) => (
              <option key={m} value={m}>
                {m} minutes
              </option>
            ))}
          </select>
        </div>
        <div className="cm-opt-row">
          <span>Default reminder:</span>
          <select className="cm-input" value={values["calendar.defaultReminderMinutes"]} onChange={(e) => set("calendar.defaultReminderMinutes", Number(e.target.value))}>
            <option value={-1}>None</option>
            {[0, 5, 10, 15, 30, 60, 120, 1440].map((m) => (
              <option key={m} value={m}>
                {m === 0 ? "0 minutes" : m < 60 ? `${m} minutes` : m === 1440 ? "1 day" : `${m / 60} hour${m > 60 ? "s" : ""}`}
              </option>
            ))}
          </select>
        </div>
      </Section>
    </>
  );
}

/** "3.5 GB" / "420 MB" */
function fmtBytes(n: number) {
  return n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${Math.max(1, Math.round(n / 1e6))} MB`;
}

type Estimate = { bodiesLeft: number; bodiesBytes: number; attachmentsBytes: number };

/** "— about 3.6 GB still to download (49,185 messages)" / "— all downloaded" */
function bodiesNote(e: Estimate | undefined) {
  if (!e) return "";
  return e.bodiesLeft > 0 ? `— about ${fmtBytes(e.bodiesBytes)} still to download (${e.bodiesLeft.toLocaleString()} messages)` : "— all downloaded";
}

function MailSetup({ values, set }: Props) {
  const all = values["mail.downloadAllBodies"];
  const days = values["mail.prefetchDays"];
  // what the background download would still fetch, for the last N days and for all mail
  const estimate = useRpc<{ recent: Estimate; all: Estimate }>(["mail", "downloadEstimate", days], "mail.downloadEstimate", { days }, { placeholderData: keepPreviousData }).data;
  const chosen = all ? estimate?.all : estimate?.recent;
  // start (or narrow) the background download at once
  const change = (key: "mail.downloadAllBodies" | "mail.downloadAttachments", v: boolean) => {
    setSetting(key, v)
      .then(() => rpc("sync.now", {}))
      .catch((e) => errorBox(e));
  };
  return (
    <>
      <Section icon="accounts" title="E-mail Accounts">
        <div className="cm-opt-row">
          <span className="cm-fill">Set up e-mail accounts and Google sign-in.</span>
          <button className="cm-button" onClick={() => import("../api/windows").then((w) => w.openWindow("accounts", {}, { label: "accounts", title: "Account Settings" }))}>
            E-mail Accounts…
          </button>
        </div>
      </Section>
      <Section icon="send-receive" title="Send/Receive">
        <div className="cm-opt-row">
          <span>Check for new mail every</span>
          <input
            className="cm-input"
            type="number"
            min={1}
            max={120}
            style={{ width: 60 }}
            value={Math.round(values["mail.pollSeconds"] / 60)}
            onChange={(e) => set("mail.pollSeconds", Math.max(1, Number(e.target.value)) * 60)}
          />
          <span>minutes</span>
        </div>
        <div className="cm-opt-row">Keep messages on this computer (others download when opened):</div>
        <label className="cm-opt-row cm-opt-sub">
          <input type="radio" name="mail-download" checked={!all} onChange={() => change("mail.downloadAllBodies", false)} />
          <span>The last</span>
          <input
            className="cm-input"
            type="number"
            min={0}
            max={3650}
            style={{ width: 60 }}
            value={days}
            disabled={all}
            onChange={(e) => set("mail.prefetchDays", Math.max(0, Number(e.target.value)))}
          />
          <span>days</span>
          {!all && <span className="cm-muted">{bodiesNote(estimate?.recent)}</span>}
        </label>
        <label className="cm-opt-row cm-opt-sub">
          <input type="radio" name="mail-download" checked={all} onChange={() => change("mail.downloadAllBodies", true)} />
          <span>All mail, downloaded in the background</span>
          <span className="cm-muted">{bodiesNote(estimate?.all)}</span>
        </label>
        <label className="cm-opt-row">
          <input
            type="checkbox"
            checked={values["mail.downloadAttachments"] && (all || days > 0)}
            disabled={!all && days <= 0}
            onChange={(e) => change("mail.downloadAttachments", e.target.checked)}
          />
          <span>{all ? "Also download all attachments" : `Also download the attachments of those ${days} days`}</span>
          {chosen && chosen.attachmentsBytes > 0 && <span className="cm-muted">— up to about {fmtBytes(chosen.attachmentsBytes)}</span>}
        </label>
        <label className="cm-opt-row">
          <input type="checkbox" checked={values["mail.fastInitialSync"]} onChange={(e) => set("mail.fastInitialSync", e.target.checked)} />
          Download message headers over IMAP the first time (much faster for big mailboxes)
        </label>
        <div className="cm-opt-row">
          <button className="cm-button" onClick={() => rpc("sync.now", {})}>
            Send/Receive Now
          </button>
        </div>
      </Section>
    </>
  );
}

function Other({ values, set }: Props) {
  const autostart = useRpc<boolean>(["settings", "autostart"], "app.getAutostart");
  // show the click at once (the registry write + refetch takes a moment)
  const [pending, setPending] = useState<boolean | null>(null);
  // the Windows mail client Office's "Email" button calls (src-tauri/src/mapi.rs)
  // ok: every piece in place, for every program (64- and 32-bit, Office or not)
  const mailClient = useRpc<{ enabled: boolean; office: boolean; ok: boolean }>(["settings", "mailClient"], "app.getMailClient");
  // on (and Fix It): for this user, and Windows asks for administrator permission for the computer-wide part
  // (Word / Excel go by it, and it serves 32-bit programs too) when that is missing or old
  const setMailClient = async (enabled: boolean) => {
    setMailPending(enabled);
    await rpc("app.setMailClient", { enabled }).catch((err) => errorBox(err));
    await mailClient.refetch();
    setMailPending(null);
  };
  const [mailPending, setMailPending] = useState<boolean | null>(null);
  const info = useAppInfo();
  return (
    <>
      <Section icon="options" title="General">
        <label className="cm-opt-row">
          <input
            type="checkbox"
            checked={pending ?? !!autostart.data}
            onChange={async (e) => {
              setPending(e.target.checked);
              await rpc("app.setAutostart", { enabled: e.target.checked }).catch((err) => errorBox(err));
              await autostart.refetch();
              setPending(null);
            }}
          />
          Start ClassicMail when Windows starts (minimised to the notification area)
        </label>
        <label className="cm-opt-row">
          <input type="checkbox" checked={mailPending ?? !!mailClient.data?.enabled} onChange={(e) => void setMailClient(e.target.checked)} />
          Use ClassicMail for e-mail from Word, Excel and other programs ("Send as Attachment")
        </label>
        {mailClient.data?.enabled && !mailClient.data.ok && mailPending === null && (
          <div className="cm-opt-row" style={{ marginLeft: 22 }}>
            <span className="cm-muted cm-fill">
              {mailClient.data.office
                ? "Some programs can't reach ClassicMail: its mail settings need repairing."
                : "Word, Excel and some other programs still use their own mail program."}
            </span>
            <button className="cm-button" onClick={() => void setMailClient(true)}>
              Fix It…
            </button>
          </div>
        )}
        <label className="cm-opt-row">
          <input type="checkbox" checked={values["app.closeToTray"]} onChange={(e) => set("app.closeToTray", e.target.checked)} />
          Closing the main window keeps ClassicMail running in the notification area
        </label>
      </Section>
      <Section icon="warning" title="Warnings">
        {(() => {
          // boxes answered with "Don't ask me again", plus the permanent-delete warning's own checkbox
          const off = values["ui.dontAsk"].length + (values["mail.confirmPermanentDelete"] ? 0 : 1);
          return (
            <div className="cm-opt-row">
              <span className="cm-fill">{off === 0 ? "Every warning is shown." : `${off} warning${off === 1 ? " is" : "s are"} turned off with "Don't ask me again".`}</span>
              <button
                className="cm-button"
                disabled={off === 0}
                onClick={() => {
                  set("ui.dontAsk", []);
                  set("mail.confirmPermanentDelete", true);
                }}
              >
                Show All Warnings Again
              </button>
            </div>
          );
        })()}
      </Section>
      <Section icon="undo" title="Undo">
        <div className="cm-opt-row">
          <span>Undo history: keep the last</span>
          <input
            className="cm-input"
            type="number"
            min={1}
            max={100}
            style={{ width: 60 }}
            value={values["app.undoSteps"]}
            onChange={(e) => {
              const n = Math.min(100, Math.max(1, Math.round(Number(e.target.value)) || 1));
              set("app.undoSteps", n);
              rpc("undo.setLimit", { steps: n }).catch(() => {});
            }}
          />
          <span>actions</span>
        </div>
        <div className="cm-muted">Ctrl+Z undoes the last thing you did (and goes further back each time); Ctrl+Y redoes. The history is cleared when ClassicMail closes.</div>
      </Section>
      <Section icon="info" title="About">
        <div>ClassicMail {info.data?.version}</div>
        <div className="cm-muted" style={{ userSelect: "text" }}>
          Data folder: {info.data?.dataDir}
        </div>
      </Section>
    </>
  );
}
