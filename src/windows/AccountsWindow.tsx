// Tools › Account Settings: the accounts list (add Google / demo, repair sign-in, rename, colour,
// order, remove) and the Google OAuth client.
import { useEffect, useState } from "react";
import { rpc } from "../api/rpc";
import { useAppInfo, useRpc, queryClient } from "../api/query";
import type { Account } from "../api/types";
import { closeCurrentWindow, setWindowTitle } from "../api/windows";
import { Dialog, Icon, confirmBox, errorBox, alertBox } from "../ui";
import { openGuide } from "../shell/FirstRun";
import "./windows.css";

const COLORS = ["#4A7EBB", "#C0504D", "#9BBB59", "#8064A2", "#F79646", "#4BACC6", "#7F7F7F"];

export default function AccountsWindow() {
  useEffect(() => {
    setWindowTitle("Account Settings");
  }, []);
  const accounts = useRpc<Account[]>(["accounts"], "accounts.list");
  const info = useAppInfo();
  const [sel, setSel] = useState<number | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [editing, setEditing] = useState<Account | null>(null);
  const [adding, setAdding] = useState(false);
  const list = accounts.data ?? [];
  const selected = list.find((a) => a.id === sel) ?? null;

  const refresh = () => queryClient.invalidateQueries({ queryKey: ["accounts"] });

  const signIn = async (hint: string) => {
    setBusy("signin");
    try {
      await rpc("accounts.signInGoogle", { loginHint: hint || undefined });
      refresh();
      setAdding(false);
    } catch (e) {
      errorBox(e, "Account Settings");
    } finally {
      setBusy(null);
    }
  };

  const importClient = async (file: File) => {
    try {
      await rpc("google.setClientJson", { json: await file.text() });
      info.refetch();
      alertBox("The Google client file was imported. You can now add your Google accounts.", "Account Settings");
    } catch (e) {
      errorBox(e, "Account Settings");
    }
  };

  const remove = async (a: Account) => {
    if (!(await confirmBox(`Remove ${a.email}?\n\nIts mail, calendar, contacts and tasks will be removed from this computer (they stay in Google).`, "Remove Account"))) return;
    try {
      await rpc("accounts.remove", { id: a.id });
      setSel(null);
      refresh();
    } catch (e) {
      errorBox(e);
    }
  };

  const move = async (a: Account, dir: -1 | 1) => {
    const idx = list.findIndex((x) => x.id === a.id);
    const other = list[idx + dir];
    if (!other) return;
    try {
      await rpc("accounts.update", { id: a.id, position: other.position });
      await rpc("accounts.update", { id: other.id, position: a.position === other.position ? a.position + dir : a.position });
    } catch (e) {
      errorBox(e, "Account Settings");
    }
    // also after a failure: show the order that was actually saved
    refresh();
  };

  return (
    <div className="cm-dlgwin">
      <div className="cm-dlgwin-tabs">
        <div className="cm-dlgwin-tab active">E-mail</div>
      </div>
      <div className="cm-dlgwin-page">
        <p style={{ marginTop: 0 }}>You can add or remove an account. You can select an account and change its settings.</p>
        <div className="cm-acct-toolbar">
          <button className="cm-tb-btn" onClick={() => setAdding(true)}>
            <Icon name="new" /> New…
          </button>
          <button className="cm-tb-btn" disabled={!selected || selected.provider !== "google" || !!busy} onClick={() => selected && signIn(selected.email)}>
            <Icon name="refresh" /> Repair…
          </button>
          <button className="cm-tb-btn" disabled={!selected} onClick={() => selected && setEditing(selected)}>
            <Icon name="rename" /> Change…
          </button>
          <button className="cm-tb-btn" disabled={!selected} onClick={() => selected && remove(selected)}>
            <Icon name="delete-red" /> Remove
          </button>
          <button className="cm-tb-btn" disabled={!selected} title="Move up" onClick={() => selected && move(selected, -1)}>
            <Icon name="arrow-up" />
          </button>
          <button className="cm-tb-btn" disabled={!selected} title="Move down" onClick={() => selected && move(selected, 1)}>
            <Icon name="arrow-down" />
          </button>
        </div>
        <div className="cm-acct-list">
          <div className="cm-acct-row head">
            <span>Name</span>
            <span>Type</span>
          </div>
          {list.map((a, i) => (
            <div key={a.id} className={`cm-acct-row${a.id === sel ? " selected" : ""}`} onMouseDown={() => setSel(a.id)} onDoubleClick={() => setEditing(a)}>
              <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <span className="cm-tree-swatch" style={{ background: a.color }} />
                {a.email}
                {i === 0 && <span className="cm-muted">(send from this account by default)</span>}
              </span>
              <span>{a.provider === "google" ? "Google Workspace" : "Demo (offline sample data)"}</span>
            </div>
          ))}
          {list.length === 0 && <div className="cm-empty">No accounts yet — click New…</div>}
        </div>
        <fieldset className="cm-fieldset" style={{ marginTop: 12 }}>
          <legend>Google sign-in</legend>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <Icon name={info.data?.googleClientConfigured ? "accept" : "warning"} />
            <span className="cm-fill">
              {info.data?.googleClientConfigured ? (
                info.data.googleClientEmbedded ? (
                  "Google OAuth client built into this copy of ClassicMail."
                ) : (
                  "Google OAuth client imported."
                )
              ) : (
                <>
                  No Google OAuth client yet — import the Desktop-app client JSON (see the{" "}
                  <a className="cm-link" onClick={openGuide}>
                    setup guide
                  </a>
                  ).
                </>
              )}
            </span>
            <label className="cm-button" style={{ display: "inline-flex", alignItems: "center" }}>
              Import client file…
              <input type="file" accept=".json,application/json" hidden onChange={(e) => e.target.files?.[0] && importClient(e.target.files[0])} />
            </label>
          </div>
        </fieldset>
      </div>
      <div className="cm-dialog-footer">
        <button className="cm-button default" onClick={() => closeCurrentWindow()}>
          Close
        </button>
      </div>

      {adding && (
        <AddAccountDialog
          busy={busy === "signin"}
          clientReady={!!info.data?.googleClientConfigured}
          onGoogle={signIn}
          onDemo={async () => {
            try {
              await rpc("accounts.addDemo", {});
              refresh();
              setAdding(false);
            } catch (e) {
              errorBox(e, "Account Settings");
            }
          }}
          onClose={() => setAdding(false)}
        />
      )}
      {editing && (
        <ChangeAccountDialog
          account={editing}
          onClose={() => setEditing(null)}
          onSave={async (name, color) => {
            try {
              await rpc("accounts.update", { id: editing.id, displayName: name, color });
              refresh();
              setEditing(null);
            } catch (e) {
              errorBox(e, "Account Settings");
            }
          }}
        />
      )}
    </div>
  );
}

function AddAccountDialog({
  busy,
  clientReady,
  onGoogle,
  onDemo,
  onClose,
}: {
  busy: boolean;
  clientReady: boolean;
  onGoogle: (hint: string) => void;
  onDemo: () => void;
  onClose: () => void;
}) {
  const [kind, setKind] = useState<"google" | "demo">("google");
  const [email, setEmail] = useState("");
  return (
    <Dialog
      title="Add New E-mail Account"
      onClose={onClose}
      width={460}
      footer={
        <>
          <button className="cm-button default" disabled={busy || (kind === "google" && !clientReady)} onClick={() => (kind === "google" ? onGoogle(email.trim()) : onDemo())}>
            {busy ? "Waiting for browser…" : "Next >"}
          </button>
          <button className="cm-button" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <label style={{ display: "flex", gap: 6, alignItems: "flex-start" }}>
          <input type="radio" checked={kind === "google"} onChange={() => setKind("google")} />
          <span>
            <b>Google Workspace / Gmail account</b>
            <br />
            <span className="cm-muted">You'll sign in with Google in your web browser.</span>
          </span>
        </label>
        {kind === "google" && (
          <div className="cm-form" style={{ marginLeft: 22 }}>
            <span className="cm-label">E-mail address:</span>
            <input className="cm-input" value={email} placeholder="you@yourcompany.com" onChange={(e) => setEmail(e.target.value)} />
          </div>
        )}
        {kind === "google" && !clientReady && (
          <div className="cm-info-bar" style={{ marginLeft: 22 }}>
            Import the Google OAuth client file first (Account Settings › Google sign-in).
          </div>
        )}
        <label style={{ display: "flex", gap: 6, alignItems: "flex-start" }}>
          <input type="radio" checked={kind === "demo"} onChange={() => setKind("demo")} />
          <span>
            <b>Demo accounts</b>
            <br />
            <span className="cm-muted">Two sample mailboxes with mail, meetings, contacts and tasks. Nothing leaves this computer.</span>
          </span>
        </label>
      </div>
    </Dialog>
  );
}

function ChangeAccountDialog({ account, onClose, onSave }: { account: Account; onClose: () => void; onSave: (name: string, color: string) => void }) {
  const [name, setName] = useState(account.displayName);
  const [color, setColor] = useState(account.color);
  return (
    <Dialog
      title={`Change Account — ${account.email}`}
      onClose={onClose}
      width={420}
      footer={
        <>
          <button className="cm-button default" onClick={() => onSave(name, color)}>
            OK
          </button>
          <button className="cm-button" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-form">
        <span className="cm-label">Your Name:</span>
        <input className="cm-input" value={name} onChange={(e) => setName(e.target.value)} />
        <span className="cm-label">E-mail Address:</span>
        <input className="cm-input" value={account.email} disabled />
        <span className="cm-label">Colour:</span>
        <div style={{ display: "flex", gap: 4 }}>
          {COLORS.map((c) => (
            <button
              key={c}
              title={c}
              onClick={() => setColor(c)}
              style={{ width: 22, height: 18, background: c, border: c === color ? "2px solid #000" : "1px solid #888", padding: 0 }}
            />
          ))}
        </div>
      </div>
    </Dialog>
  );
}
