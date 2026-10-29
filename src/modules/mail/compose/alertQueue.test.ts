import { describe, expect, it } from "vitest";
import { AlertQueue } from "./alertQueue";

/** A fake alert window: what it was opened with, what it was sent, what was logged. */
function fake(opts: { createFails?: boolean } = {}) {
  const log: string[] = [];
  const st = { exists: false, opened: [] as string[], sent: [] as string[], errors: log };
  const q = new AlertQueue<string>(
    {
      exists: async () => st.exists,
      create: async (b) => {
        await Promise.resolve();
        if (opts.createFails) throw new Error("label taken");
        st.exists = true;
        st.opened.push(b);
      },
      send: async (b) => void st.sent.push(b),
      log: (m) => void log.push(m),
    },
    async () => {
      st.exists = false; // the closing window is gone after the first wait
    },
  );
  return { q, st };
}

describe("AlertQueue", () => {
  it("opens the window once for two batches at once, and the second waits for ready", async () => {
    const { q, st } = fake();
    await Promise.all([q.show("a"), q.show("b")]);
    expect(st.opened).toEqual(["a"]);
    expect(st.sent).toEqual([]);
    await q.onReady();
    expect(st.sent).toEqual(["b"]);
    await q.show("c");
    expect(st.sent).toEqual(["b", "c"]);
  });

  it("holds batches while the window closes, then opens a new one with them", async () => {
    const { q, st } = fake();
    await q.show("a");
    await q.onReady();
    // "b" is already on its way when the window says it is closing
    const late = q.show("b");
    const closing = q.onClosing();
    await Promise.all([late, closing]);
    expect(st.sent).toEqual([]);
    expect(st.opened).toEqual(["a", "b"]);
  });

  it("opens again when the window went away without saying so", async () => {
    const { q, st } = fake();
    await q.show("a");
    await q.onReady();
    st.exists = false;
    await q.show("b");
    expect(st.opened).toEqual(["a", "b"]);
  });

  it("logs a window that can't be made and carries on", async () => {
    const { q, st } = fake({ createFails: true });
    await q.show("a");
    expect(st.errors[0]).toMatch(/couldn't be opened/);
    await q.show("b");
    expect(st.errors).toHaveLength(2);
  });
});
