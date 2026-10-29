#!/usr/bin/env node
// Build the Windows installer: node scripts/release.mjs
// → release/ClassicMail_<version>_x64-setup.exe (NSIS, per-user install, no admin needed), and a
//   copy outside Google Drive to run (see below).
// If google-client.json exists in the project root it is embedded, so the installed app can sign
// in to Google without importing the client file on each computer.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const conf = JSON.parse(fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const env = { ...process.env };
const pathKey = Object.keys(env).find((k) => k.toLowerCase() === "path") ?? "PATH";
env[pathKey] = `${path.join(process.env.USERPROFILE, ".cargo", "bin")};${env[pathKey]}`;
const targetDir = env.CARGO_TARGET_DIR || "C:/dev/outlook_build/target";
// Rust keeps source paths in the program (for error messages): leave the builder's user folder
// and project folder out of them, so an installer you share doesn't carry your Windows user name
const remap = [
  [process.env.USERPROFILE, "~"],
  [root, "classicmail"],
].map(([from, to]) => `--remap-path-prefix=${from}=${to}`);
// (the encoded form: the folders can have spaces, as in "My Drive")
const flags = env.CARGO_ENCODED_RUSTFLAGS ? env.CARGO_ENCODED_RUSTFLAGS.split("\x1f") : (env.RUSTFLAGS ?? "").split(" ");
env.CARGO_ENCODED_RUSTFLAGS = [...flags.filter(Boolean), ...remap].join("\x1f");
delete env.RUSTFLAGS;

const clientFile = path.join(root, "google-client.json");
// Cargo re-reads the file only when it looks newer than the last build: one put back after a
// build without it (an installer to share) would otherwise be left out
if (fs.existsSync(clientFile)) fs.utimesSync(clientFile, new Date(), new Date());
console.log(`[release] building ClassicMail ${conf.version}${fs.existsSync(clientFile) ? " (with embedded Google client)" : ""}…`);
const r = spawnSync("npx", ["tauri", "build"], { cwd: root, env, stdio: "inherit", shell: true });
if (r.status !== 0) process.exit(r.status ?? 1);

const nsisDir = path.join(targetDir, "release", "bundle", "nsis");
const exe = fs.readdirSync(nsisDir).find((f) => f.endsWith("-setup.exe") && f.includes(conf.version));
if (!exe) throw new Error(`installer not found in ${nsisDir}`);
const outDir = path.join(root, "release");
fs.mkdirSync(outDir, { recursive: true });
fs.copyFileSync(path.join(nsisDir, exe), path.join(outDir, exe));
console.log(`[release] ${path.join(outDir, exe)} (${(fs.statSync(path.join(outDir, exe)).size / 1e6).toFixed(1)} MB)`);

// A copy outside Google Drive to run: an installer started from a Drive folder that is still
// uploading it fails with "NSIS Error: Error launching installer". CLASSICMAIL_INSTALLERS_DIR
// overrides where (default: next to the build output).
const localDir = env.CLASSICMAIL_INSTALLERS_DIR || path.join(path.dirname(targetDir), "installers");
fs.mkdirSync(localDir, { recursive: true });
fs.copyFileSync(path.join(nsisDir, exe), path.join(localDir, exe));
console.log(`[release] to install, run: ${path.join(localDir, exe)}`);
