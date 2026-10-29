// Tools › Options › AI: provider, model, API key (kept in Windows Credential Manager, never read
// back), Test, and the translation preferences (target language, "never translate" list).
import { useEffect, useState } from "react";
import { aiApi, type AiProvider, type AiSettingsDto } from "../../api/ai";
import { errorBox } from "../../ui";
import { setSetting, useSettings } from "../../api/settings";
import { TARGET_LANGUAGES } from "./lang";
import "./ai.css";

const MODELS: Record<AiProvider, { id: string; label: string }[]> = {
  gemini: [
    { id: "gemini-2.5-flash", label: "Gemini 2.5 Flash (fast, inexpensive)" },
    { id: "gemini-2.5-flash-lite", label: "Gemini 2.5 Flash-Lite (cheapest)" },
    { id: "gemini-2.5-pro", label: "Gemini 2.5 Pro (best quality, slower)" },
  ],
  claude: [
    { id: "claude-haiku-5-5", label: "Claude Haiku 5.5 (fast, inexpensive)" },
    { id: "claude-sonnet-5-5", label: "Claude Sonnet 5.5" },
    { id: "claude-opus-5-5", label: "Claude Opus 5.5 (best quality, slower)" },
  ],
};

const KEY_HELP: Record<AiProvider, string> = {
  gemini: "Create a key at aistudio.google.com › Get API key.",
  claude: "Create a key at console.anthropic.com › API Keys.",
};

export function AiSettings() {
  const [s, setS] = useState<AiSettingsDto | null>(null);
  const myLang = useSettings().values["ai.myTranslateLanguage"];
  const [provider, setProvider] = useState<AiProvider>("gemini");
  const [model, setModel] = useState("");
  const [key, setKey] = useState("");
  const [status, setStatus] = useState<{ ok: boolean | null; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  // the models the saved key can actually use (asked from the provider — names change over time)
  const [live, setLive] = useState<{ provider: AiProvider; models: { id: string; label: string }[]; recommended: string | null; checkedAt: number } | null>(null);

  /** Fetch the model list; if the saved model isn't offered any more, switch to the recommended one.
   *  `announce`: say how many there are (Update Models List). */
  const refreshModels = async (d: AiSettingsDto, announce = false) => {
    const has = d.provider === "gemini" ? d.hasGeminiKey : d.hasAnthropicKey;
    if (!has) return setLive(null);
    try {
      const l = await aiApi.listModels();
      setLive({ provider: d.provider, ...l });
      if (announce)
        setStatus({
          ok: true,
          text: `Models list updated: ${l.models.length} model${l.models.length === 1 ? "" : "s"} your key can use${l.recommended ? `; recommended: ${l.recommended}` : ""}.`,
        });
      if (l.recommended && (!d.model || !l.models.some((m) => m.id === d.model))) {
        const nd = await aiApi.setSettings({ model: l.recommended });
        setS(nd);
        setModel(nd.model);
        if (d.model) setStatus({ ok: null, text: `"${d.model}" isn't available for your key any more — switched to ${l.recommended}.` });
      }
    } catch (e) {
      setStatus({ ok: false, text: String((e as Error)?.message ?? e) });
    }
  };

  const adopt = (d: AiSettingsDto) => {
    setS(d);
    setProvider(d.provider);
    setModel(d.model);
  };
  useEffect(() => {
    aiApi
      .getSettings()
      .then((d) => {
        adopt(d);
        refreshModels(d);
      })
      .catch((e) => setStatus({ ok: false, text: String(e?.message ?? e) }));
  }, []);

  const hasKey = (p: AiProvider) => (p === "gemini" ? s?.hasGeminiKey : s?.hasAnthropicKey) ?? false;
  const dirty = !!s && (provider !== s.provider || model !== s.model || key.trim() !== "");
  const liveModels = live && live.provider === provider ? live.models : null;
  const models = liveModels ? liveModels.map((m) => ({ id: m.id, label: m.id === live!.recommended ? `${m.label} (recommended)` : m.label })) : MODELS[provider];
  const custom = model !== "" && !models.some((m) => m.id === model);

  const save = async () => {
    setBusy(true);
    try {
      const d = await aiApi.setSettings({ provider, model, ...(key.trim() ? { key: key.trim() } : {}) });
      adopt(d);
      setKey("");
      setStatus({ ok: null, text: "Saved." });
      await refreshModels(d);
    } catch (e) {
      await errorBox(e, "AI");
    } finally {
      setBusy(false);
    }
  };

  /** Test: check the (typed or saved) key against the provider's model list, pick a model it can use,
   *  run a tiny request — and if it works with a typed key, save everything right away. */
  const test = async () => {
    setBusy(true);
    setStatus({ ok: null, text: "Testing…" });
    try {
      const typed = key.trim();
      let useModel = model;
      if (typed || hasKey(provider)) {
        const l = await aiApi.listModels({ provider, ...(typed ? { key: typed } : {}) });
        setLive({ provider, ...l });
        if (l.recommended && (!useModel || !l.models.some((m) => m.id === useModel))) useModel = "";
      }
      const r = await aiApi.test({ provider, model: useModel || undefined, key: typed || undefined });
      if (r.ok && (typed || provider !== s?.provider || useModel !== model)) {
        const d = await aiApi.setSettings({ provider, model: useModel, ...(typed ? { key: typed } : {}) });
        adopt(d);
        setKey("");
        setStatus({ ok: true, text: `${r.message} Saved.` });
        return;
      }
      setStatus({ ok: r.ok, text: r.message });
    } catch (e) {
      setStatus({ ok: false, text: String((e as Error)?.message ?? e) });
    } finally {
      setBusy(false);
    }
  };

  const removeKey = async () => {
    setBusy(true);
    try {
      adopt(await aiApi.setSettings({ provider, key: "" }));
      setStatus({ ok: null, text: "The saved key was removed." });
    } catch (e) {
      await errorBox(e, "AI");
    } finally {
      setBusy(false);
    }
  };

  const setTranslate = async (p: { targetLanguage?: string; neverTranslate?: string[] }) => {
    try {
      adopt(await aiApi.setSettings(p));
    } catch (e) {
      errorBox(e, "AI");
    }
  };

  return (
    <div className="cm-ai-settings">
      <fieldset className="cm-fieldset">
        <legend>AI provider</legend>
        <div className="cm-form">
          <span className="cm-label">Provider:</span>
          <select
            className="cm-input"
            value={provider}
            onChange={(e) => {
              setProvider(e.target.value as AiProvider);
              setModel("");
              setKey("");
            }}
          >
            <option value="gemini">Google Gemini</option>
            <option value="claude">Anthropic Claude</option>
          </select>
          <span className="cm-label">Model:</span>
          <select className="cm-input" value={custom ? "__custom" : model} onChange={(e) => setModel(e.target.value === "__custom" ? model || " " : e.target.value)}>
            <option value="">Default — {(liveModels && live?.recommended) ?? s?.defaultModels[provider] ?? MODELS[provider][0].id}</option>
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
            <option value="__custom">Other…</option>
          </select>
          {/* the list as the provider has it now (it is also checked once a day by itself) */}
          <span />
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <button
              className="cm-button"
              disabled={busy || !s || !hasKey(provider) || provider !== s.provider}
              title={provider !== s?.provider ? "Save the provider first" : "Ask the provider which models your key can use now"}
              onClick={async () => {
                if (!s) return;
                setBusy(true);
                try {
                  await refreshModels(s, true);
                } finally {
                  setBusy(false);
                }
              }}
            >
              Update Models List
            </button>
            <span className="cm-muted">
              {liveModels && live?.checkedAt ? `Last checked ${new Date(live.checkedAt).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}` : ""}
            </span>
          </div>
          {custom && (
            <>
              <span className="cm-label">Model name:</span>
              <input className="cm-input" value={model.trim()} onChange={(e) => setModel(e.target.value)} spellCheck={false} />
            </>
          )}
          <span className="cm-label">API key:</span>
          <div style={{ display: "flex", gap: 6 }}>
            <input
              className="cm-input cm-fill"
              type="password"
              value={key}
              autoComplete="off"
              placeholder={hasKey(provider) ? "•••••••• (saved — type to replace)" : "paste your API key"}
              onChange={(e) => setKey(e.target.value)}
            />
            <button className="cm-button" disabled={busy} onClick={test}>
              Test
            </button>
            {hasKey(provider) && (
              <button className="cm-button" disabled={busy} onClick={removeKey}>
                Remove
              </button>
            )}
          </div>
        </div>
        <div style={{ display: "flex", gap: 6, marginTop: 8, alignItems: "center" }}>
          <span className="cm-muted cm-fill">{KEY_HELP[provider]}</span>
          <button className="cm-button default" disabled={busy || !dirty} onClick={save}>
            Save
          </button>
        </div>
        <div className={`cm-ai-status${status?.ok === true ? " ok" : status?.ok === false ? " bad" : ""}`}>
          {status?.text ??
            (!s
              ? "Loading…"
              : provider === s.provider && s.demo
                ? "Using the built-in demo translator (demo accounts are present and no API key is saved)."
                : hasKey(provider)
                  ? `Ready: ${provider === "gemini" ? "Google Gemini" : "Anthropic Claude"}, ${model.trim() || s.defaultModels[provider]}.`
                  : "Not set up: enter an API key.")}
        </div>
      </fieldset>
      <fieldset className="cm-fieldset" style={{ marginTop: 8 }}>
        <legend>Translation</legend>
        <div className="cm-form">
          <span className="cm-label">Translate messages into:</span>
          <select className="cm-input" style={{ width: 180 }} value={s?.targetLanguage ?? "English"} onChange={(e) => setTranslate({ targetLanguage: e.target.value })}>
            {[...new Set([...(s ? [s.targetLanguage] : []), ...TARGET_LANGUAGES])].map((l) => (
              <option key={l}>{l}</option>
            ))}
          </select>
          <span className="cm-label">Translate my messages to:</span>
          <select
            className="cm-input"
            style={{ width: 180 }}
            title="Used by Translate in a new message or a reply to English mail; a reply to another language translates into that language"
            value={myLang}
            onChange={(e) => setSetting("ai.myTranslateLanguage", e.target.value)}
          >
            {[...new Set([myLang, ...TARGET_LANGUAGES])].map((l) => (
              <option key={l}>{l}</option>
            ))}
          </select>
          <span className="cm-label">Never translate:</span>
          <div className="cm-ai-chips">
            {s?.neverTranslate.length ? (
              s.neverTranslate.map((l) => (
                <span key={l} className="cm-ai-chip">
                  {l}
                  <button title={`Offer to translate ${l} again`} onClick={() => setTranslate({ neverTranslate: s.neverTranslate.filter((x) => x !== l) })}>
                    ×
                  </button>
                </span>
              ))
            ) : (
              <span className="cm-muted">(none — add languages from the Translate bar)</span>
            )}
          </div>
        </div>
      </fieldset>
      <div style={{ display: "flex", gap: 6, marginTop: 8, alignItems: "center" }}>
        <span className="cm-fill">
          {s?.consentAt
            ? `AI features are on (you agreed to the AI notice on ${new Date(s.consentAt).toLocaleDateString(undefined, { dateStyle: "medium" })}).`
            : "AI features are off until you use one and agree to the AI notice."}
        </span>
        {s?.consentAt ? (
          <button className="cm-button" disabled={busy} onClick={() => aiApi.setConsent(false).then(adopt, (e) => errorBox(e, "AI"))}>
            Turn AI Off
          </button>
        ) : null}
      </div>
      <p className="cm-muted" style={{ marginBottom: 0 }}>
        Keys are kept in Windows Credential Manager. Text goes to the AI provider only when you use an AI command (Translate, Reply with AI, AI Review), straight from this
        computer; translations are cached on this computer.
      </p>
    </div>
  );
}

export default AiSettings;
