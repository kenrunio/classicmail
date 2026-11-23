// Tools › Options › Large Files: attachments too large for Gmail go to a folder in each account's
// Google Drive and are sent as links (src-tauri/src/mail/compose/drive_folder.rs). The folder's name,
// what is in it, who may open a file, and the clean-up of old files.
import { useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Icon, errorBox, alertBox, confirmBox } from "../../../ui";
import { rpc } from "../../../api/rpc";
import { queryClient } from "../../../api/query";
import { setSetting, useSettings } from "../../../api/settings";
import { openWindow } from "../../../api/windows";
import { driveApi, type DriveAccountStatus } from "../../../api/mailCompose";

function bytes(n: number): string {
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(1)} GB`;
  if (n >= 1024 ** 2) return `${Math.round(n / 1024 ** 2)} MB`;
  return n > 0 ? `${Math.max(1, Math.round(n / 1024))} KB` : "0 KB";
}

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

function AccountLine({ a }: { a: DriveAccountStatus }) {
  const open = (url: string) => void rpc("app.openUrl", { url }).catch(errorBox);
  return (
    <div className="cm-opt-row" style={{ alignItems: "flex-start", marginBottom: 8 }}>
      <span className="cm-fill" style={{ minWidth: 0 }}>
        <div style={{ fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis" }} title={a.email}>
          {a.email}
        </div>
        {a.state === "ok" && (
          <>
            {a.files === 1 ? "1 file" : `${a.files} files`}, {bytes(a.bytes)}
            {a.folderName && <span className="cm-muted"> in "{a.folderName}"</span>}
          </>
        )}
        {a.state === "none" && <span className="cm-muted">No folder yet: it is made with the first large file.</span>}
        {(a.state === "signIn" || a.state === "error") && (
          <span style={{ color: "#c00000" }}>
            <Icon name="warning" size={14} /> {a.message}
          </span>
        )}
        {a.usage != null && (
          <div className="cm-muted">
            Google Drive: {bytes(a.usage)} {a.limit ? `of ${bytes(a.limit)} used` : "used"}
          </div>
        )}
      </span>
      {a.folderLink && (
        <button className="cm-button" onClick={() => open(a.folderLink!)}>
          Open in Drive
        </button>
      )}
      {a.state === "signIn" && (
        <button className="cm-button" onClick={() => openWindow("accounts", {}, { label: "accounts", title: "Account Settings" })}>
          Account Settings…
        </button>
      )}
    </div>
  );
}

export function LargeFilesOptions() {
  const { values } = useSettings();
  const status = useQuery({ queryKey: ["drive", "status"], queryFn: driveApi.status, staleTime: 30_000 });
  const [name, setName] = useState<string | null>(null);
  const [cleaning, setCleaning] = useState(false);
  const folderName = values["drive.folderName"] || "ClassicMail";
  const refresh = () => queryClient.invalidateQueries({ queryKey: ["drive", "status"] });

  const rename = async () => {
    const n = (name ?? "").trim();
    setName(null);
    if (!n || n === folderName) return;
    try {
      await driveApi.renameFolder(n);
      refresh();
    } catch (e) {
      errorBox(e, "Large Files");
    }
  };

  const cleanNow = async () => {
    const days = values["drive.cleanupDays"];
    if (
      !(await confirmBox(
        `Move the files ClassicMail uploaded more than ${days} days ago to the Google Drive trash?\n\nTheir links stop working for anyone who hasn't downloaded them. Drive keeps them in its trash for 30 days.`,
        "Clean Up",
      ))
    )
      return;
    setCleaning(true);
    try {
      const r = await driveApi.cleanNow(days);
      refresh();
      const moved = r.trashed === 1 ? "1 file was" : `${r.trashed} files were`;
      await alertBox(
        `${moved} moved to the Google Drive trash.${r.failed.length ? `\n\nNot done for:\n${r.failed.join("\n")}` : ""}`,
        "Clean Up",
        r.failed.length ? "warning" : "info",
      );
    } catch (e) {
      errorBox(e, "Clean Up");
    } finally {
      setCleaning(false);
    }
  };

  return (
    <>
      <Section icon="drive" title="Google Drive links">
        <div className="cm-opt-row">
          A file too large for e-mail (Gmail takes up to 25 MB per message) is put in a folder in your Google Drive and sent as a link. ClassicMail can see only that folder and the
          files it put there.
        </div>
        <div className="cm-opt-row">
          <span style={{ whiteSpace: "nowrap" }}>Folder name:</span>
          <input
            className="cm-input"
            style={{ width: 200 }}
            maxLength={100}
            value={name ?? folderName}
            onChange={(e) => setName(e.target.value)}
            onBlur={() => void rename()}
            onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
          />
        </div>
        <div className="cm-muted">You can also move the folder anywhere in your Drive: ClassicMail still finds it.</div>
        <div className="cm-opt-row" style={{ marginTop: 10 }}>
          <b className="cm-fill">The folder in each account</b>
          <button className="cm-button" onClick={refresh} disabled={status.isFetching}>
            {status.isFetching ? "Checking…" : "Refresh"}
          </button>
        </div>
        {status.isPending && <div className="cm-muted">Checking Google Drive…</div>}
        {status.isError && <div style={{ color: "#c00000" }}>{String((status.error as Error)?.message ?? status.error)}</div>}
        {status.data?.accounts.map((a) => (
          <AccountLine key={a.accountId} a={a} />
        ))}
        {status.data && status.data.accounts.length === 0 && <div className="cm-muted">No Google accounts.</div>}
      </Section>
      <Section icon="contacts" title="Who can open the file">
        <div className="cm-opt-row">When you press Send, everyone the message goes to is given access to the file. Nobody else can open it.</div>
        <div className="cm-opt-row">If a recipient has no Google account:</div>
        <div className="cm-opt-row">
          <select className="cm-input" value={values["drive.noGoogleAccount"]} onChange={(e) => setSetting("drive.noGoogleAccount", e.target.value).catch(errorBox)}>
            <option value="anyone">let anyone with the link open the file</option>
            <option value="private">keep the file private (they must ask you for access)</option>
          </select>
        </div>
      </Section>
      <Section icon="deleted" title="Clean up">
        <div className="cm-opt-row" style={{ flexWrap: "wrap" }}>
          <label className="cm-opt-pair">
            <input type="checkbox" checked={values["drive.cleanup"]} onChange={(e) => setSetting("drive.cleanup", e.target.checked).catch(errorBox)} />
            Move files older than
          </label>
          <input
            className="cm-input"
            type="number"
            min={1}
            max={3650}
            style={{ width: 60 }}
            value={values["drive.cleanupDays"]}
            onChange={(e) => setSetting("drive.cleanupDays", Math.min(3650, Math.max(1, Number(e.target.value) || 30))).catch(errorBox)}
          />
          <span>days to the Google Drive trash (once a day)</span>
        </div>
        <div className="cm-opt-row">
          <span className="cm-fill" />
          <button className="cm-button" onClick={() => void cleanNow()} disabled={cleaning}>
            {cleaning ? "Cleaning…" : "Clean Up Now"}
          </button>
        </div>
        <div className="cm-muted">Only files ClassicMail uploaded are touched. A link stops working once its file is gone; Drive keeps the file in its trash for 30 days.</div>
      </Section>
    </>
  );
}
