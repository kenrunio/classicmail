// Help › Keyboard Shortcuts (also what the "Type a question for help" box shows) and Help ›
// Privacy Policy (docs/privacy-policy.md, also used for the website).
import { alertBox } from "../ui";
import { markdownToHtml } from "../modules/mail/compose/markdown";
import policy from "../../docs/privacy-policy.md?raw";

const SHORTCUTS: [string, string][] = [
  ["Ctrl+1 / 2 / 3 / 4", "Mail / Calendar / Contacts / Tasks"],
  ["Ctrl+N", "New item in the current module"],
  ["Ctrl+Shift+M / A / Q / C / K", "New message / appointment / meeting / contact / task"],
  ["Ctrl+R, Ctrl+Shift+R, Ctrl+F", "Reply, Reply to All, Forward"],
  ["Ctrl+Enter or Alt+S", "Send (in a message)"],
  ["Ctrl+Q / Ctrl+U", "Mark read / unread"],
  ["Insert", "Flag for follow-up / mark a task complete"],
  ["Ctrl+Shift+V", "Move to folder"],
  ["Ctrl+Shift+B", "Address Book"],
  ["Ctrl+E", "Instant Search"],
  ["Ctrl+G", "Go to Date (Calendar)"],
  ["Ctrl+Alt+1 / 2 / 3 / 4", "Day / Work Week / Week / Month (Calendar)"],
  ["Alt+Left / Alt+Right", "Previous / next period (Calendar)"],
  ["F9", "Send/Receive now"],
  ["Alt+F1 / Alt+F2", "Navigation Pane / To-Do Bar"],
];

export function showShortcuts(question?: string) {
  alertBox(
    <div>
      {question ? <p style={{ marginTop: 0 }}>ClassicMail has no online help for “{question}”. These keyboard shortcuts work like in Outlook 2007:</p> : null}
      <table style={{ borderCollapse: "collapse", fontSize: 12 }}>
        <tbody>
          {SHORTCUTS.map(([k, v]) => (
            <tr key={k}>
              <td style={{ padding: "1px 12px 1px 0", whiteSpace: "nowrap", fontWeight: 600 }}>{k}</td>
              <td style={{ padding: "1px 0" }}>{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>,
    "Keyboard Shortcuts",
  );
}

export function showPrivacyPolicy() {
  alertBox(
    // our own document, bundled at build time
    <div
      className="cm-privacy-policy"
      style={{ maxWidth: 560, maxHeight: "60vh", overflowY: "auto", fontSize: 12 }}
      dangerouslySetInnerHTML={{ __html: markdownToHtml(policy) }}
    />,
    "Privacy Policy",
  );
}

export function showAbout(info: { version: string; dataDir: string }) {
  const built = new Date(`${__BUILD_DATE__}T12:00:00`).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
  alertBox(
    <div>
      <p style={{ marginTop: 0 }}>
        <b>ClassicMail {info.version}</b>, built {built}
        <br />A desktop mail and calendar client for Google Workspace and Gmail, in the classic Outlook style.
      </p>
      <p>
        © 2026 Duvz Software · Written by Kenny Green
        <br />
        Free and open source (Apache License 2.0)
        <br />
        Comments: info@duvz.com
        <br />
        <a href="#" onClick={(e) => (e.preventDefault(), showPrivacyPolicy())}>
          Privacy Policy
        </a>
      </p>
      <p style={{ marginBottom: 0 }}>
        Data folder:
        <br />
        {info.dataDir}
      </p>
    </div>,
    "About ClassicMail",
  );
}
