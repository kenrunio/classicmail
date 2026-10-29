// Shown in the main window when there are no accounts yet.
import { useState } from "react";
import { rpc } from "../api/rpc";
import { queryClient, useAppInfo } from "../api/query";
import { Icon, errorBox } from "../ui";

// the guide on GitHub (docs/google-setup.md), readable without the source
export const SETUP_GUIDE = "https://github.com/kenrunio/classicmail/blob/main/docs/google-setup.md";
export const openGuide = () => void rpc("app.openUrl", { url: SETUP_GUIDE }).catch(errorBox);

export function FirstRun() {
  const info = useAppInfo();
  const [busy, setBusy] = useState<string | null>(null);

  const importClient = async (file: File) => {
    try {
      await rpc("google.setClientJson", { json: await file.text() });
      info.refetch();
    } catch (e) {
      errorBox(e);
    }
  };
  const signIn = async (hint: string) => {
    setBusy(hint);
    try {
      await rpc("accounts.signInGoogle", { loginHint: hint });
      queryClient.invalidateQueries({ queryKey: ["accounts"] });
    } catch (e) {
      errorBox(e);
    } finally {
      setBusy(null);
    }
  };
  const demo = async () => {
    setBusy("demo");
    try {
      await rpc("accounts.addDemo", {});
      queryClient.invalidateQueries();
    } catch (e) {
      errorBox(e);
    } finally {
      setBusy(null);
    }
  };

  const configured = info.data?.googleClientConfigured;
  return (
    <div className="cm-firstrun">
      <div className="cm-firstrun-box">
        <div className="cm-firstrun-head">
          <Icon name="mail" size={40} />
          <div>
            <div className="cm-firstrun-title">Welcome to ClassicMail</div>
            <div className="cm-muted">Mail, calendar, contacts and tasks for your Google Workspace and Gmail accounts.</div>
          </div>
        </div>

        <fieldset className="cm-fieldset">
          <legend>Connect your Google accounts</legend>
          {!configured ? (
            <>
              <p style={{ marginTop: 0 }}>
                First import your Google sign-in client: the JSON file you downloaded for your <b>Desktop app</b> client in your own Google Cloud project. Don&apos;t have one yet?
                It&apos;s free and takes about 15 minutes, once: follow the{" "}
                <a className="cm-link" onClick={openGuide}>
                  step-by-step guide
                </a>{" "}
                (works for Google Workspace and @gmail.com accounts).
              </p>
              <label className="cm-button" style={{ display: "inline-flex", alignItems: "center" }}>
                Import client file…
                <input type="file" accept=".json,application/json" hidden onChange={(e) => e.target.files?.[0] && importClient(e.target.files[0])} />
              </label>
            </>
          ) : (
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
              <button className="cm-button default" disabled={busy !== null} onClick={() => signIn("")}>
                {busy === "" ? "Waiting for browser…" : "Sign in with Google…"}
              </button>
            </div>
          )}
        </fieldset>

        <fieldset className="cm-fieldset">
          <legend>Just looking?</legend>
          <p style={{ marginTop: 0 }}>Try ClassicMail with two demo mailboxes full of sample mail, meetings, contacts and tasks. Nothing is sent anywhere.</p>
          <button className="cm-button" disabled={!!busy} onClick={demo}>
            {busy === "demo" ? "Setting up…" : "Use demo accounts"}
          </button>
        </fieldset>
      </div>
    </div>
  );
}
