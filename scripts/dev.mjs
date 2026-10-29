#!/usr/bin/env node
// Run the app in dev mode in an isolated "slot" so several instances (e.g. parallel worktrees)
// can run at once without clashing:
//   node scripts/dev.mjs --slot 1          (slot 0 = defaults)
// Each slot gets its own Vite port (1420+10N), WebView2 remote-debugging port (9300+N),
// data dir (C:/dev/outlook_build/devdata/slotN — outside Google Drive) and, for N>0, its own cargo target dir.
// Extra args after `--` are passed to `tauri dev`.
import { spawn } from "node:child_process";
import path from "node:path";
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const slotIdx = args.indexOf("--slot");
const slot = slotIdx >= 0 ? Number(args[slotIdx + 1]) : 0;
const dd = args.indexOf("--");
const extra = dd >= 0 ? args.slice(dd + 1) : [];

const vitePort = 1420 + slot * 10;
const cdpPort = 9300 + slot;
const dataDir = path.join(process.env.CLASSICMAIL_DEVDATA_ROOT || "C:/dev/outlook_build/devdata", `slot${slot}`);
fs.mkdirSync(dataDir, { recursive: true });

const env = {
  ...process.env,
  CLASSICMAIL_VITE_PORT: String(vitePort),
  CLASSICMAIL_DATA_DIR: dataDir,
  // each slot needs its own WebView2 profile, or a second instance fails with 0x8007139F
  WEBVIEW2_USER_DATA_FOLDER: path.join(dataDir, "webview2"),
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${cdpPort}`,
};
// Windows env keys are case-insensitive but a spread object isn't: update the existing Path key.
const pathKey = Object.keys(env).find((k) => k.toLowerCase() === "path") ?? "PATH";
env[pathKey] = `${path.join(process.env.USERPROFILE, ".cargo", "bin")};${env[pathKey]}`;
if (slot > 0 && !process.env.CARGO_TARGET_DIR) env.CARGO_TARGET_DIR = `C:/dev/outlook_build/target-s${slot}`;

const config = path.join(dataDir, "tauri.dev.json");
fs.writeFileSync(config, JSON.stringify({ build: { devUrl: `http://localhost:${vitePort}` } }));
console.log(`[dev] slot ${slot}: vite ${vitePort}, cdp ${cdpPort}, data ${dataDir}`);
const child = spawn("npx", ["tauri", "dev", "--config", JSON.stringify(config), ...extra], { cwd: root, env, stdio: "inherit", shell: true });
child.on("exit", (code) => process.exit(code ?? 0));
