// The one-time AI notice. The back end refuses to send the user's text to a real model until it
// has been accepted (`ai_consent`, src-tauri/src/ai/mod.rs `require_use`); every AI call goes
// through `withAiConsent`, which shows the notice on that error and, once accepted, retries.
import { messageBox } from "../ui/dialog";
import { rpc, RpcError } from "./rpc";

/** Thrown when the user declines the notice; error boxes stay quiet for it. */
export const AI_DECLINED = "ai_declined";

let asking: Promise<boolean> | null = null;

async function askOnce(): Promise<boolean> {
  const s = await rpc<{ provider: "gemini" | "claude" }>("ai.getSettings").catch(() => null);
  const who = s?.provider === "claude" ? "Anthropic's Claude" : "Google's Gemini";
  const answer = await messageBox({
    title: "Turn on AI features",
    icon: "info",
    buttons: ["Turn On AI", "Cancel"],
    defaultButton: "Turn On AI",
    text: (
      <div style={{ maxWidth: 440 }}>
        <p style={{ marginTop: 0 }}>
          ClassicMail&apos;s AI features (Translate, Reply with AI, AI Review) send the text you are working on to <b>{who}</b>, using your own API key.
        </p>
        <ul style={{ paddingLeft: 18 }}>
          <li>Only when you use an AI command: the message or draft it works on, and for AI Review the calendar or task details it looks up.</li>
          <li>It goes straight from this computer to {who}, never to ClassicMail or anyone else.</li>
          <li>
            With a <b>free</b> Gemini key, Google may use what it receives to improve its products. For private mail use a paid key, or Claude.
          </li>
        </ul>
        <p style={{ marginBottom: 0 }}>You are asked once. To turn AI off again: Tools › Options › AI.</p>
      </div>
    ),
  });
  if (answer !== "Turn On AI") return false;
  await rpc("ai.setConsent", { consent: true });
  return true;
}

/** Runs an AI call; on `ai_consent` shows the notice once (shared by parallel calls) and retries. */
export async function withAiConsent<T>(call: () => Promise<T>): Promise<T> {
  try {
    return await call();
  } catch (e) {
    if (!(e instanceof RpcError) || e.code !== "ai_consent") throw e;
    asking ??= askOnce().finally(() => (asking = null));
    if (!(await asking)) throw new RpcError(AI_DECLINED, "AI features are off.");
    return call();
  }
}
