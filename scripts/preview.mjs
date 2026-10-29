#!/usr/bin/env node
// Preview the latest code on your REAL ClassicMail data (same accounts, mail, settings as the
// installed app) — no installer needed. Screen changes appear live; back-end changes restart it.
//   node scripts/preview.mjs        (or double-click "Preview ClassicMail.cmd")
// The installed ClassicMail must be closed first (they must not share the database at once).
import { spawn, execSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function installedAppRunning() {
  try {
    const out = execSync(
      `powershell -NoProfile -Command "@(Get-CimInstance Win32_Process -Filter \\"name='classicmail.exe'\\" | Where-Object { $_.ExecutablePath -like '*\\AppData\\Local\\ClassicMail\\*' }).Count"`,
      { encoding: "utf8" },
    );
    return Number(out.trim()) > 0;
  } catch {
    return false;
  }
}

if (installedAppRunning()) {
  console.log("\nClassicMail is open. Close it (File > Exit) and run the preview again.\n");
  process.exit(1);
}

const env = { ...process.env, VITE_CM_PREVIEW: "1" };
// real profile: no CLASSICMAIL_DATA_DIR / WEBVIEW2 overrides
delete env.CLASSICMAIL_DATA_DIR;
delete env.WEBVIEW2_USER_DATA_FOLDER;
delete env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;
const pathKey = Object.keys(env).find((k) => k.toLowerCase() === "path") ?? "PATH";
env[pathKey] = `${path.join(process.env.USERPROFILE, ".cargo", "bin")};${env[pathKey]}`;

console.log("[preview] starting ClassicMail (preview of the latest code) on your real data…");
const child = spawn("npx", ["tauri", "dev"], { cwd: root, env, stdio: "inherit", shell: true });
child.on("exit", (code) => process.exit(code ?? 0));
