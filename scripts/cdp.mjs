// Drive the running dev app over the WebView2 DevTools protocol (see scripts/dev.mjs).
//   import { connect } from "./cdp.mjs";
//   const app = await connect(1);                  // slot
//   const main = await app.page("main");           // window kind from ?window=
//   await main.click("text=Calendar");
//   await main.screenshot({ path: "out.png" });
//   await app.rpc("mail.listFolders", {});          // call the back end directly
//   await app.close();
import { chromium } from "playwright-core";

export async function connect(slot = 0, { timeoutMs = 120000 } = {}) {
  const port = 9300 + Number(slot);
  const deadline = Date.now() + timeoutMs;
  let browser;
  for (;;) {
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
      break;
    } catch (e) {
      if (Date.now() > deadline) throw new Error(`app not reachable on CDP port ${port}: ${e.message}`, { cause: e });
      await new Promise((r) => setTimeout(r, 1000));
    }
  }
  const pages = () => browser.contexts().flatMap((c) => c.pages());
  async function page(kind = "main", { timeoutMs = 30000 } = {}) {
    const end = Date.now() + timeoutMs;
    for (;;) {
      const p = pages().find((pg) => {
        try {
          const u = new URL(pg.url());
          const ours = u.hostname === "localhost" || u.hostname === "tauri.localhost";
          return ours && (u.searchParams.get("window") || "main") === kind;
        } catch {
          return false;
        }
      });
      if (p) {
        await p.waitForLoadState("domcontentloaded").catch(() => {});
        return p;
      }
      if (Date.now() > end)
        throw new Error(
          `no window "${kind}" (have: ${pages()
            .map((p) => p.url())
            .join(", ")})`,
        );
      await new Promise((r) => setTimeout(r, 300));
    }
  }
  async function rpc(method, params = {}) {
    const p = await page("main");
    return p.evaluate(([m, pr]) => window.__cm.rpc(m, pr), [method, params]);
  }
  return { browser, pages, page, rpc, close: () => browser.close().catch(() => {}) };
}
