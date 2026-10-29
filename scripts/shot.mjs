#!/usr/bin/env node
// Quick screenshot of a running dev app window:
//   node scripts/shot.mjs --slot 1 --window main --out shots/main.png [--wait 1500] [--eval "js"]
import { connect } from "./cdp.mjs";
import fs from "node:fs";
import path from "node:path";

const a = process.argv.slice(2);
const opt = (k, d) => (a.includes(`--${k}`) ? a[a.indexOf(`--${k}`) + 1] : d);
const slot = Number(opt("slot", 0));
const kind = opt("window", "main");
const out = opt("out", `shots/${kind}.png`);
const wait = Number(opt("wait", 800));
const js = opt("eval", null);

const app = await connect(slot);
try {
  const p = await app.page(kind);
  if (js) console.log(JSON.stringify(await p.evaluate(js), null, 2));
  await p.waitForTimeout(wait);
  fs.mkdirSync(path.dirname(out), { recursive: true });
  await p.screenshot({ path: out });
  console.log(`saved ${out}`);
} finally {
  await app.close();
}
