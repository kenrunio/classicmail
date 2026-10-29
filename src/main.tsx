import React, { Suspense, type ComponentType } from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import "./theme/office2007.css";
import "./ui/ui.css";
import "./shell/shell.css";
import { queryClient } from "./api/query";
import { onCmEvent, startEvents } from "./api/events";
import { MenuHost, MessageBoxHost, errorBox } from "./ui";
import { keyOf } from "./ui/keys";
import { setQuitPending } from "./ui/dialog";
import { loadWindow } from "./windows/registry";
import { NewItemKeys } from "./shell/newItems";
import { SPARE_KINDS } from "./api/windows";
import { errorForLog, rpc } from "./api/rpc";
import { applyUiFont } from "./ui/font";
import { installUndoKeys, setUndoFailureHandler } from "./api/undo";
import { installZoom } from "./ui/zoom";

const kind = new URLSearchParams(window.location.search).get("window") || "main";
document.documentElement.dataset.window = kind;
// Development/test builds are clearly marked so they are never mistaken for the installed app.
if (import.meta.env.DEV) {
  const badge = document.createElement("div");
  const preview = import.meta.env.VITE_CM_PREVIEW === "1";
  badge.textContent = preview ? "PREVIEW of the next version — your real mail" : "TEST BUILD — not your real ClassicMail";
  badge.style.cssText = `position:fixed;right:8px;bottom:26px;z-index:99999;background:${preview ? "#e07b00" : "#d63c34"};color:#fff;font:bold 11px Segoe UI;padding:3px 8px;border-radius:3px;pointer-events:none;opacity:.9`;
  document.addEventListener("DOMContentLoaded", () => document.body.appendChild(badge));
  if (document.readyState !== "loading") document.body.appendChild(badge);
}
startEvents();

// Ctrl+Z / Ctrl+Y: the app-wide undo history (src/api/undo.ts) in the main window and item windows.
// The main window reports results in its status bar; item windows show a failed undo in a box.
if (["main", "explorer", "message", "appointment", "contact", "task", "address-book"].includes(kind)) {
  installUndoKeys();
  if (kind !== "main" && kind !== "explorer") setUndoFailureHandler((m) => void errorBox(m));
}

// UI font (Tools › Options › Preferences › Appearance): optional custom family / size / weight.
// Remembered locally too, so a window doesn't flash the other font before the settings arrive. See applyUiFont in src/ui/font.ts.
try {
  applyUiFont(JSON.parse(localStorage.getItem("cm.uiFont") ?? "null"));
} catch {
  applyUiFont(null);
}
const loadFont = () =>
  rpc<Record<string, unknown>>("settings.getMany", { keys: ["app.fontFamily", "app.fontSize", "app.fontWeight"] })
    .then((v) => {
      const f = { family: v["app.fontFamily"], size: v["app.fontSize"], weight: v["app.fontWeight"] };
      applyUiFont(f);
      try {
        localStorage.setItem("cm.uiFont", JSON.stringify(f));
      } catch {
        /* storage unavailable */
      }
    })
    .catch(() => {});
loadFont();
onCmEvent((e) => {
  if (e.kind === "settings.changed" && (typeof e.key !== "string" || e.key.startsWith("app.font"))) loadFont();
});
// the app is quitting: a Cancel in this window's "save changes?" prompt must abort it (shell.rs)
import("@tauri-apps/api/event").then(({ listen }) => listen("cm-quit-start", () => setQuitPending(true))).catch(() => {});

window.addEventListener("error", (e) => {
  rpc("app.log", { level: "error", message: `[${kind}] ${e.message} at ${e.filename}:${e.lineno}` }).catch(() => {});
});
window.addEventListener("unhandledrejection", (e) => {
  rpc("app.log", { level: "error", message: `[${kind}] unhandled: ${errorForLog(e.reason)}` }).catch(() => {});
});
// Block the WebView's own context menu / reload shortcuts so it feels like a desktop app.
window.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement;
  if (!(t.closest("input, textarea, [contenteditable=true]") || import.meta.env.DEV)) e.preventDefault();
});
installZoom();
// A file dropped where nothing takes it would open in the window, replacing the app's page:
// refuse it (a place that takes files, like a message being written, has accepted it already).
for (const type of ["dragover", "drop"])
  window.addEventListener(type, (e) => {
    const de = e as DragEvent;
    if (!de.dataTransfer?.types.includes("Files") || de.defaultPrevented) return;
    e.preventDefault();
    de.dataTransfer.dropEffect = "none";
  });
window.addEventListener("keydown", (e) => {
  if ((e.key === "F5" || (e.ctrlKey && keyOf(e) === "r")) && !import.meta.env.DEV) e.preventDefault();
});

class ErrorBoundary extends React.Component<{ children: React.ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error) {
    rpc("app.log", { level: "error", message: `[${kind}] render: ${errorForLog(error, true)}` }).catch(() => {});
  }
  render() {
    if (this.state.error)
      return (
        <div style={{ padding: 20 }}>
          <div className="cm-error-bar">
            <b>Something went wrong in this window.</b>
            <pre style={{ whiteSpace: "pre-wrap", fontSize: 11 }}>{this.state.error.message}</pre>
            <button className="cm-button" onClick={() => location.reload()}>
              Reload
            </button>
          </div>
        </div>
      );
    return this.props.children;
  }
}

// item windows that take the New shortcuts (Ctrl+Shift+M and so on)
const ITEM_WINDOWS = new Set(["compose", "message", "appointment", "contact", "task", "address-book"]);

const render = (Win: ComponentType) =>
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <QueryClientProvider client={queryClient}>
        <ErrorBoundary>
          <Suspense fallback={<div className="cm-empty">Loading…</div>}>
            <Win />
          </Suspense>
        </ErrorBoundary>
        <MenuHost />
        <MessageBoxHost />
        {ITEM_WINDOWS.has(kind) && <NewItemKeys kind={kind} />}
      </QueryClientProvider>
    </React.StrictMode>,
  );

// A spare window (src-tauri/src/spare.rs) loads hidden and waits: when New / Reply claims it, it
// gets that window's query string and is shown fully transparent; it renders the item and is made
// opaque once that is drawn.
if (new URLSearchParams(window.location.search).get("spare") === "1") {
  void (async () => {
    const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const me = getCurrentWebviewWindow();
    const Win = await loadWindow(kind);
    await me.once<{ query: string; title: string }>("cm-spare-claim", ({ payload }) => {
      history.replaceState(null, "", `?${payload.query}`);
      document.title = payload.title;
      let shown = false;
      const reveal = () => {
        if (shown) return;
        shown = true;
        rpc("app.revealSpare", { label: me.label }).catch(() => void me.show());
      };
      // the next frame has the content in it
      window.addEventListener("cm-painted", () => requestAnimationFrame(() => setTimeout(reveal, 0)), { once: true });
      window.setTimeout(reveal, 3000); // whatever happens, don't stay invisible
      void me.setTitle(payload.title);
      render(Win);
    });
    await rpc("app.spareReady", { label: me.label });
  })();
} else {
  void loadWindow(kind).then(render);
  // the main window: have a compose window and a message window ready in the background (New /
  // Reply / opening an e-mail), one after the other
  if (kind === "main")
    window.setTimeout(async () => {
      for (const k of SPARE_KINDS) await rpc("app.prepareSpare", { kind: k }).catch(() => {});
    }, 4000);
}
