// The new-Outlook style Translate bar for the Reading Pane and the message window:
// "This message is in Hebrew. Translate message to English | Never translate from Hebrew".
// Translates the rendered body in place (text nodes of the same-origin srcdoc iframe), keeping
// the formatting, plus the subject. "Show original" toggles back without another call; the back
// end caches translations, so reopening a message translates instantly and for free.
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { emit } from "@tauri-apps/api/event";
import { useRpc } from "../../api/query";
import { RpcError } from "../../api/rpc";
import { AI_DECLINED } from "../../api/aiConsent";
import { aiApi, type AiSettingsDto } from "../../api/ai";
import { openWindow } from "../../api/windows";
import { isRtl, messageLanguage, shouldOffer } from "./lang";
import { applyTranslation, batchTexts, collectTextNodes, worthTranslating, type Applied } from "./textNodes";
import "./ai.css";

export interface TranslatableMessage {
  id: number;
  subject: string;
  text: string;
  html: string;
}

type Mode = "idle" | "busy" | "translated" | "original" | "error";

export function openAiOptions() {
  openWindow("options", { tab: "ai" }, { label: "options", title: "Options" })
    .then(() => emit("cm-options-tab", "ai"))
    .catch(() => {});
}

function plainText(m: TranslatableMessage) {
  if (m.text?.trim()) return m.text;
  return m.html.replace(/<(style|script)[\s\S]*?<\/\1>/gi, " ").replace(/<[^>]+>/g, " ");
}

export function useAiSettings() {
  return useRpc<AiSettingsDto>(["ai", "settings"], "ai.getSettings", {}, { staleTime: 60_000 });
}

export function useTranslation(m: TranslatableMessage | undefined) {
  const settings = useAiSettings();
  const s = settings.data;
  const target = s?.targetLanguage ?? "English";
  // the language of the message's own opening, not of the signatures / disclaimers / quotes below
  const detected = useMemo(() => (m ? messageLanguage(plainText(m), m.subject).language : null), [m?.id, m?.text, m?.html, m?.subject]); // eslint-disable-line react-hooks/exhaustive-deps

  const [mode, setMode] = useState<Mode>("idle");
  const [error, setError] = useState<{ code: string; text: string } | null>(null);
  const [progress, setProgress] = useState(0);
  const [subject, setSubject] = useState<string | null>(null);
  const docRef = useRef<Document | null>(null);
  const appliedRef = useRef<Applied | null>(null);
  /** last translation of the current document (for "Translate" after "Show original") */
  const savedRef = useRef<{ doc: Document; nodes: Text[]; translations: string[]; subject: string | null } | null>(null);
  const modeRef = useRef<Mode>("idle");
  modeRef.current = mode;
  const runRef = useRef(0);

  // another message: start over
  useEffect(() => {
    runRef.current++;
    appliedRef.current = null;
    savedRef.current = null;
    setMode("idle");
    setError(null);
    setSubject(null);
  }, [m?.id]);

  const translate = useCallback(async () => {
    if (!m) return;
    const doc = docRef.current;
    const saved = savedRef.current;
    if (saved && doc && saved.doc === doc && saved.nodes.every((n) => n.isConnected)) {
      appliedRef.current = applyTranslation(doc, saved.nodes, saved.translations, isRtl(target));
      setSubject(saved.subject);
      setMode("translated");
      return;
    }
    const run = ++runRef.current;
    setMode("busy");
    setError(null);
    setProgress(0);
    try {
      const nodes = doc?.body ? collectTextNodes(doc.body) : [];
      const withSubject = worthTranslating(m.subject);
      const texts = [...(withSubject ? [m.subject] : []), ...nodes.map((n) => n.nodeValue ?? "")];
      const out: string[] = new Array(texts.length);
      const batches = batchTexts(texts);
      let done = 0;
      // two requests in flight; each is itself split / validated / cached by the back end
      const queue = [...batches];
      // a worker stops once the message changes or the other worker failed: no paid calls nobody sees
      const worker = async () => {
        for (let b = queue.shift(); b && run === runRef.current; b = queue.shift()) {
          const [a, z] = b;
          const r = await aiApi.translate(m.id, texts.slice(a, z), target).catch((e) => {
            queue.length = 0;
            throw e;
          });
          r.translations.forEach((t, i) => (out[a + i] = t));
          done += z - a;
          if (run === runRef.current) setProgress(texts.length ? done / texts.length : 1);
        }
      };
      await Promise.all([worker(), worker()]);
      if (run !== runRef.current) return; // the message changed meanwhile
      const subj = withSubject ? out[0] : null;
      const bodyTr = withSubject ? out.slice(1) : out;
      if (doc && docRef.current === doc) {
        appliedRef.current = applyTranslation(doc, nodes, bodyTr, isRtl(target));
        savedRef.current = { doc, nodes, translations: bodyTr, subject: subj };
      }
      setSubject(subj);
      setMode("translated");
    } catch (e) {
      if (run !== runRef.current) return;
      if (e instanceof RpcError && e.code === AI_DECLINED) return setMode("idle");
      setError({ code: e instanceof RpcError ? e.code : "internal", text: e instanceof Error ? e.message : String(e) });
      setMode("error");
    }
  }, [m, target]);

  const showOriginal = useCallback(() => {
    appliedRef.current?.restore();
    appliedRef.current = null;
    setSubject(null);
    setMode("original");
  }, []);

  /** MessageBody calls this whenever the body document (re)loads. */
  const onBodyLoad = useCallback(
    (doc: Document) => {
      docRef.current = doc;
      appliedRef.current = null;
      if (savedRef.current?.doc !== doc) savedRef.current = null;
      // e.g. "Download Pictures" re-rendered the body: translate again (served from the cache)
      if (modeRef.current === "translated") translate();
    },
    [translate],
  );

  const neverFrom = useCallback(() => {
    if (!detected || !s) return;
    aiApi.setSettings({ neverTranslate: [...s.neverTranslate, detected] }).catch(() => {});
  }, [detected, s]);

  let bar: ReactNode = null;
  const offer = !!m && !!s && shouldOffer(detected, target, s.neverTranslate);
  if (m && s && (offer || mode === "translated" || mode === "busy")) {
    const link = (label: string, onClick: () => void) => (
      <button type="button" className="cm-ai-link" onClick={onClick}>
        {label}
      </button>
    );
    let body: ReactNode;
    if (!s.configured)
      body = (
        <>
          <span>This message is in {detected}.</span>
          {link("Set up translation…", openAiOptions)}
        </>
      );
    else if (mode === "busy")
      body = (
        <span>
          Translating to {target}…{progress > 0 && progress < 1 ? ` ${Math.round(progress * 100)}%` : ""}
        </span>
      );
    else if (mode === "translated")
      body = (
        <>
          <span>
            Translated from {detected ?? "the original language"}
            {s.demo ? " by the demo translator" : ""}.
          </span>
          {link("Show original", showOriginal)}
        </>
      );
    else if (mode === "error")
      body = (
        <>
          <span className="cm-ai-error">Couldn't translate this message: {error?.text}</span>
          {error && ["ai_bad_key", "ai_not_configured", "ai_model"].includes(error.code) ? link("Set up translation…", openAiOptions) : link("Try again", translate)}
        </>
      );
    else
      body = (
        <>
          <span>This message is in {detected}.</span>
          {link(`Translate message to ${target}`, translate)}
          <span className="cm-ai-sep">|</span>
          {link(`Never translate from ${detected}`, neverFrom)}
        </>
      );
    bar = (
      <div className="cm-mail-infobar cm-ai-bar" data-testid="translate-bar">
        <TranslateIcon />
        {body}
      </div>
    );
  }

  return { bar, subject: mode === "translated" ? subject : null, onBodyLoad, mode, detected };
}

function TranslateIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="0.5" y="1.5" width="9" height="8" rx="1" fill="#fff" stroke="#3a6fc4" />
      <text x="5" y="8.2" textAnchor="middle" fontSize="6.5" fontWeight="700" fill="#3a6fc4" fontFamily="Segoe UI, sans-serif">
        A
      </text>
      <rect x="6.5" y="6.5" width="9" height="8" rx="1" fill="#3a6fc4" stroke="#2a559c" />
      <text x="11" y="13.2" textAnchor="middle" fontSize="6.5" fontWeight="700" fill="#fff" fontFamily="Segoe UI, sans-serif">
        文
      </text>
    </svg>
  );
}
