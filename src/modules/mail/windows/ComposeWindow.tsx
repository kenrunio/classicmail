// Outlook 2007 "Untitled - Message (HTML)" window.
// Params: mode=new|reply|replyAll|forward|draft, id (local message id; draft id for mode=draft),
// accountId, to, subject, outboxId (open an Outbox item: it goes back to Drafts first),
// merge (path of a mail-merge list: one message per row, see ../merge.ts).
import { useCallback, useEffect, useMemo, useRef, useState, type ComponentProps, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Ribbon, type RibbonTab, type MenuItem, keyOf, alertBox, errorBox, messageBox, contextMenu, showMenu, useHotkeys, usePersistentState } from "../../../ui";
import { rpc } from "../../../api/rpc";
import { queryClient } from "../../../api/query";
import { markPainted, windowParams, setWindowTitle } from "../../../api/windows";
import {
  composeApi,
  formatAddr,
  formatSize,
  isValidEmail,
  parseAddresses,
  suggestRecipients,
  type Addr,
  type AttachmentSpec,
  type Compose,
  type ComposeMode,
  type Importance,
  type Prepared,
  type Suggestion,
} from "../../../api/mailCompose";
import { SelectNamesDialog } from "../../contacts/SelectNames";
import { openWithPrompt } from "../actions";
import { mergeApi, type MergeMode, type MergeSummary } from "../../../api/mailMerge";
import { pickMergeList, preflightText } from "../merge";
import { aiComposeApi } from "../../../api/aiCompose";
import { canToggle, instructions, showsReplyAll, splitSubject, toggleReply } from "../compose/aiHelpers";
import { mailApi } from "../../../api/mail";
import { braceUnits, maskedHtml, unitsToReview, unmask } from "../compose/quotedInstructions";
import { withoutImages } from "../compose/aiImages";
import { companiesWarning } from "../compose/domainCheck";
import { useUsualGroup } from "../compose/usualGroup";
import { directed, textForAi } from "../compose/aiText";
import { inlinePageScript } from "../compose/webPage";
import { htmlToMarkdown, looksLikeMarkdown, markdownToHtml } from "../compose/markdown";
import { BodyTabs } from "../BodyTabs";
import { prettyHtml } from "../MessageSource";
import { detectLanguage, messageLanguage } from "../../ai/lang";
import { useAiSettings } from "../../ai/useTranslation";
import { useSettings } from "../../../api/settings";
import { useBodyZoom } from "../../../ui/zoom";
import { RecipientField, toRecipients, type Recipient, type RecipientFieldHandle } from "../compose/RecipientField";
import { clickSelect } from "../addrClip";
import { FONTS, FONT_SIZES, htmlToText, textToHtml, escapeHtml } from "../compose/editor";
import {
  changeUndoably,
  cleanPastedHtml,
  finalizeBody,
  keepSelection,
  listBlockHtml,
  nextCell,
  normalizeLoaded,
  originalOf,
  replaceUndoably,
  replyBlocks,
  replyRange,
} from "../compose/body";
import { attachTypingFixes } from "../compose/typingFixes";
import { Big, Small, SmallSplit, Row, Stack, Check } from "../compose/parts";
import { CIcon, fileIcon } from "../compose/icons";
import {
  CheckNamesDialog,
  ColorPalette,
  HyperlinkDialog,
  MessageOptionsDialog,
  PropertiesDialog,
  SymbolDialog,
  SYMBOLS,
  TableGrid,
  WebPageDialog,
  type MessageOptions,
  type PaletteKind,
} from "../compose/dialogs";
import { paintedHtml, paintFromDom, paintRange, type Paint } from "../compose/painter";
import * as qt from "../compose/pageTables";
import { attachResizer, measureEven, type ResizeTarget } from "../compose/tableSize";
import { domFormats } from "../compose/formatState";
import { resetSelState, trackSelection, useSelState, type Command, type SelState } from "../compose/selState";
import { SignaturesDialog, DEFAULT_COMPOSE_FONT, COMPOSE_FONT_KEY, type ComposeFont } from "../compose/SignaturesDialog";
import "../compose/compose.css";

const AUTOSAVE_MS = 3 * 60_000;

export default function ComposeWindow() {
  const [state, setState] = useState<{ prep: Prepared; font: ComposeFont; merge: MergeInfo | null } | { error: string } | null>(null);
  useEffect(() => {
    const p = windowParams();
    const mode = (p.get("mode") ?? "new") as ComposeMode;
    const num = (k: string) => (p.get(k) ? Number(p.get(k)) : undefined);
    (async () => {
      const font = await rpc<ComposeFont | null>("settings.get", { key: COMPOSE_FONT_KEY }).catch(() => null);
      let prep: Prepared;
      const outboxId = num("outboxId");
      if (outboxId) {
        const r = await composeApi.outboxCancel(outboxId);
        prep = await composeApi.prepare("draft", r.messageId);
      } else if (num("archive")) {
        // Reply / Forward from an Outlook archive (read-only source)
        const { archiveApi } = await import("../../../api/archive");
        prep = await archiveApi.prepareCompose(num("archive")!, mode as "reply" | "replyAll" | "forward", num("accountId"));
      } else {
        prep = await composeApi.prepare(mode, num("id"), num("accountId"));
      }
      if (mode === "new") {
        const to = p.get("to");
        if (to) prep.compose.to = parseAddresses(to);
        const subject = p.get("subject");
        if (subject) prep.compose.subject = subject;
        // Word / Excel's "Send as Attachment" (ClassicMail as the mail client: src-tauri/src/mapi.rs)
        const mapiDir = p.get("mapi");
        if (mapiDir) {
          const m = await composeApi.readMapi(mapiDir);
          Object.assign(prep.compose, { to: m.to, cc: m.cc, bcc: m.bcc });
          if (m.subject) prep.compose.subject = m.subject;
          prep.compose.attachments = [...prep.compose.attachments, ...m.attachments.map(({ path, name, size, mimeType }) => ({ path, name, size, mimeType }))];
          if (m.body.trim()) prep.bodyHtml = textToHtml(m.body.replace(/\r\n?/g, "\n").trim()) + prep.bodyHtml;
        }
        // Reply / Reply All / Forward of a calendar item: its people and details
        const eventId = num("event");
        if (eventId) {
          const [{ cal }, { itemMail }] = await Promise.all([import("../../../api/calendar"), import("../../calendar/itemMail")]);
          const m = itemMail(await cal.getEvent(eventId), p.get("eventMode") === "reply" ? "reply" : "forward");
          Object.assign(prep.compose, { to: m.to, cc: m.cc, subject: m.subject });
          prep.quotedHtml = m.quotedHtml;
          prep.replyTargets = m.targets;
        }
      }
      const mergePath = p.get("merge");
      const merge = mergePath ? { path: mergePath, summary: await mergeApi.load(mergePath) } : null;
      setState({ prep, font: { ...DEFAULT_COMPOSE_FONT, ...(font ?? {}) }, merge });
    })().catch((e) => setState({ error: e instanceof Error ? e.message : String(e) }));
  }, []);
  // a spare window shows itself now that the message (or the error) is drawn
  useEffect(() => {
    if (state) markPainted();
  }, [state]);
  if (!state) return <div className="cm-empty">Loading…</div>;
  if ("error" in state)
    return (
      <div style={{ padding: 20 }}>
        <div className="cm-error-bar">
          <b>The message could not be opened.</b>
          <div>{state.error}</div>
          <button className="cm-button" onClick={() => getCurrentWindow().destroy()}>
            Close
          </button>
        </div>
      </div>
    );
  return <ComposeForm prep={state.prep} font={state.font} merge0={state.merge} />;
}

type MergeInfo = { path: string; summary: MergeSummary };

interface AttachItem {
  key: string;
  spec: AttachmentSpec;
  name: string;
  size: number;
  mimeType: string;
}

/** A caret at the start of `page`. */
function pageStart(page: HTMLElement): Range {
  const r = document.createRange();
  r.setStart(page, 0);
  return r;
}

/** Make `r` (none: a caret at the start) the selection on `page`, with the focus. */
function select(page: HTMLElement, r?: Range | null) {
  page.focus();
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(r ?? pageStart(page));
}

let attachSeq = 0;
function toAttachItem(spec: AttachmentSpec): AttachItem {
  return {
    key: `a${++attachSeq}`,
    spec,
    name: spec.name ?? (spec.path ? spec.path.split(/[\\/]/).pop()! : "attachment"),
    size: spec.size ?? 0,
    mimeType: spec.mimeType ?? "",
  };
}

/** A part of the ribbon that shows the selection (selState.ts): only it redraws when the caret moves. */
function OnSel({ children }: { children: (s: SelState) => ReactNode }) {
  return <>{children(useSelState())}</>;
}

/** An alignment / list button, pressed when the browser's command is on at the selection. */
function CmdButton({ cmd, off, ...p }: ComponentProps<typeof Small> & { cmd: Command; off?: boolean }) {
  const on = useSelState((s) => !!s.on[cmd]);
  return <Small {...p} pressed={!off && on} />;
}

/** Bold / Italic / Underline / Strikethrough, pressed when all of the selection has it. */
function FmtButton({ mark, off, ...p }: ComponentProps<typeof Small> & { mark: "bold" | "italic" | "underline" | "strike"; off?: boolean }) {
  const on = useSelState((s) => !!s.fmt?.[mark]);
  return <Small {...p} pressed={!off && on} />;
}

function ComposeForm({ prep, font, merge0 }: { prep: Prepared; font: ComposeFont; merge0: MergeInfo | null }) {
  const c0 = prep.compose;
  // mail merge: no To/Cc/Bcc; one message per list row ({Column} placeholders filled in)
  const [merge, setMerge] = useState<MergeInfo | null>(merge0);
  const lastField = useRef<"subject" | "body">("body");
  const sendRef = useRef<HTMLButtonElement>(null);
  // Reply to One / Reply All toggle (replies only)
  const [replyAllOn, setReplyAllOn] = useState(() => showsReplyAll({ to: c0.to, cc: c0.cc }));
  // AI: translate my reply / AI Review / Reply with AI
  const aiReady = !!useAiSettings().data?.configured;
  const [aiBusy, setAiBusy] = useState<null | "translate" | "review" | "draft">(null);
  /** the text before translating (`source` HTML in `sourceLang`) and the translation as it was put
   *  in; the subject likewise */
  const [translation, setTranslation] = useState<{
    source: string;
    sourceLang: string;
    translated: string;
    lang: string;
    subject: { source: string; translated: string };
  } | null>(null);
  const myLang = useSettings().values["ai.myTranslateLanguage"];
  // message zoom (view only: what is sent keeps its real size)
  const zoom = useBodyZoom();
  /** the language of the message being answered, when it isn't English (Translate to …) */
  const origLang = useMemo(() => {
    if (!prep.quotedHtml) return null;
    const l = messageLanguage(htmlToText(prep.quotedHtml)).language;
    return l && l !== "English" ? l : null;
  }, [prep.quotedHtml]);
  const [accountId, setAccountId] = useState(c0.accountId);
  const [from, setFrom] = useState<string | null>(c0.from ?? null);
  const [to, setTo] = useState<Recipient[]>(toRecipients(c0.to));
  const [cc, setCc] = useState<Recipient[]>(toRecipients(c0.cc));
  const [bcc, setBcc] = useState<Recipient[]>(toRecipients(c0.bcc));
  const [subject, setSubject] = useState(c0.subject);
  const [attachments, setAttachments] = useState<AttachItem[]>(() => c0.attachments.filter((a) => !a.cid).map(toAttachItem));
  // selected attachments (keys); click, Ctrl/Shift+click and Ctrl+A select, Delete removes them
  const [selAttach, setSelAttach] = useState<string[]>([]);
  const attAnchor = useRef<number | null>(null);
  const [opts, setOpts] = useState<MessageOptions>({
    importance: c0.importance ?? "normal",
    requestReadReceipt: !!c0.requestReadReceipt,
    requestDeliveryReceipt: !!c0.requestDeliveryReceipt,
    deferUntil: c0.deferUntil ?? null,
    replyTo: c0.replyTo ?? [],
    flagForRecipients: c0.flagForRecipients ?? null,
  });
  const [showBccPref, setShowBccPref] = usePersistentState("compose.showBcc", false);
  const [showFromPref, setShowFromPref] = usePersistentState("compose.showFrom", prep.fromChoices.length > 1);
  const showBcc = showBccPref || bcc.length > 0;
  const showFrom = showFromPref;
  const [plainText, setPlainText] = useState(!!c0.plainText);
  // Format › Markdown: written in the text box (with plainText), sent as HTML made from it
  const [markdown, setMarkdown] = useState(false);
  // the tabs above the text: Message / HTML Source (View Source), or Markdown / Preview
  const [srcTabs, setSrcTabs] = useState(false);
  const [bodyTab, setBodyTab] = useState<"main" | "alt">("main");
  const [srcText, setSrcText] = useState("");
  const srcStart = useRef("");
  // Format Text › View Gridlines (remembered on this computer)
  const [gridlines, setGridlines] = useState(() => {
    try {
      return localStorage.getItem("cm.compose.gridlines") !== "0";
    } catch {
      return true;
    }
  });
  const [plainValue, setPlainValue] = useState(() => (c0.plainText ? htmlToText(prep.bodyHtml + prep.quotedHtml) : ""));
  const [spell, setSpell] = usePersistentState("compose.spellcheck", true);
  const [busy, setBusy] = useState(false);
  const [info, setInfo] = useState<string | null>(null);
  const [dropping, setDropping] = useState(false);

  // dialogs / pop-ups
  const [optionsOpen, setOptionsOpen] = useState(false);
  // Insert Hyperlink: the selected text (or the link at the cursor) when it opened
  const [link, setLink] = useState<{ text: string; a: HTMLAnchorElement | null; selected: boolean } | null>(null);
  // Include › Web Page
  const [webPageOpen, setWebPageOpen] = useState(false);
  const [tableAnchor, setTableAnchor] = useState<DOMRect | null>(null);
  const [symbolOpen, setSymbolOpen] = useState(false);
  const [palette, setPalette] = useState<{ kind: PaletteKind; rect: DOMRect } | null>(null);
  // Format Text › Table: last shading colour, the border pen
  const [shadeColor, setShadeColor] = useState<string | null>("#D9E2F3");
  const [pen, setPen] = useState<qt.Line>({ width: 1, color: "#000000" });
  const [selectNames, setSelectNames] = useState(false);
  const [sigOpen, setSigOpen] = useState(false);
  const [propsOpen, setPropsOpen] = useState(false);
  const [checkNames, setCheckNames] = useState<{ name: string; candidates: Suggestion[]; resolve: (s: Suggestion | null) => void } | null>(null);
  const [fontColor, setFontColor] = useState<string | null>("#C00000");
  const [hiColor, setHiColor] = useState<string | null>("#FFFF00");

  // save state
  const draftId = useRef<string | null>(c0.draftId ?? null);
  const openedFromDraft = useRef(!!prep.draftMessageId);
  const savedExplicitly = useRef(!!prep.draftMessageId);
  const dirty = useRef(false); // since last explicit save
  const autoDirty = useRef(false); // since last save of any kind
  const saving = useRef<Promise<void> | null>(null);
  const closing = useRef(false);
  const markDirty = useCallback(() => {
    dirty.current = true;
    autoDirty.current = true;
  }, []);

  const toRef = useRef<RecipientFieldHandle>(null);
  const ccRef = useRef<RecipientFieldHandle>(null);
  const bccRef = useRef<RecipientFieldHandle>(null);
  // Tab after a name in To: the people usually with them (usualGroup.ts)
  const group = useUsualGroup({ to, setTo, cc, setCc, bcc, next: () => ccRef.current?.focus() });
  const subjectRef = useRef<HTMLInputElement>(null);
  // the one editable page: your reply, then the original (`#cm-original`) below it
  const pageRef = useRef<HTMLDivElement>(null);
  // what the page loads: the message, or the text coming back from Plain Text
  const loadHtml = useRef(prep.bodyHtml + prep.quotedHtml);
  const painter = useRef<Paint | null>(null);
  // the last selection on the page: the ribbon's font / size boxes take the focus when clicked,
  // and their change applies there (what the ribbon shows about it: selState.ts)
  const savedSel = useRef<Range | null>(null);
  const [painterOn, setPainterOn] = useState(false);

  /** The focus back on the page when a ribbon box, a dialog or another field has it: at the saved
   *  selection, or at the start when the page never had one. */
  const restoreSel = useCallback(() => {
    const page = pageRef.current;
    if (page && !page.contains(document.activeElement)) select(page, savedSel.current);
  }, []);
  /** A browser editing command on the page (see `restoreSel`). */
  const exec = useCallback(
    (cmd: string, arg?: string) => {
      restoreSel();
      document.execCommand("styleWithCSS", false, "true");
      document.execCommand(cmd, false, arg);
      markDirty();
    },
    [markDirty, restoreSel],
  );
  const imgHtml = (src: string, alt: string) => `<img src="${escapeHtml(src)}" alt="${escapeHtml(alt)}">`;
  const insertImageFiles = useCallback(
    (files: File[]) => {
      for (const f of files) {
        const r = new FileReader();
        r.onload = () => exec("insertHTML", imgHtml(String(r.result), f.name));
        r.readAsDataURL(f);
      }
    },
    [exec],
  );

  // the page's content, set once (and again coming back from Plain Text); new lines are <p>
  useEffect(() => {
    const page = pageRef.current;
    if (!page) return;
    page.innerHTML = normalizeLoaded(loadHtml.current);
    savedSel.current = null;
    resetSelState();
    document.execCommand("defaultParagraphSeparator", false, "p");
    return attachTypingFixes(page, { replyOnly: (b) => !originalOf(page)?.contains(b) });
  }, [plainText]);

  // cursor at the top like Outlook: To for new mail, body for replies
  useEffect(() => {
    const t = setTimeout(() => {
      if (c0.mode === "reply" || c0.mode === "replyAll") {
        if (pageRef.current) select(pageRef.current);
      } else if (merge) subjectRef.current?.focus();
      else toRef.current?.focus();
    }, 50);
    return () => clearTimeout(t);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // title
  useEffect(() => {
    setWindowTitle(`${merge ? "Mail Merge: " : ""}${subject.trim() || "Untitled"} - Message (${markdown ? "Markdown" : plainText ? "Plain Text" : "HTML"})`);
  }, [subject, plainText, markdown, merge]);

  // ---- building the compose ---------------------------------------------------------------
  // a usual group's people nobody kept (usualGroup.ts) never go out or into the draft
  const strip = (v: Recipient[]): Addr[] => v.filter((r) => !r.suggested).map(({ name, email }) => ({ name, email }));
  const buildCompose = useCallback((): Compose => {
    const t = toRef.current?.commit() ?? to;
    const c = ccRef.current?.commit() ?? cc;
    const b = bccRef.current?.commit() ?? bcc;
    // Markdown goes out as the HTML made from it (its text is the plain version); the HTML Source
    // tab, while open, is what is sent
    const pageHtml = srcTabs && bodyTab === "alt" ? srcText : (pageRef.current?.innerHTML ?? "");
    const html = markdown
      ? finalizeBody(markdownToHtml(plainValue), font)
      : plainText
        ? `<div style="font-family:'Courier New',monospace;font-size:10pt">${textToHtml(plainValue)}</div>`
        : finalizeBody(pageHtml, font);
    return {
      accountId,
      from,
      to: strip(t),
      cc: strip(c),
      bcc: strip(b),
      subject,
      html,
      text: plainText ? plainValue : null,
      plainText: plainText && !markdown,
      attachments: attachments.map((a) => a.spec),
      importance: opts.importance,
      requestReadReceipt: opts.requestReadReceipt,
      requestDeliveryReceipt: opts.requestDeliveryReceipt,
      deferUntil: opts.deferUntil,
      replyTo: opts.replyTo,
      flagForRecipients: opts.flagForRecipients,
      mode: c0.mode === "draft" ? "new" : c0.mode,
      sourceMessageId: c0.sourceMessageId ?? null,
      draftId: draftId.current,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId, from, to, cc, bcc, subject, plainText, markdown, srcTabs, bodyTab, srcText, plainValue, attachments, opts, font]);

  /** What you wrote (the page above the original): its text (without the zero-width space of a
   *  size picked at the caret), and whether it holds a picture. */
  const yourPart = () => {
    const blocks = pageRef.current ? replyBlocks(pageRef.current) : [];
    const text = blocks.map((b) => b.textContent ?? "").join("\n");
    return { text: text.replace(/\u200b/g, ""), picture: blocks.some((b) => b.matches("img") || !!b.querySelector("img")) };
  };
  const hasContent = () => to.length + cc.length + bcc.length > 0 || subject.trim() !== "" || attachments.length > 0 || yourPart().text.trim() !== "" || plainValue.trim() !== "";

  const saveDraft = useCallback(
    async (explicit: boolean) => {
      if (merge) {
        // a template full of {fields} is not a draft to anyone
        if (explicit) setInfo("A mail merge is not saved as one draft. Use Send ▸ Save All as Drafts to create one draft per recipient.");
        return;
      }
      if (saving.current) await saving.current.catch(() => {});
      const c = buildCompose();
      const p = (async () => {
        const r = await composeApi.saveDraft(c);
        draftId.current = r.draftId;
        autoDirty.current = false;
        if (explicit) {
          dirty.current = false;
          savedExplicitly.current = true;
        }
        const t = new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
        setInfo(explicit ? `Saved to Drafts at ${t}.` : `Autosaved to Drafts at ${t}.`);
      })();
      saving.current = p;
      try {
        await p;
      } finally {
        if (saving.current === p) saving.current = null;
      }
    },
    [buildCompose, merge],
  );

  // AutoSave every 3 minutes (Outlook's default). One timer for the window's life: the form
  // re-renders on every keystroke, and restarting it each time would never let it fire.
  const autoSave = useRef(() => {});
  autoSave.current = () => {
    if (autoDirty.current && !closing.current && !merge && hasContent()) {
      saveDraft(false).catch((e) => setInfo(`AutoSave failed: ${e instanceof Error ? e.message : e}`));
    }
  };
  useEffect(() => {
    const t = setInterval(() => autoSave.current(), AUTOSAVE_MS);
    return () => clearInterval(t);
  }, []);

  // ---- check names ------------------------------------------------------------------------
  const resolveList = async (list: Recipient[]): Promise<{ list: Recipient[]; ok: boolean }> => {
    const out: Recipient[] = [];
    let ok = true;
    for (const r of list) {
      if (r.resolved || isValidEmail(r.email)) {
        out.push({ ...r, resolved: true });
        continue;
      }
      const q = (r.name || r.email).trim();
      const cands = await suggestRecipients(q, 20);
      const exact = cands.filter((c) => c.name.toLowerCase() === q.toLowerCase() || c.email.toLowerCase() === q.toLowerCase());
      const pool = exact.length ? exact : cands;
      if (pool.length === 1) {
        out.push({ name: pool[0].name, email: pool[0].email, resolved: true });
      } else if (pool.length > 1) {
        const pick = await new Promise<Suggestion | null>((resolve) => setCheckNames({ name: q, candidates: pool, resolve }));
        if (pick) out.push({ name: pick.name, email: pick.email, resolved: true });
        else {
          out.push(r);
          ok = false;
        }
      } else {
        await alertBox(`ClassicMail does not recognize "${q}".`, "Check Names", "warning");
        out.push(r);
        ok = false;
      }
    }
    return { list: out, ok };
  };

  const runCheckNames = async (): Promise<{ ok: boolean; to: Recipient[]; cc: Recipient[]; bcc: Recipient[] }> => {
    group.drop();
    const kept = (v: Recipient[]) => v.filter((r) => !r.suggested);
    const t = await resolveList(kept(toRef.current?.commit() ?? to));
    const c = await resolveList(kept(ccRef.current?.commit() ?? cc));
    const b = await resolveList(bccRef.current?.commit() ?? bcc);
    setTo(t.list);
    setCc(c.list);
    setBcc(b.list);
    return { ok: t.ok && c.ok && b.ok, to: t.list, cc: c.list, bcc: b.list };
  };

  // ---- send / close -------------------------------------------------------------------------
  const destroy = () => getCurrentWindow().destroy();

  // ---- Reply to One / Reply All -------------------------------------------------------------
  const toggleRecipients = () => {
    const t = prep.replyTargets;
    if (!t) return;
    const cur = { to: toRef.current?.commit() ?? to, cc: ccRef.current?.commit() ?? cc };
    const next = toggleReply(cur, t, replyAllOn, (a) => ({ name: a.name, email: a.email, resolved: true }));
    setTo(next.to);
    setCc(next.cc);
    setReplyAllOn(!replyAllOn);
    markDirty();
  };

  // ---- AI: the reply part of the page (above the signature) ---------------------------------
  const signatureHtmlFor = async (acct: number) => {
    const list = await composeApi.signatures().catch(() => null);
    if (!list) return "";
    const reply = c0.mode === "reply" || c0.mode === "replyAll" || c0.mode === "forward";
    const d = list.defaults.find((x) => x.accountId === acct);
    const id = d ? (reply ? d.replySignatureId : d.newSignatureId) : null;
    return list.signatures.find((x) => x.id === id)?.html ?? "";
  };

  /** The user's own text and its HTML: before the signature (and the blank lines above it), or
   *  with `withSignature` everything above the original message (Translate). */
  const replyPart = async (withSignature = false): Promise<{ range: Range; html: string } | null> => {
    const sigText = withSignature ? "" : htmlToText(await signatureHtmlFor(accountId));
    const page = pageRef.current;
    if (!page) return null;
    const range = replyRange(page, sigText, withSignature);
    const box = document.createElement("div");
    box.append(range.cloneContents());
    return { range, html: box.innerHTML };
  };

  /** Replace the user's text as its own undo step, so Ctrl+Z puts exactly it back. */
  const replaceReply = (range: Range, html: string) => {
    pageRef.current?.focus();
    replaceUndoably(range, html);
    markDirty();
  };

  /** Replace a block of the quoted original with an edit (as an edit of its own, so Ctrl+Z there
   *  puts it back). */
  const replaceQuotedBlock = (el: Element, html: string) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    replaceReply(range, html);
  };

  const translateTarget = origLang ?? myLang;
  // Translate: the reply above the signature (the signature stays as it is) and the subject (its
  // "RE:" / "FW:" stay); Back to <language> puts both back
  const translateReply = async () => {
    if (!translateTarget || aiBusy || plainText) return;
    const r = await replyPart();
    if (!r) return;
    if (translation) {
      // Back to <the language it was written in>
      const back = `Back to ${translation.sourceLang}`;
      if (r.html !== translation.translated) {
        const ok = await messageBox({
          title: back,
          text: `You changed the translated text. Go back to your ${translation.sourceLang} text anyway?\n\nYour changes to the translation will be lost.`,
          icon: "warning",
          buttons: [back, "Cancel"],
          defaultButton: "Cancel",
          dontAsk: { key: "compose.backToOriginal", answer: back },
        });
        if (ok !== back) return;
      }
      replaceReply(r.range, translation.source);
      // the subject too, unless it was changed since
      if (subject === translation.subject.translated) setSubject(translation.subject.source);
      setTranslation(null);
      setInfo(null);
      return;
    }
    const text = htmlToText(r.html);
    if (!text.trim()) {
      setInfo(`Write your text first, then translate it to ${translateTarget}.`);
      return;
    }
    const sourceLang = detectLanguage(text).language ?? "English";
    // text already in that language goes the other way: Hebrew → English, English → your language
    const target = sourceLang !== translateTarget ? translateTarget : sourceLang !== "English" ? "English" : myLang !== "English" ? myLang : null;
    if (!target) {
      setInfo("Your text is already in English. Choose another language in Tools › Options › AI to translate into it.");
      return;
    }
    setAiBusy("translate");
    try {
      const { prefix, rest } = splitSubject(subject);
      // the AI gets the text with its structure only (aiText.ts), pictures as short references
      // (aiImages.ts); Back to <language> puts the original back exactly
      const body = withoutImages(textForAi(r.html));
      const [out, subj] = await Promise.all([
        aiComposeApi.translate(body.html, target),
        rest.trim() ? aiComposeApi.translate(`<p>${escapeHtml(rest)}</p>`, target).catch(() => null) : null,
      ]);
      replaceReply(r.range, directed(body.restore(out.html)));
      const newSubject = subj ? `${prefix}${htmlToText(subj.html).replace(/\s+/g, " ").trim()}` : subject;
      if (newSubject !== subject) setSubject(newSubject);
      const after = await replyPart();
      setTranslation({ source: r.html, sourceLang, translated: after?.html ?? out.html, lang: target, subject: { source: subject, translated: newSubject } });
      setInfo(`Translated to ${target}. Press Back to ${sourceLang} to undo.`);
    } catch (e) {
      errorBox(e, "Translate");
    } finally {
      setAiBusy(null);
    }
  };

  const aiReview = async () => {
    if (aiBusy || plainText) return;
    const r = await replyPart();
    if (!r) return;
    const fields = merge?.summary.headers ?? [];
    const inReply = instructions(htmlToText(r.html), fields).length > 0;
    // {instructions} typed into the original below count too (see quotedInstructions.ts)
    const baseline = prep.draftMessageId ? null : braceUnits(new DOMParser().parseFromString(prep.quotedHtml, "text/html").body, fields).flatMap((u) => u.found);
    const original = pageRef.current && originalOf(pageRef.current);
    const inQuote = original ? unitsToReview(braceUnits(original, fields), baseline) : [];
    if (!inReply && !inQuote.length) {
      setInfo("No {instructions} found. Write what you want changed in curly braces, e.g. {thank John for the great work}, then press AI Review.");
      return;
    }
    const names = (v: Recipient[]) => v.map((x) => (x.name ? `${x.name} <${x.email}>` : x.email)).join("; ");
    const context = [
      `Subject: ${subject}`,
      `To: ${names(toRef.current?.commit() ?? to)}`,
      `Cc: ${names(ccRef.current?.commit() ?? cc)}`,
      "",
      htmlToText(original?.outerHTML ?? ""),
    ].join("\n");
    setAiBusy("review");
    try {
      // pictures stay here: the AI gets short references to them (aiImages.ts)
      const replyBody = withoutImages(r.html);
      const blocks = inQuote.map((u) => withoutImages(maskedHtml(u.el, u.keep)));
      const [reply, ...quoted] = await Promise.all([
        inReply ? aiComposeApi.review(replyBody.html, context, fields) : null,
        ...blocks.map((b) => aiComposeApi.review(b.html, context, fields)),
      ]);
      if (reply) replaceReply(r.range, replyBody.restore(reply.html));
      quoted.forEach((out, i) => out && replaceQuotedBlock(inQuote[i].el, unmask(blocks[i].restore(out.html))));
      const applied = [reply, ...quoted].reduce((n, o) => n + (o?.applied ?? 0), 0);
      setInfo(`AI Review applied ${applied} instruction${applied === 1 ? "" : "s"}. Press Ctrl+Z to undo.`);
    } catch (e) {
      errorBox(e, "AI Review");
    } finally {
      setAiBusy(null);
    }
  };

  // Reply with AI: the window was opened with aiDraft=1
  const drafted = useRef(false);
  useEffect(() => {
    if (drafted.current || !windowParams().get("aiDraft") || !c0.sourceMessageId) return;
    drafted.current = true;
    setAiBusy("draft");
    setInfo("Drafting a reply with AI…");
    aiComposeApi
      .draftReply(c0.sourceMessageId)
      .then((out) => {
        const page = pageRef.current;
        if (page) replaceReply(pageStart(page), out.html);
        else {
          // Plain Text: the draft goes in as text above what is there
          setPlainValue((v) => `${htmlToText(out.html).trim()}\n${v}`);
          markDirty();
        }
        setInfo(`AI draft added — edit it as you like${page ? " (Ctrl+Z removes it)" : ""}.`);
      })
      .catch((e) => setInfo(`AI draft failed: ${e instanceof Error ? e.message : String(e)}`))
      .finally(() => setAiBusy(null));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // ---- mail merge ---------------------------------------------------------------------------
  const insertField = (header: string) => {
    const token = `{${header}}`;
    if (lastField.current === "subject" && subjectRef.current) {
      const el = subjectRef.current;
      const a = el.selectionStart ?? subject.length;
      const b = el.selectionEnd ?? a;
      setSubject(subject.slice(0, a) + token + subject.slice(b));
      markDirty();
      setTimeout(() => {
        el.focus();
        el.setSelectionRange(a + token.length, a + token.length);
      }, 0);
    } else if (plainText) {
      const el = document.querySelector<HTMLTextAreaElement>(".cm-compose-plain");
      const a = el?.selectionStart ?? plainValue.length;
      const b = el?.selectionEnd ?? a;
      setPlainValue(plainValue.slice(0, a) + token + plainValue.slice(b));
      markDirty();
    } else exec("insertText", token);
  };

  const changeList = async () => {
    const l = await pickMergeList();
    if (l) setMerge(l);
  };

  const mergeMenu = (): MenuItem[] => [
    { label: "Send Test to Me", icon: "mail", onClick: () => runMerge("test") },
    { label: "Save All as Drafts", icon: "save", onClick: () => runMerge("drafts") },
    { type: "separator" },
    { label: "Send All…", icon: "send", bold: true, onClick: () => runMerge("send") },
  ];
  const openMergeMenu = () => {
    const r = sendRef.current?.getBoundingClientRect();
    if (r) showMenu(r.left, r.bottom, mergeMenu(), { minWidth: 200 });
  };

  const runMerge = async (mode: MergeMode) => {
    if (!merge || busy) return;
    if (!subject.trim()) {
      const r = await messageBox({
        title: "Mail Merge",
        text: "This message has no subject.\n\nDo you want to continue anyway?",
        icon: "warning",
        buttons: ["Continue", "Cancel"],
        defaultButton: "Cancel",
        dontAsk: { key: "merge.noSubject", answer: "Continue" },
      });
      if (r !== "Continue") {
        subjectRef.current?.focus();
        return;
      }
    }
    const c = buildCompose();
    c.to = [];
    c.cc = [];
    c.bcc = [];
    c.draftId = null;
    setBusy(true);
    try {
      const p = await mergeApi.check(merge.path, c);
      if (p.unknownFields.length) {
        await alertBox(
          `These fields don't match any column in ${merge.summary.fileName}:\n\n${p.unknownFields.join("\n")}\n\nColumns: ${merge.summary.headers.filter(Boolean).join(", ")}`,
          "Mail Merge",
          "warning",
        );
        return;
      }
      if (p.messages === 0) {
        await alertBox(`${merge.summary.fileName} has no rows with a valid e-mail address.`, "Mail Merge", "warning");
        return;
      }
      if (mode === "test") {
        const r = await mergeApi.run(merge.path, c, "test");
        const t = new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
        setInfo(`Test message sent to you at ${t}, filled in from row ${r.testRow} (${r.testEmail}).`);
        return;
      }
      const go = await messageBox({
        title: "Mail Merge",
        text: preflightText(p, mode),
        icon: mode === "send" ? "help" : "info",
        buttons: [mode === "send" ? "Send" : "Save Drafts", "Cancel"],
        defaultButton: "Cancel",
      });
      if (go !== "Send" && go !== "Save Drafts") return;
      if (mode === "drafts") setInfo(`Saving ${p.messages} drafts…`);
      await mergeApi.run(merge.path, c, mode);
      closing.current = true;
      await destroy();
    } catch (e) {
      errorBox(e, "Mail Merge");
    } finally {
      if (!closing.current) setBusy(false);
    }
  };

  const send = async () => {
    if (busy) return;
    if (merge) {
      openMergeMenu();
      return;
    }
    const chk = await runCheckNames();
    if (!chk.ok) return;
    if (chk.to.length + chk.cc.length + chk.bcc.length === 0) {
      await alertBox("There must be at least one name or contact group in the To, Cc, or Bcc box.", "ClassicMail", "warning");
      toRef.current?.focus();
      return;
    }
    if (!subject.trim()) {
      const r = await messageBox({
        title: "ClassicMail",
        text: "This message has no subject.\n\nDo you want to send it anyway?",
        icon: "warning",
        buttons: ["Send Anyway", "Don't Send"],
        defaultButton: "Don't Send",
        dontAsk: { key: "compose.noSubject", answer: "Send Anyway" },
      });
      if (r !== "Send Anyway") {
        subjectRef.current?.focus();
        return;
      }
    }
    // several people at one company and one at another may be the wrong address (domainCheck.ts)
    const companies = companiesWarning(
      [...chk.to, ...chk.cc, ...chk.bcc].map((r) => r.email),
      [...prep.fromChoices.map((f) => f.email), ...(from ? [from] : [])],
      prep.replyTargets ? [...prep.replyTargets.all.to, ...prep.replyTargets.all.cc].map((a) => a.email) : [],
    );
    if (companies) {
      const people = (n: number) => (n === 1 ? "1 person" : `${n} people`);
      const r = await messageBox({
        title: "More Than One Company",
        text: `This message is going to people at more than one company:\n\n${companies.map((c) => `   ${c.domain} (${people(c.count)})`).join("\n")}\n\nCheck that everyone should get it.`,
        icon: "warning",
        buttons: ["Send Anyway", "Don't Send"],
        defaultButton: "Don't Send",
        dontAsk: { key: "compose.manyCompanies", answer: "Send Anyway" },
      });
      if (r !== "Send Anyway") {
        toRef.current?.focus();
        return;
      }
    }
    const mine = yourPart();
    const bodyText = plainText ? plainValue : mine.text;
    // a picture in what you wrote counts as attached too ("see the attached chart" over a pasted one)
    const hasPicture = !plainText && mine.picture;
    if (attachments.length === 0 && !hasPicture && /\b(attached|attachment|enclosed)\b|מצורף|מצורפת|מצ"ב|קובץ מצורף/i.test(bodyText)) {
      const r = await messageBox({
        title: "Attachment Reminder",
        text: "You may have forgotten to attach a file.\n\nThe message mentions an attachment, but nothing is attached.",
        icon: "warning",
        buttons: ["Send Anyway", "Don't Send"],
        defaultButton: "Don't Send",
        dontAsk: { key: "compose.attachmentReminder", answer: "Send Anyway" },
      });
      if (r !== "Send Anyway") return;
    }
    setBusy(true);
    try {
      if (saving.current) await saving.current.catch(() => {});
      const c = buildCompose();
      c.to = strip(chk.to);
      c.cc = strip(chk.cc);
      c.bcc = strip(chk.bcc);
      await composeApi.send(c);
      closing.current = true;
      await destroy();
    } catch (e) {
      setBusy(false);
      errorBox(e, "Send");
    }
  };

  const requestClose = async () => {
    if (closing.current) return;
    if (merge) {
      if (dirty.current) {
        const r = await messageBox({
          title: "Mail Merge",
          text: "Discard this mail merge?",
          icon: "warning",
          buttons: ["Discard", "Cancel"],
          defaultButton: "Cancel",
          dontAsk: { key: "merge.discard", answer: "Discard" },
        });
        if (r !== "Discard") return;
      }
      closing.current = true;
      await destroy();
      return;
    }
    if (!dirty.current) {
      closing.current = true;
      await destroy();
      return;
    }
    const r = await messageBox({
      title: "ClassicMail",
      text: "Do you want to save changes?",
      icon: "warning",
      buttons: ["Yes", "No", "Cancel"],
      defaultButton: "Yes",
    });
    if (r === "Yes") {
      try {
        await saveDraft(true);
      } catch (e) {
        errorBox(e, "Save");
        return;
      }
      closing.current = true;
      await destroy();
    } else if (r === "No") {
      closing.current = true;
      // an AutoSave copy of a message that was never saved on purpose goes away
      if (draftId.current && !savedExplicitly.current && !openedFromDraft.current) {
        await composeApi.discardDraft(accountId, draftId.current).catch(() => {});
      }
      await destroy();
    }
  };
  const requestCloseRef = useRef(requestClose);
  requestCloseRef.current = requestClose;

  useEffect(() => {
    const un = getCurrentWindow().onCloseRequested((e) => {
      if (closing.current) return;
      e.preventDefault();
      requestCloseRef.current();
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  type FileRow = { path: string; name: string; folder: string; size: number; used: number };
  const recentFiles = useQuery({
    queryKey: ["mail", "recentFiles"],
    queryFn: () => rpc<FileRow[]>("mail.compose.recentFiles", { limit: 12 }),
    staleTime: 15_000,
    refetchOnWindowFocus: true,
  });
  // the recipients' company folder (learnt from saving their attachments and attaching files to
  // them, save_folders.rs) and its newest files: the top of Attach File when writing to a client
  const emailsOf = (v: Recipient[]) => v.map((r) => r.email).filter(Boolean);
  const recips = { to: emailsOf(to), cc: emailsOf(cc) };
  const recipsRef = useRef(recips);
  recipsRef.current = recips;
  const clientFolder = useQuery({
    queryKey: ["mail", "clientFolder", recips.to.join(","), recips.cc.join(",")],
    queryFn: () => mailApi.suggestFolder(recips).catch(() => null),
    enabled: recips.to.length + recips.cc.length > 0,
    staleTime: 30_000,
  });
  const clientFiles = useQuery({
    queryKey: ["mail", "clientFiles", clientFolder.data],
    queryFn: () => rpc<FileRow[]>("mail.compose.folderFiles", { folder: clientFolder.data, limit: 6 }),
    enabled: !!clientFolder.data,
    staleTime: 15_000,
    refetchOnWindowFocus: true,
  });

  // ---- drag & drop files → attachments ------------------------------------------------------
  const addPaths = useCallback(
    async (paths: string[], remember = true) => {
      if (!paths.length) return;
      try {
        const infos = await composeApi.fileInfo(paths);
        const ok = infos.filter((i) => i.exists);
        if (ok.length < infos.length) alertBox("Folders and missing files can't be attached.", "Attach File", "warning");
        setAttachments((a) => [...a, ...ok.map((i) => toAttachItem({ path: i.path, name: i.name, size: i.size, mimeType: i.mimeType }))]);
        // most recently attached files head the Attach File › Recent Items list next time, and the
        // folder becomes the recipients' company folder (as saving their attachments does)
        if (ok.length && remember) {
          rpc("mail.compose.rememberAttachments", { paths: ok.map((i) => i.path) }).catch(() => {});
          const r = recipsRef.current;
          const folder = ok[0].path.replace(/[\\/][^\\/]*$/, "");
          if (r.to.length + r.cc.length && folder)
            mailApi
              .rememberFolder(r, folder)
              .then(() => queryClient.invalidateQueries({ queryKey: ["mail", "clientFolder"] }))
              .catch(() => {});
        }
        markDirty();
      } catch (e) {
        errorBox(e);
      }
    },
    [markDirty],
  );
  // Files dropped anywhere on the window (from Explorer, the desktop, an attachment dragged out of
  // another message) are attached, as in Outlook. The page gets their contents but not their
  // paths, so the back end keeps a copy to attach (src-tauri/src/mail/compose/dropped.rs).
  const attachDropped = async (files: File[]) => {
    const paths: string[] = [];
    for (const f of files) {
      if (!f.size && !f.type) {
        alertBox(`"${f.name}" looks like a folder; folders can't be attached.`, "Attach File", "warning");
        continue;
      }
      if (f.size > 25 * 1024 * 1024) {
        alertBox(`"${f.name}" is too big to attach (Gmail takes up to 25 MB per message).`, "Attach File", "warning");
        continue;
      }
      try {
        const data = await new Promise<string>((resolve, reject) => {
          const r = new FileReader();
          r.onload = () => resolve(String(r.result).replace(/^data:[^,]*,/, ""));
          r.onerror = () => reject(r.error ?? new Error(`${f.name} couldn't be read`));
          r.readAsDataURL(f);
        });
        paths.push(await rpc<string>("mail.compose.stageDropped", { name: f.name, data }));
      } catch (e) {
        errorBox(e);
      }
    }
    // temporary copies: not for Attach File's Recent Items
    void addPaths(paths, false);
  };
  const hasFiles = (e: React.DragEvent) => e.dataTransfer.types.includes("Files");

  // Ctrl+V of files copied in Explorer attaches them, as in Outlook (the page gets no path for a
  // pasted file, so the back end reads the clipboard); a picture with no file behind it (a
  // screenshot) still goes into the text
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const dt = e.clipboardData;
      if (!dt || dt.types.includes("text/plain") || dt.types.includes("text/html")) return;
      const images = Array.from(dt.files).filter((f) => f.type.startsWith("image/"));
      const inBody = !!pageRef.current?.contains(document.activeElement);
      e.preventDefault();
      e.stopImmediatePropagation();
      void rpc<string[]>("app.clipboard.files", {})
        .catch(() => [] as string[])
        .then((paths) => {
          if (paths.length) addPaths(paths);
          else if (images.length && inBody) insertImageFiles(images);
        });
    };
    document.addEventListener("paste", onPaste, true);
    return () => document.removeEventListener("paste", onPaste, true);
  }, [addPaths, insertImageFiles]);

  if (import.meta.env.DEV) (window as unknown as { __composeTest: unknown }).__composeTest = { addPaths, page: () => pageRef.current };

  const attachFile = async () => {
    // opens in the folder used for the recipients' company (writing to @acme.example → the Acme folder)
    const emails = (v: Recipient[]) => v.map((r) => r.email).filter(Boolean);
    const folder = await mailApi.suggestFolder({ to: emails(toRef.current?.commit() ?? to), cc: emails(ccRef.current?.commit() ?? cc) }).catch(() => null);
    const sel = await openDialog({ multiple: true, title: "Insert File", defaultPath: folder ?? undefined }).catch(() => null);
    if (!sel) return;
    addPaths(Array.isArray(sel) ? sel : [sel]);
  };

  // Attach File ▾ — the recipients' company folder's newest files, Recent Items (like the new
  // Outlook), then Browse This PC…
  const attachMenu = (): MenuItem[] => {
    const imageExt = /.(png|jpe?g|gif|bmp|webp|tiff?)$/i;
    const item = (r: FileRow): MenuItem => ({
      label: r.name,
      icon: imageExt.test(r.name) ? "picture" : "attach-file",
      shortcut: r.folder.split(/[\\/]/).filter(Boolean).pop() ?? "",
      onClick: () => addPaths([r.path]),
    });
    const client = clientFolder.data ? (clientFiles.data ?? []) : [];
    const inClient = new Set(client.map((r) => r.path.toLowerCase()));
    const files = (recentFiles.data ?? []).filter((r) => !inClient.has(r.path.toLowerCase()));
    const clientName = clientFolder.data?.split(/[\\/]/).filter(Boolean).pop() ?? "";
    return [
      ...(client.length ? [{ type: "header", label: `${clientName} Folder` } as MenuItem, ...client.map(item), { type: "separator" } as MenuItem] : []),
      { type: "header", label: "Recent Items" },
      ...(files.length ? files.map(item) : [{ label: "(no recent files)", disabled: true } as MenuItem]),
      { type: "separator" },
      { label: "Browse This PC…", icon: "folder-open", onClick: attachFile },
    ];
  };

  const insertPicture = async () => {
    const sel = await openDialog({
      multiple: false,
      title: "Insert Picture",
      filters: [{ name: "All Pictures", extensions: ["png", "jpg", "jpeg", "gif", "bmp", "webp"] }],
    }).catch(() => null);
    if (!sel || Array.isArray(sel)) return;
    try {
      const img = await composeApi.readImage(sel);
      if (plainText) return alertBox("Pictures can't be inserted in a Plain Text message.");
      exec("insertHTML", imgHtml(img.dataUrl, img.name));
    } catch (e) {
      errorBox(e);
    }
  };

  const removeAttachments = (keys: string[]) => {
    if (!keys.length) return;
    setAttachments((a) => a.filter((x) => !keys.includes(x.key)));
    setSelAttach([]);
    markDirty();
  };
  const openAttachment = (a: AttachItem) => {
    const src = a.spec.path ? { path: a.spec.path } : { attachmentId: a.spec.attachmentId };
    void openWithPrompt((confirmed) => composeApi.openAttachment({ ...src, confirmed }));
  };

  // ---- formatting: the browser's editing commands on the page ---------------------------------
  // the selection is remembered here; only the ribbon buttons that show it redraw when it moves
  useEffect(
    () =>
      trackSelection(
        () => pageRef.current,
        (r) => (savedSel.current = r),
      ),
    [],
  );
  /** Font size: execCommand only knows sizes 1–7, so mark with 7 and make that pt. The <font> the
   *  command made stays (only its attributes change), so Ctrl+Z / Ctrl+Y still find it. At a caret
   *  with nothing selected, a zero-width space gets the size, so what is typed next has it too. */
  const fontSize = (pt: number) => {
    const page = pageRef.current;
    if (!page) return;
    restoreSel();
    page.querySelectorAll('font[size="7"]').forEach((f) => f.setAttribute("data-cm-own", ""));
    document.execCommand("styleWithCSS", false, "false");
    document.execCommand("fontSize", false, "7");
    if (window.getSelection()?.isCollapsed) document.execCommand("insertText", false, "\u200b");
    page.querySelectorAll<HTMLElement>('font[size="7"]:not([data-cm-own])').forEach((f) => {
      f.removeAttribute("size");
      f.style.fontSize = `${pt}pt`;
    });
    page.querySelectorAll("[data-cm-own]").forEach((f) => f.removeAttribute("data-cm-own"));
    markDirty();
  };
  /** Bullets / Numbering, with the caret staying where it was in the text (see `keepSelection`). */
  const list = (cmd: "insertUnorderedList" | "insertOrderedList") => {
    const page = pageRef.current;
    if (!page) return;
    restoreSel();
    keepSelection(page, () => exec(cmd));
  };
  const fmt = {
    bold: () => exec("bold"),
    italic: () => exec("italic"),
    underline: () => exec("underline"),
    strike: () => exec("strikeThrough"),
    bullets: () => list("insertUnorderedList"),
    numbering: () => list("insertOrderedList"),
    indent: () => exec("indent"),
    outdent: () => exec("outdent"),
    align: (a: "left" | "center" | "right" | "justify") => exec({ left: "justifyLeft", center: "justifyCenter", right: "justifyRight", justify: "justifyFull" }[a]),
    color: (c: string | null) => exec("foreColor", c ?? "#000000"),
    highlight: (c: string | null) => exec("hiliteColor", c ?? "transparent"),
    font: (f: string) => exec("fontName", f),
    size: fontSize,
    clear: () => exec("removeFormat"),
  };
  /** Grow / Shrink Font: the next size up or down the list from the size at the cursor. */
  const growFont = (dir: 1 | -1) => {
    const page = pageRef.current;
    const cur = (page && savedSel.current && domFormats(savedSel.current, page)[0]?.size) || font.size;
    fontSize(dir > 0 ? (FONT_SIZES.find((s) => s > cur) ?? cur) : ([...FONT_SIZES].reverse().find((s) => s < cur) ?? cur));
  };
  /** Tab in a list item indents it (Shift+Tab: back out); in a table cell it goes to the next cell
   *  (Shift+Tab: the one before) and selects its text, as in Word. */
  const onTab = (e: React.KeyboardEvent) => {
    const n = window.getSelection()?.anchorNode;
    const at = (n instanceof Element ? n : n?.parentElement)?.closest("li, td, th");
    if (!at || !pageRef.current?.contains(at)) return;
    e.preventDefault();
    if (at.tagName === "LI") return exec(e.shiftKey ? "outdent" : "indent");
    const next = nextCell(at as HTMLTableCellElement, e.shiftKey ? -1 : 1);
    if (!next) return;
    const r = document.createRange();
    r.selectNodeContents(next);
    select(pageRef.current, r);
  };
  // Word's formatting keys (Ctrl+B / I / U, Ctrl+Z / Y and Ctrl+A are the browser's own)
  const onPageKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Tab" && !e.ctrlKey && !e.altKey) return onTab(e);
    if (!e.ctrlKey || e.altKey) return;
    const keys: Record<string, () => void> = e.shiftKey
      ? { m: fmt.outdent }
      : {
          l: () => fmt.align("left"),
          e: () => fmt.align("center"),
          r: () => fmt.align("right"),
          j: () => fmt.align("justify"),
          m: fmt.indent,
          "]": () => growFont(1),
          "[": () => growFont(-1),
          " ": fmt.clear,
        };
    const f = keys[keyOf(e)];
    if (!f) return;
    e.preventDefault();
    f();
  };

  const clipboard = {
    cut: () => document.execCommand("cut"),
    copy: () => document.execCommand("copy"),
    paste: async () => {
      try {
        // files copied in Explorer: attach them
        const copied = await rpc<string[]>("app.clipboard.files", {}).catch(() => [] as string[]);
        if (copied.length) {
          void addPaths(copied);
          return;
        }
        // in To / Cc / Bcc / Subject: plain text, through the box's own paste handling
        const field = document.activeElement;
        if (field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) {
          const t = await navigator.clipboard.readText();
          const dt = new DataTransfer();
          dt.setData("text/plain", t);
          if (field.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }))) document.execCommand("insertText", false, t);
          return;
        }
        const items = await navigator.clipboard.read();
        for (const it of items) {
          if (it.types.includes("text/html")) {
            exec("insertHTML", cleanPastedHtml(await (await it.getType("text/html")).text()));
            return;
          }
          const img = it.types.find((t) => t.startsWith("image/"));
          if (img) {
            const blob = await it.getType(img);
            insertImageFiles([new File([blob], "image.png", { type: img })]);
            return;
          }
          if (it.types.includes("text/plain")) {
            exec("insertText", await (await it.getType("text/plain")).text());
            return;
          }
        }
      } catch {
        alertBox("Use Ctrl+V to paste here.", "Paste");
      }
    },
  };

  // Format Painter: copies the formatting at the cursor (reply or original) and paints it on the
  // next selection, in either part
  const startPainter = () => {
    if (painter.current) {
      painter.current = null;
      setPainterOn(false);
      return;
    }
    const page = pageRef.current;
    if (!page || !savedSel.current) return;
    painter.current = paintFromDom(savedSel.current.startContainer, page);
    setPainterOn(true);
  };
  const applyPainter = () => {
    const p = painter.current;
    if (!p) return;
    const sel = window.getSelection();
    const page = pageRef.current;
    // nothing selected yet: the brush stays on
    if (!page || !sel?.rangeCount || sel.isCollapsed || !page.contains(sel.anchorNode)) return;
    // the text gets the formatting and keeps its links, lines and cells: inside one paragraph, list
    // or table it is painted in place (one undo step); over several, a painted copy replaces it
    const range = sel.getRangeAt(0);
    const at = range.commonAncestorContainer;
    const box = (at.nodeType === Node.ELEMENT_NODE ? (at as Element) : at.parentElement)?.closest("table, ul, ol, p, h1, h2, h3, h4, h5, h6, pre, blockquote");
    if (box && box !== page && page.contains(box)) changeUndoably(box, () => paintRange(range, p));
    else replaceUndoably(range, paintedHtml(range, p));
    markDirty();
    painter.current = null;
    setPainterOn(false);
  };

  const insertSignatureMenu = async (e: React.MouseEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const list = await composeApi.signatures().catch(() => ({ signatures: [] as { id: number; name: string; html: string }[] }));
    const items: MenuItem[] = list.signatures.map((s) => ({
      label: s.name,
      onClick: () => {
        if (plainText) {
          setPlainValue((v) => `${v}\n${htmlToText(s.html)}`);
          markDirty();
        } else exec("insertHTML", s.html);
      },
    }));
    if (items.length) items.push({ type: "separator" });
    items.push({ label: "Signatures…", icon: "signature", onClick: () => setSigOpen(true) });
    showMenu(r.left, r.bottom, items, { minWidth: 180 });
  };

  // ---- format: HTML ⇄ plain text --------------------------------------------------------------
  /** The HTML Source tab's edits onto the page (switching back to Message, or away). */
  const applySource = () => {
    const page = pageRef.current;
    if (page && srcTabs && bodyTab === "alt" && srcText !== srcStart.current) {
      page.innerHTML = srcText;
      savedSel.current = null;
      resetSelState();
      markDirty();
    }
  };
  /** View Source: the HTML Source tab, its text the page's HTML. */
  const showSource = () => {
    if (plainText) return;
    const t = prettyHtml(pageRef.current?.innerHTML ?? "");
    srcStart.current = t;
    setSrcText(t);
    setSrcTabs(true);
    setBodyTab("alt");
  };
  const closeSource = () => {
    applySource();
    setSrcTabs(false);
    setBodyTab("main");
  };
  const toPlain = async () => {
    if (markdown) {
      // Markdown → Plain Text: the same text, sent as it is
      setMarkdown(false);
      setBodyTab("main");
      markDirty();
      return;
    }
    if (plainText) return;
    closeSource();
    const r = await messageBox({
      title: "ClassicMail",
      text: "Converting this message to Plain Text will lose any formatting, pictures, or other content that is not text.\n\nDo you want to continue?",
      icon: "warning",
      buttons: ["Continue", "Cancel"],
      defaultButton: "Continue",
      dontAsk: { key: "compose.toPlainText", answer: "Continue" },
    });
    if (r !== "Continue") return;
    setPlainValue(htmlToText(pageRef.current?.innerHTML ?? ""));
    setPlainText(true);
    markDirty();
  };
  const toHtml = () => {
    if (!plainText) return;
    loadHtml.current = markdown ? markdownToHtml(plainValue) : textToHtml(plainValue);
    setMarkdown(false);
    setPlainText(false);
    setBodyTab("main");
    markDirty();
  };
  /** Format › Markdown: the message written in Markdown (Preview shows it), sent as formatting. */
  const toMarkdown = async () => {
    if (markdown) return;
    if (!plainText) {
      const r = await messageBox({
        title: "ClassicMail",
        text: "Converting this message to Markdown keeps its headings, lists, tables and links. Colours, fonts and pictures are lost.\n\nDo you want to continue?",
        icon: "warning",
        buttons: ["Continue", "Cancel"],
        defaultButton: "Continue",
        dontAsk: { key: "compose.toMarkdown", answer: "Continue" },
      });
      if (r !== "Continue") return;
      closeSource();
      setPlainValue(htmlToMarkdown(pageRef.current?.innerHTML ?? ""));
      setPlainText(true);
    }
    setMarkdown(true);
    setBodyTab("main");
    markDirty();
  };

  const saveAs = async () => {
    const name = (subject.trim() || "Untitled").replace(/[\\/:*?"<>|]/g, "_");
    const path = await saveDialog({
      title: "Save As",
      defaultPath: `${name}.htm`,
      filters: [
        { name: "HTML", extensions: ["htm", "html"] },
        { name: "Text Only", extensions: ["txt"] },
      ],
    }).catch(() => null);
    if (!path) return;
    const c = buildCompose();
    const header = `From: ${from ?? ""}\nTo: ${c.to.map(formatAddr).join("; ")}\n${c.cc.length ? `Cc: ${c.cc.map(formatAddr).join("; ")}\n` : ""}Subject: ${subject}\n\n`;
    const content = path.toLowerCase().endsWith(".txt")
      ? header + (plainText ? plainValue : htmlToText(c.html))
      : `<html><head><meta charset="utf-8"><title>${escapeHtml(subject)}</title></head><body>${escapeHtml(header).replace(/\n/g, "<br>")}${c.html}</body></html>`;
    composeApi.saveAs(path, content).catch(errorBox);
  };

  const addSelectNames = (v: { to: Addr[]; cc: Addr[]; bcc: Addr[] }) => {
    setTo(toRecipients(v.to));
    setCc(toRecipients(v.cc));
    setBcc(toRecipients(v.bcc));
    if (v.bcc.length) setShowBccPref(true);
    markDirty();
  };

  const openLink = () => {
    const r = savedSel.current;
    const at = r && (r.startContainer.nodeType === Node.ELEMENT_NODE ? (r.startContainer as Element) : r.startContainer.parentElement);
    const a = at?.closest("a") ?? null;
    const selected = !!r && !r.collapsed;
    setLink({ text: selected ? r.toString() : (a?.textContent ?? ""), a: a && pageRef.current?.contains(a) ? a : null, selected });
  };
  /** Link the selection (all of the link at the cursor) to `url`, or with null remove the link. */
  const linkTo = (url: string | null) => {
    const page = pageRef.current;
    if (page && link?.a) {
      const r = document.createRange();
      r.selectNodeContents(link.a);
      select(page, r);
    }
    if (url) exec("createLink", url);
    else exec("unlink");
  };

  const isBodyFocused = () => !!pageRef.current?.contains(document.activeElement);

  useHotkeys(
    {
      "ctrl+enter": () => send(),
      "alt+s": () => send(),
      "ctrl+s": () => saveDraft(true).catch((e) => errorBox(e, "Save")),
      "ctrl+k": () => (isBodyFocused() ? openLink() : runCheckNames()),
      "ctrl+shift+b": () => setSelectNames(true),
      "ctrl+p": () => window.print(),
      f7: () => setSpell(!spell),
    },
    { allowInInputs: true },
  );
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // an Esc cancelled elsewhere (a drop-down, a dialog) was meant for that
      if (e.defaultPrevented) return;
      if (document.querySelector(".cm-dialog-overlay, .cm-menu, .cm-compose-pop, .cm-compose-ac")) return;
      e.preventDefault();
      requestCloseRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // ---- ribbon --------------------------------------------------------------------------------
  // the Font group shows the formatting at the cursor / over the selection (selState.ts); a box is
  // blank where the selection mixes values
  const fmtOf = (s: SelState) => (plainText ? null : s.fmt);
  const textDisabled = plainText;

  const fontCombo = (
    <OnSel>
      {(s) => {
        const f = fmtOf(s);
        const familyNow = f ? (f.family ?? "") : font.family;
        return (
          <select
            className="cm-input cm-compose-fontsel"
            disabled={textDisabled}
            value={FONTS.includes(familyNow) ? familyNow : ""}
            onChange={(e) => fmt.font(e.target.value)}
            title="Font"
          >
            {!FONTS.includes(familyNow) && <option value="">{familyNow}</option>}
            {FONTS.map((f) => (
              <option key={f} value={f} style={{ fontFamily: f }}>
                {f}
              </option>
            ))}
          </select>
        );
      }}
    </OnSel>
  );
  const sizeCombo = (
    <OnSel>
      {(s) => {
        const f = fmtOf(s);
        const sizeNow = f ? f.size : font.size;
        return (
          <select
            className="cm-input cm-compose-sizesel"
            disabled={textDisabled}
            value={sizeNow !== null && FONT_SIZES.includes(sizeNow) ? sizeNow : ""}
            onChange={(e) => fmt.size(+e.target.value)}
            title="Font Size"
          >
            {(sizeNow === null || !FONT_SIZES.includes(sizeNow)) && <option value="">{sizeNow ?? ""}</option>}
            {FONT_SIZES.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
        );
      }}
    </OnSel>
  );
  const textButtons = (
    <>
      <FmtButton mark="bold" off={plainText} icon="bold" title="Bold (Ctrl+B)" disabled={textDisabled} onClick={fmt.bold} />
      <FmtButton mark="italic" off={plainText} icon="italic" title="Italic (Ctrl+I)" disabled={textDisabled} onClick={fmt.italic} />
      <FmtButton mark="underline" off={plainText} icon="underline" title="Underline (Ctrl+U)" disabled={textDisabled} onClick={fmt.underline} />
    </>
  );
  const colorButtons = (
    <>
      <SmallSplit
        icon="highlight"
        title="Text Highlight Color"
        disabled={textDisabled}
        swatch={hiColor}
        onClick={() => fmt.highlight(hiColor)}
        onArrow={(rect) => setPalette({ kind: "highlight", rect })}
      />
      <SmallSplit
        icon="font-color"
        title="Font Color"
        disabled={textDisabled}
        swatch={fontColor ?? "#000000"}
        onClick={() => fmt.color(fontColor)}
        onArrow={(rect) => setPalette({ kind: "font", rect })}
      />
    </>
  );
  // ---- Format Text › Table (Word's Table Design / Layout essentials) ----
  /** The table cells at the saved selection, when it is in a table. */
  const tableSel = (): qt.TableSel | null => {
    const page = pageRef.current;
    return page && savedSel.current ? qt.tableSelection(savedSel.current, page) : null;
  };
  /** A change to `table`, as one undo step. */
  const changeTable = (table: HTMLTableElement | undefined, f: () => unknown) => {
    restoreSel();
    if (table) changeUndoably(table, f);
    markDirty();
  };
  /** Run a table change on the table at the selection. */
  const onTable = (f: (s: qt.TableSel) => unknown) => {
    const s = tableSel();
    changeTable(s?.g.table, () => s && f(s));
  };
  const shade = (c: string | null) => onTable((s) => qt.shadeCells(s, c));
  const rectOf = (e: React.MouseEvent) => (e.currentTarget as HTMLElement).getBoundingClientRect();
  const borders = (kind: qt.BorderKind) => () => onTable((s) => qt.setBorders(s, kind, pen));
  const bordersMenu = (): MenuItem[] => [
    { label: "Bottom Border", onClick: borders("bottom") },
    { label: "Top Border", onClick: borders("top") },
    { label: "Left Border", onClick: borders("left") },
    { label: "Right Border", onClick: borders("right") },
    { type: "separator" },
    { label: "No Border", onClick: borders("none") },
    { label: "All Borders", onClick: borders("all") },
    { label: "Outside Borders", onClick: borders("outside") },
    { label: "Inside Borders", onClick: borders("inside") },
    { type: "separator" },
    {
      label: "Line Width",
      submenu: [1, 2, 3, 4].map((w) => ({ label: `${w} px`, checked: pen.width === w, onClick: () => setPen((p) => ({ ...p, width: w })) })),
    },
  ];
  const rowsMenu = (): MenuItem[] => [
    ...(["above", "below", "left", "right"] as const).map((w) => ({
      label: `Insert ${w[0].toUpperCase()}${w.slice(1)}`,
      onClick: () => onTable((s) => (w === "above" || w === "below" ? qt.insertRow(s, w) : qt.insertCol(s, w))),
    })),
    { type: "separator" },
    { label: "Delete Rows", icon: "delete", onClick: () => onTable(qt.deleteRows) },
    { label: "Delete Columns", onClick: () => onTable(qt.deleteCols) },
    { label: "Delete Table", onClick: () => onTable(qt.deleteTable) },
  ];
  /** Even Out: every column after the first as wide as the widest cell, every row after the
   *  header as tall as the tallest — a tidy table. */
  const evenOut = () => {
    const table = tableSel()?.g.table;
    if (!table) return;
    const e = measureEven(table);
    const g = qt.gridOf(table);
    const w = e.width;
    const cols = w === null ? undefined : new Map(Array.from({ length: g.width - 1 }, (_, i) => [i + 1, w] as [number, number]));
    const start = e.header ? 1 : 0;
    const rows = e.height === null ? undefined : new Map(Array.from({ length: g.grid.length - start }, (_, i) => [i + start, e.height!] as [number, number]));
    changeTable(table, () => qt.sizeTable(table, { cols, rows }));
  };
  const resizeTo = useRef<(t: ResizeTarget, px: number) => void>(() => {});
  resizeTo.current = (t, px) => changeTable(t.table, () => qt.sizeTable(t.table, t.kind === "col" ? { cols: new Map([[t.index, px]]) } : { rows: new Map([[t.index, px]]) }));
  useEffect(() => {
    const page = pageRef.current;
    if (!page || plainText) return;
    return attachResizer(page, (t, px) => resizeTo.current(t, px));
  }, [plainText]);

  // icons only, with tooltips (Word's Table Design / Layout essentials in one small group)
  const tableGroup = (
    <>
      <Big icon="table" label="Table" disabled={textDisabled} onClick={(e) => setTableAnchor(rectOf(e))} />
      <OnSel>
        {(s) => {
          const inTable = !textDisabled && s.table !== "";
          const canMerge = s.table.startsWith("cells");
          const canSplit = s.table.endsWith("merged");
          return (
            <Stack>
              <Row>
                <SmallSplit
                  icon="shading"
                  title="Shading"
                  disabled={!inTable}
                  swatch={shadeColor}
                  onClick={() => shade(shadeColor)}
                  onArrow={(rect) => setPalette({ kind: "shading", rect })}
                />
                <Small icon="borders" title="Borders (the selected cells, or the whole table)" disabled={!inTable} menu={bordersMenu} />
                <SmallSplit
                  icon="border-color"
                  title="Pen Color (for the borders you add next)"
                  disabled={!inTable}
                  swatch={pen.color}
                  onClick={(rect) => setPalette({ kind: "border", rect })}
                  onArrow={(rect) => setPalette({ kind: "border", rect })}
                />
              </Row>
              <Row>
                <Small icon="table-rows" title="Insert or Delete Rows and Columns" disabled={!inTable} menu={rowsMenu} />
                <Small icon="merge-cells" title="Merge Cells" disabled={!inTable || !canMerge} onClick={() => onTable(qt.mergeCells)} />
                <Small icon="split-cells" title="Split Cells" disabled={!inTable || !canSplit} onClick={() => onTable(qt.splitCell)} />
              </Row>
              <Row>
                <Small
                  icon="table-even"
                  title="Even Out — columns after the first as wide as the widest cell, rows after the header as tall as the tallest"
                  disabled={!inTable}
                  onClick={evenOut}
                />
                <Small icon="header-row" title="Header Row" disabled={!inTable} onClick={() => onTable(qt.toggleHeaderRow)} />
                <Small
                  icon="table-gridlines"
                  title="View Gridlines — faint lines on every table while you write (not sent)"
                  pressed={gridlines}
                  disabled={textDisabled}
                  onClick={() => {
                    setGridlines(!gridlines);
                    try {
                      localStorage.setItem("cm.compose.gridlines", gridlines ? "0" : "1");
                    } catch {
                      // not remembered: still on / off in this window
                    }
                  }}
                />
                <Small
                  icon="cell-align"
                  title="Cell Alignment"
                  disabled={!inTable}
                  menu={() =>
                    (["top", "middle", "bottom"] as const).map((v) => ({
                      label: { top: "Top", middle: "Center", bottom: "Bottom" }[v],
                      onClick: () => onTable((s) => qt.alignCells(s, v)),
                    }))
                  }
                />
              </Row>
            </Stack>
          );
        }}
      </OnSel>
    </>
  );
  const alignButtons = (
    <>
      <CmdButton cmd="justifyLeft" off={plainText} icon="align-left" title="Align Text Left (Ctrl+L)" disabled={textDisabled} onClick={() => fmt.align("left")} />
      <CmdButton cmd="justifyCenter" off={plainText} icon="align-center" title="Center (Ctrl+E)" disabled={textDisabled} onClick={() => fmt.align("center")} />
      <CmdButton cmd="justifyRight" off={plainText} icon="align-right" title="Align Text Right (Ctrl+R)" disabled={textDisabled} onClick={() => fmt.align("right")} />
    </>
  );
  const listButtons = (
    <>
      <CmdButton cmd="insertUnorderedList" off={plainText} icon="bullets" title="Bullets" disabled={textDisabled} onClick={fmt.bullets} />
      <CmdButton cmd="insertOrderedList" off={plainText} icon="numbering" title="Numbering" disabled={textDisabled} onClick={fmt.numbering} />
      <Small icon="outdent" title="Decrease Indent" disabled={textDisabled} onClick={fmt.outdent} />
      <Small icon="indent" title="Increase Indent" disabled={textDisabled} onClick={fmt.indent} />
    </>
  );

  const followUpMenu = (): MenuItem[] => [
    ...["Follow up", "Call", "For Your Information", "Reply", "Review"].map((f) => ({
      label: f,
      icon: "flag",
      checked: opts.flagForRecipients === f,
      onClick: () => {
        setOpts((o) => ({ ...o, flagForRecipients: f }));
        markDirty();
      },
    })),
    { type: "separator" as const },
    { label: "Flag for Recipients…", onClick: () => setOptionsOpen(true) },
    {
      label: "Clear Flag",
      disabled: !opts.flagForRecipients,
      onClick: () => {
        setOpts((o) => ({ ...o, flagForRecipients: null }));
        markDirty();
      },
    },
  ];
  const setImportance = (i: Importance) => {
    setOpts((o) => ({ ...o, importance: o.importance === i ? "normal" : i }));
    markDirty();
  };
  const aiTitle = (t: string) => (aiReady ? t : "Set up AI in Tools › Options › AI");
  const proofingGroup = (
    <>
      <Big icon="spell" label="Spelling" pressed={spell} title="Check spelling as you type (F7)" onClick={() => setSpell(!spell)} />
      {!merge && (
        <Big
          icon="translate"
          label={aiBusy === "translate" ? "Translating…" : translation ? `Back to ${translation.sourceLang}` : origLang ? `Translate to ${origLang}` : "Translate"}
          pressed={!!translation}
          disabled={!aiReady || !!aiBusy || plainText}
          title={aiTitle(
            translation
              ? `Put your ${translation.sourceLang} text back`
              : origLang
                ? `Translate your reply (not the original message) into everyday ${origLang}${origLang === "English" ? "" : " (text already in it: into English)"}`
                : `Translate your text into everyday ${myLang}${myLang === "English" ? "" : ` (${myLang} text: into English)`} (change the language in Tools › Options › AI)`,
          )}
          onClick={translateReply}
        />
      )}
    </>
  );
  const aiGroup = (
    <Big
      icon="ai"
      label={aiBusy === "review" ? "Reviewing…" : "AI Review"}
      disabled={!aiReady || !!aiBusy || plainText}
      title={aiTitle(merge ? "Carry out the {instructions} in your text ({List Columns} stay mail-merge fields)" : "Carry out the {instructions} in your text")}
      onClick={aiReview}
    />
  );
  const includeGroup = (
    <>
      <Big icon="attach-file" label="Attach File" menu={attachMenu} title="Attach File — recent items or browse" />
      <Big
        icon="web-page"
        label="Web Page or MD"
        disabled={textDisabled}
        onClick={() => setWebPageOpen(true)}
        title="Insert a web page or a Markdown document (a web address or a file)"
      />
      <Big icon="attach-item" label="Attach Item" disabled title="Attach Item (not available)" />
      <Stack>
        <Small icon="business-card" label="Business Card" disabled />
        <Small icon="calendar" label="Calendar" disabled />
        <Small icon="signature" label="Signature" onClick={insertSignatureMenu} />
      </Stack>
    </>
  );

  // Message tab: only the working Include commands, so the ribbon fits a 900px window (the
  // Insert tab keeps the full Outlook set)
  const messageIncludeGroup = (
    <>
      <Big icon="attach-file" label="Attach File" menu={attachMenu} title="Attach File — recent items or browse" />
      <Big
        icon="web-page"
        label="Web Page or MD"
        disabled={textDisabled}
        onClick={() => setWebPageOpen(true)}
        title="Insert a web page or a Markdown document (a web address or a file)"
      />
      <Big icon="signature" label="Signature" onClick={insertSignatureMenu} title="Insert a signature" />
    </>
  );

  const tabs: RibbonTab[] = [
    {
      id: "message",
      label: "Message",
      groups: [
        {
          label: "Clipboard",
          content: (
            <>
              <Big icon="paste" label="Paste" onClick={clipboard.paste} />
              <Stack>
                <Small icon="cut" title="Cut (Ctrl+X)" onClick={clipboard.cut} />
                <Small icon="copy" title="Copy (Ctrl+C)" onClick={clipboard.copy} />
                <Small icon="format-painter" title="Format Painter" pressed={painterOn} disabled={textDisabled} onClick={startPainter} />
              </Stack>
            </>
          ),
        },
        {
          label: "Basic Text",
          content: (
            <Stack>
              <Row>
                {fontCombo}
                {sizeCombo}
                <Small icon="grow-font" title="Grow Font (Ctrl+])" disabled={textDisabled} onClick={() => growFont(1)} />
                <Small icon="shrink-font" title="Shrink Font (Ctrl+[)" disabled={textDisabled} onClick={() => growFont(-1)} />
                <CmdButton cmd="insertUnorderedList" off={plainText} icon="bullets" title="Bullets" disabled={textDisabled} onClick={fmt.bullets} />
                <CmdButton cmd="insertOrderedList" off={plainText} icon="numbering" title="Numbering" disabled={textDisabled} onClick={fmt.numbering} />
              </Row>
              <Row>
                {textButtons}
                {colorButtons}
                {alignButtons}
                <Small icon="outdent" title="Decrease Indent" disabled={textDisabled} onClick={fmt.outdent} />
                <Small icon="indent" title="Increase Indent" disabled={textDisabled} onClick={fmt.indent} />
              </Row>
              <Row>
                <Small icon="clear-format" title="Clear Formatting" disabled={textDisabled} onClick={fmt.clear} />
              </Row>
            </Stack>
          ),
        },
        {
          label: "Names",
          content: (
            <>
              <Big icon="address-book" label="Address Book" onClick={() => setSelectNames(true)} />
              <Big icon="check-names" label="Check Names" onClick={() => runCheckNames()} />
              {prep.replyTargets && !merge && (
                <Big
                  icon={replyAllOn ? "reply" : "reply-all"}
                  label={replyAllOn ? "Reply to One" : "Reply All"}
                  disabled={!canToggle(prep.replyTargets)}
                  title={replyAllOn ? "Reply only to the sender" : "Reply to everyone on the original message"}
                  onClick={toggleRecipients}
                />
              )}
            </>
          ),
        },
        { label: "Include", content: messageIncludeGroup },
        {
          label: "Options",
          content: (
            <>
              <Big icon="follow-up" label="Follow Up" pressed={!!opts.flagForRecipients} menu={followUpMenu} />
              <Stack>
                <Small icon="importance-high" title="High Importance" pressed={opts.importance === "high"} onClick={() => setImportance("high")} />
                <Small icon="importance-low" title="Low Importance" pressed={opts.importance === "low"} onClick={() => setImportance("low")} />
              </Stack>
            </>
          ),
        },
        { label: "Proofing", content: proofingGroup },
        { label: "AI", content: aiGroup },
      ],
    },
    {
      id: "insert",
      label: "Insert",
      groups: [
        { label: "Include", content: includeGroup },
        {
          label: "Tables",
          content: <Big icon="table" label="Table" disabled={textDisabled} onClick={(e) => setTableAnchor((e.currentTarget as HTMLElement).getBoundingClientRect())} />,
        },
        { label: "Illustrations", content: <Big icon="picture" label="Picture" disabled={textDisabled} onClick={insertPicture} /> },
        { label: "Links", content: <Big icon="link" label="Hyperlink" disabled={textDisabled} onClick={openLink} /> },
        {
          label: "Text",
          content: <Big icon="hr" label="Horizontal Line" disabled={textDisabled} onClick={() => exec("insertHorizontalRule")} />,
        },
        {
          label: "Symbols",
          content: (
            <Big
              icon="symbol"
              label="Symbol"
              menu={() => [
                ...SYMBOLS.slice(0, 20).map((s) => ({
                  label: s,
                  onClick: () => {
                    if (plainText) {
                      setPlainValue((v) => v + s);
                      markDirty();
                    } else exec("insertText", s);
                  },
                })),
                { type: "separator" as const },
                { label: "More Symbols…", onClick: () => setSymbolOpen(true) },
              ]}
            />
          ),
        },
      ],
    },
    {
      id: "options",
      label: "Options",
      groups: [
        {
          label: "Fields",
          content: (
            <>
              <Big icon="bcc" label="Show Bcc" pressed={showBcc} onClick={() => setShowBccPref(!showBccPref)} />
              <Big icon="from-field" label="Show From" pressed={showFrom} onClick={() => setShowFromPref(!showFromPref)} />
            </>
          ),
        },
        {
          label: "Format",
          content: (
            <Stack>
              <Small icon="format-html" label="HTML" pressed={!plainText} onClick={toHtml} />
              <Small icon="format-text" label="Plain Text" pressed={plainText && !markdown} onClick={toPlain} />
              <Small icon="format-markdown" label="Markdown" pressed={markdown} onClick={() => void toMarkdown()} />
            </Stack>
          ),
        },
        {
          label: "Tracking",
          content: (
            <Stack>
              <Check
                label="Request a Delivery Receipt"
                checked={opts.requestDeliveryReceipt}
                onChange={(v) => {
                  setOpts((o) => ({ ...o, requestDeliveryReceipt: v }));
                  markDirty();
                }}
              />
              <Check
                label="Request a Read Receipt"
                checked={opts.requestReadReceipt}
                onChange={(v) => {
                  setOpts((o) => ({ ...o, requestReadReceipt: v }));
                  markDirty();
                }}
              />
            </Stack>
          ),
        },
        {
          label: "More Options",
          content: (
            <>
              <Big icon="delay" label="Delay Delivery" pressed={opts.deferUntil != null} onClick={() => setOptionsOpen(true)} />
              <Big icon="direct-replies" label="Direct Replies To" pressed={opts.replyTo.length > 0} onClick={() => setOptionsOpen(true)} />
            </>
          ),
        },
      ],
    },
    {
      id: "format",
      label: "Format Text",
      groups: [
        {
          label: "Format",
          content: (
            <>
              <Big icon="format-html" label="HTML" pressed={!plainText} onClick={toHtml} />
              <Big icon="format-text" label="Plain Text" pressed={plainText && !markdown} onClick={toPlain} />
              <Big icon="format-markdown" label="Markdown" pressed={markdown} onClick={() => void toMarkdown()} />
              <Big
                icon="format-source"
                label="View Source"
                pressed={srcTabs}
                disabled={plainText}
                title="The message's HTML, to read or edit (tabs above the text flip between them)"
                onClick={() => (srcTabs ? closeSource() : showSource())}
              />
            </>
          ),
        },
        {
          label: "Font",
          content: (
            <Stack>
              <Row>
                {fontCombo}
                {sizeCombo}
              </Row>
              <Row>
                {textButtons}
                <FmtButton mark="strike" off={plainText} icon="strike" title="Strikethrough" disabled={textDisabled} onClick={fmt.strike} />
                {colorButtons}
              </Row>
              <Row>
                <Small icon="grow-font" title="Grow Font" disabled={textDisabled} onClick={() => growFont(1)} />
                <Small icon="shrink-font" title="Shrink Font" disabled={textDisabled} onClick={() => growFont(-1)} />
                <Small icon="clear-format" title="Clear Formatting" disabled={textDisabled} onClick={fmt.clear} />
                <Small icon="format-painter" title="Format Painter" pressed={painterOn} disabled={textDisabled} onClick={startPainter} />
              </Row>
            </Stack>
          ),
        },
        {
          label: "Paragraph",
          content: (
            <Stack>
              <Row>{listButtons}</Row>
              <Row>
                {alignButtons}
                <CmdButton cmd="justifyFull" off={plainText} icon="align-justify" title="Justify (Ctrl+J)" disabled={textDisabled} onClick={() => fmt.align("justify")} />
              </Row>
            </Stack>
          ),
        },
        { label: "Table", content: tableGroup },
        { label: "Proofing", content: proofingGroup },
        { label: "AI", content: aiGroup },
      ],
    },
  ];

  const orbMenu: MenuItem[] = [
    { label: "Save", icon: "save", shortcut: "Ctrl+S", onClick: () => saveDraft(true).catch((e) => errorBox(e, "Save")) },
    { label: "Save As…", onClick: saveAs },
    { type: "separator" },
    { label: "Properties", icon: "options", onClick: () => setPropsOpen(true) },
    { label: "Message Options…", onClick: () => setOptionsOpen(true) },
    { type: "separator" },
    { label: "Print", icon: "print", shortcut: "Ctrl+P", onClick: () => window.print() },
    { type: "separator" },
    { label: "Close", icon: "close", onClick: () => requestClose() },
  ];

  const quickAccess = (
    <>
      <button className="cm-compose-qat" title="Save (Ctrl+S)" onMouseDown={(e) => e.preventDefault()} onClick={() => saveDraft(true).catch((e) => errorBox(e, "Save"))}>
        <CIcon name="save" />
      </button>
      <button className="cm-compose-qat" title="Undo (Ctrl+Z)" onMouseDown={(e) => e.preventDefault()} onClick={() => document.execCommand("undo")}>
        <CIcon name="undo" />
      </button>
      <button className="cm-compose-qat" title="Redo (Ctrl+Y)" onMouseDown={(e) => e.preventDefault()} onClick={() => document.execCommand("redo")}>
        <CIcon name="redo" />
      </button>
    </>
  );

  // ---- info bar ------------------------------------------------------------------------------
  const infoLines = useMemo(() => {
    const v: string[] = [];
    if (opts.importance === "high") v.push("This message has High Importance.");
    if (opts.importance === "low") v.push("This message has Low Importance.");
    if (opts.deferUntil)
      v.push(
        `Do not deliver before ${new Date(opts.deferUntil).toLocaleString([], { weekday: "long", year: "numeric", month: "long", day: "numeric", hour: "numeric", minute: "2-digit" })}.`,
      );
    if (opts.flagForRecipients) v.push(`Flag for recipients: ${opts.flagForRecipients}.`);
    if (opts.replyTo.length) v.push(`Replies will be sent to ${opts.replyTo.map(formatAddr).join("; ")}.`);
    if (openedFromDraft.current) v.push("This message has not been sent.");
    if (info) v.push(info);
    return v;
  }, [opts, info]);

  const fromChoice = prep.fromChoices.find((f) => f.accountId === accountId && f.email.toLowerCase() === (from ?? "").toLowerCase());
  const fromMenu = (): MenuItem[] =>
    prep.fromChoices.map((f) => ({
      label: f.name ? `${f.name} <${f.email}>` : f.email,
      radio: true,
      checked: f.accountId === accountId && f.email.toLowerCase() === (from ?? "").toLowerCase(),
      onClick: () => {
        const before = accountId;
        setAccountId(f.accountId);
        setFrom(f.email);
        markDirty();
        if (f.accountId !== before) swapSignature(before, f.accountId);
      },
    }));

  // Outlook: choosing another From account swaps the default signature at the end of the message
  const swapSignature = async (fromAcct: number, toAcct: number) => {
    const oldHtml = await signatureHtmlFor(fromAcct);
    const newHtml = await signatureHtmlFor(toAcct);
    if (oldHtml === newHtml) return;
    if (plainText) {
      const oldPlain = htmlToText(oldHtml).trim();
      setPlainValue((v) => {
        const t = v.replace(/\s+$/, "");
        if (oldPlain && t.endsWith(oldPlain)) return `${t.slice(0, t.length - oldPlain.length)}${htmlToText(newHtml).trim()}\n`;
        return v;
      });
      return;
    }
    const page = pageRef.current;
    if (!page) return;
    const blocks = replyBlocks(page);
    const range = document.createRange();
    const textOf = (html: string) => htmlToText(html).replace(/\s+/g, " ").trim();
    const oldText = textOf(oldHtml);
    const last = blocks[blocks.length - 1];
    if (!oldText) {
      // no signature before: the new one goes after your reply, below a blank line. The original
      // is read-only meanwhile, or Chromium merges the new lines into it. After a list there is no
      // place outside it, so the list goes back in too, from the end of the line above it.
      if (!last || !newHtml.trim()) return;
      let html = `<p><br></p>${newHtml}`;
      if (/^(UL|OL)$/.test(last.tagName) || /^(UL|OL)$/.test(last.lastElementChild?.tagName ?? "")) {
        const prev = last.previousElementSibling;
        if (prev) range.setStart(prev, prev.childNodes.length);
        else range.setStartBefore(last);
        range.setEndAfter(last);
        html = listBlockHtml(last) + html;
      } else range.setStartAfter(last);
      const original = originalOf(page);
      original?.setAttribute("contenteditable", "false");
      try {
        select(page, range);
        document.execCommand("insertHTML", false, html);
      } finally {
        original?.removeAttribute("contenteditable");
      }
      markDirty();
      return;
    }
    // the trailing blocks of your reply whose text is the old signature
    for (let i = blocks.length - 1; i >= 0; i--) {
      const tail = textOf(blocks.slice(i).reduce((h, b) => h + b.outerHTML, ""));
      if (tail === oldText) {
        range.setStartBefore(blocks[i]);
        range.setEndAfter(last);
        replaceReply(range, newHtml.trim() ? newHtml : "<p><br></p>");
        return;
      }
      if (tail.length > oldText.length) return; // you typed after the signature: leave it alone
    }
  };

  return (
    <div
      className={`cm-itemwin cm-compose${dropping ? " dropping" : ""}`}
      // files from outside: before the page or a recipient box can take them
      onDragOverCapture={(e) => {
        if (!hasFiles(e)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
        setDropping(true);
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropping(false);
      }}
      onDropCapture={(e) => {
        setDropping(false);
        if (!hasFiles(e) || !e.dataTransfer.files.length) return;
        e.preventDefault();
        e.stopPropagation();
        void attachDropped(Array.from(e.dataTransfer.files));
      }}
    >
      <Ribbon tabs={tabs} orbMenu={orbMenu} quickAccess={quickAccess} />
      {infoLines.length > 0 && (
        <div className="cm-compose-info">
          {infoLines.map((l) => (
            <div key={l}>{l}</div>
          ))}
        </div>
      )}
      <div className="cm-compose-head">
        <button ref={sendRef} className="cm-compose-send" disabled={busy} title={merge ? "Send Test to Me, Save All as Drafts or Send All" : "Send (Ctrl+Enter)"} onClick={send}>
          <CIcon name="send" size={32} />
          <span>{merge ? "Send ▾" : "Send"}</span>
        </button>
        <div className="cm-compose-fields">
          {showFrom && (
            <>
              <button
                className="cm-field-label-btn"
                tabIndex={-1}
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect();
                  showMenu(r.left, r.bottom, fromMenu(), { minWidth: 260 });
                }}
              >
                From ▾
              </button>
              <div className="cm-compose-from">{fromChoice ? `${fromChoice.name} <${fromChoice.email}>` : (from ?? "")}</div>
            </>
          )}
          {merge ? (
            <>
              <label className="cm-compose-label">Merge:</label>
              <div className="cm-compose-merge">
                <CIcon name="mail-merge" />
                <span className="cm-compose-merge-file" title={merge.path}>
                  {merge.summary.fileName}
                </span>
                <span className="cm-muted">
                  — {merge.summary.rowCount} row{merge.summary.rowCount === 1 ? "" : "s"}, one message each
                </span>
                <span className="cm-fill" />
                <button
                  className="cm-button"
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={(e) => {
                    const r = e.currentTarget.getBoundingClientRect();
                    showMenu(
                      r.left,
                      r.bottom,
                      merge.summary.headers.filter(Boolean).map((h) => ({ label: `{${h}}`, onClick: () => insertField(h) })),
                      { minWidth: 180 },
                    );
                  }}
                >
                  Insert Field ▾
                </button>
                <button className="cm-button" onClick={changeList}>
                  Change List…
                </button>
              </div>
            </>
          ) : (
            <>
              <button className="cm-field-label-btn" tabIndex={-1} onClick={() => setSelectNames(true)}>
                To...
              </button>
              <RecipientField ref={toRef} label="To" value={to} onChange={setTo} onDirty={markDirty} group={group} />
              <button className="cm-field-label-btn" tabIndex={-1} onClick={() => setSelectNames(true)}>
                Cc...
              </button>
              <RecipientField ref={ccRef} label="Cc" value={cc} onChange={setCc} onDirty={markDirty} />
            </>
          )}
          {showBcc && !merge && (
            <>
              <button className="cm-field-label-btn" tabIndex={-1} onClick={() => setSelectNames(true)}>
                Bcc...
              </button>
              <RecipientField ref={bccRef} label="Bcc" value={bcc} onChange={setBcc} onDirty={markDirty} />
            </>
          )}
          <label className="cm-compose-label">Subject:</label>
          <input
            ref={subjectRef}
            className="cm-input cm-compose-subject"
            value={subject}
            dir="auto"
            spellCheck={spell}
            onFocus={() => {
              lastField.current = "subject";
            }}
            onChange={(e) => {
              setSubject(e.target.value);
              markDirty();
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" || (e.key === "Tab" && !e.shiftKey)) {
                e.preventDefault();
                if (plainText) document.querySelector<HTMLTextAreaElement>(".cm-compose-plain")?.focus();
                else if (pageRef.current) select(pageRef.current);
              }
            }}
          />
          {attachments.length > 0 && (
            <>
              <label className="cm-compose-label">Attached:</label>
              <div
                className="cm-compose-attached"
                tabIndex={0}
                onKeyDown={(e) => {
                  if (e.ctrlKey && !e.altKey && keyOf(e) === "a") {
                    e.preventDefault();
                    e.stopPropagation();
                    setSelAttach(attachments.map((x) => x.key));
                    attAnchor.current = 0;
                  } else if (e.key === "Delete" || e.key === "Backspace") {
                    e.preventDefault();
                    removeAttachments(selAttach);
                  }
                }}
              >
                {attachments.map((a, i) => (
                  <span key={a.key}>
                    <span
                      className={`cm-compose-att${selAttach.includes(a.key) ? " sel" : ""}`}
                      title={a.name}
                      onMouseDown={(e) => {
                        // a right-click on a selected one keeps the selection (Remove takes them all)
                        if (e.button === 2 && selAttach.includes(a.key)) return;
                        const idx = attachments.map((x, j) => (selAttach.includes(x.key) ? j : -1)).filter((j) => j >= 0);
                        const r = clickSelect(idx, attAnchor.current, i, { ctrl: e.ctrlKey, shift: e.shiftKey });
                        attAnchor.current = r.anchor;
                        setSelAttach(r.sel.map((j) => attachments[j].key));
                      }}
                      onDoubleClick={() => openAttachment(a)}
                      onContextMenu={contextMenu(() => {
                        const keys = selAttach.includes(a.key) ? selAttach : [a.key];
                        return [
                          { label: "Open", bold: true, disabled: keys.length > 1, onClick: () => openAttachment(a) },
                          { type: "separator" },
                          { label: keys.length > 1 ? `Remove ${keys.length} Attachments` : "Remove", icon: "delete", onClick: () => removeAttachments(keys) },
                        ];
                      })}
                    >
                      <CIcon name={fileIcon(a.name, a.mimeType)} />
                      <span className="cm-compose-att-name">{a.name}</span>
                      {a.size > 0 && <span className="cm-compose-att-size">({formatSize(a.size)})</span>}
                    </span>
                    {i < attachments.length - 1 && <span>; </span>}
                  </span>
                ))}
              </div>
            </>
          )}
        </div>
      </div>
      <div
        className="cm-compose-body"
        data-zoom
        style={{ fontFamily: `'${font.family}', sans-serif`, fontSize: `${font.size}pt`, zoom: zoom / 100 }}
        onMouseUp={() => painter.current && applyPainter()}
      >
        {(markdown || (srcTabs && !plainText)) && (
          <BodyTabs
            tabs={
              markdown
                ? [
                    { key: "main", label: "Markdown" },
                    { key: "alt", label: "Preview" },
                  ]
                : [
                    { key: "main", label: "Message" },
                    { key: "alt", label: "HTML Source" },
                  ]
            }
            active={bodyTab}
            onPick={(k) => {
              if (markdown) return setBodyTab(k as "main" | "alt");
              if (k === "alt") showSource();
              else {
                applySource();
                setBodyTab("main");
              }
            }}
            onClose={markdown ? undefined : closeSource}
          />
        )}
        {markdown && bodyTab === "alt" ? (
          <div className="cm-compose-page cm-compose-preview" dir="auto" dangerouslySetInnerHTML={{ __html: markdownToHtml(plainValue) }} />
        ) : plainText ? (
          <textarea
            className="cm-compose-plain"
            value={plainValue}
            dir="auto"
            spellCheck={spell}
            onFocus={() => {
              lastField.current = "body";
            }}
            onChange={(e) => {
              setPlainValue(e.target.value);
              markDirty();
            }}
          />
        ) : (
          <>
            {srcTabs && bodyTab === "alt" && (
              <textarea
                className="cm-compose-source"
                value={srcText}
                spellCheck={false}
                dir="ltr"
                onChange={(e) => {
                  setSrcText(e.target.value);
                  markDirty();
                }}
              />
            )}
            <div
              ref={pageRef}
              className={`cm-compose-page${gridlines ? " gridlines" : ""}`}
              style={srcTabs && bodyTab === "alt" ? { display: "none" } : undefined}
              contentEditable
              suppressContentEditableWarning
              spellCheck={spell}
              onInput={markDirty}
              onFocus={() => {
                lastField.current = "body";
              }}
              onKeyDown={onPageKeyDown}
              onPaste={(e) => {
                // Word / Outlook HTML without its scaffolding; a picture with no HTML goes in as one
                const html = e.clipboardData.getData("text/html");
                const images = Array.from(e.clipboardData.files).filter((f) => f.type.startsWith("image/"));
                // Markdown pasted as text (an AI chat's answer, a README) comes in formatted
                const text = e.clipboardData.getData("text/plain");
                if (html) exec("insertHTML", cleanPastedHtml(html));
                else if (images.length) insertImageFiles(images);
                else if (text && looksLikeMarkdown(text)) exec("insertHTML", markdownToHtml(text));
                else return;
                e.preventDefault();
              }}
            />
          </>
        )}
      </div>

      {palette && (
        <ColorPalette
          kind={palette.kind}
          anchor={palette.rect}
          onClose={() => setPalette(null)}
          onPick={(c) => {
            if (palette.kind === "font") {
              setFontColor(c);
              fmt.color(c);
            } else if (palette.kind === "shading") {
              setShadeColor(c);
              shade(c);
            } else if (palette.kind === "border") {
              setPen((p) => ({ ...p, color: c ?? "#000000" }));
            } else {
              setHiColor(c);
              fmt.highlight(c);
            }
          }}
        />
      )}
      {tableAnchor && <TableGrid anchor={tableAnchor} onClose={() => setTableAnchor(null)} onPick={(rows, cols) => exec("insertHTML", qt.tableHtml(rows, cols))} />}
      {symbolOpen && (
        <SymbolDialog
          onClose={() => setSymbolOpen(false)}
          onPick={(s) => {
            if (plainText) {
              setPlainValue((v) => v + s);
              markDirty();
            } else exec("insertText", s);
          }}
        />
      )}
      {webPageOpen && (
        <WebPageDialog
          onClose={() => setWebPageOpen(false)}
          onOk={async (source) => {
            // the page opens in a hidden window of its own and comes back as e-mail HTML
            // (src-tauri/src/mail/compose/webpage.rs, compose/webPage.ts), inserted at the cursor
            try {
              // a Markdown file or link comes in as its formatting; a web page as it shows on screen
              const html = /\.(md|markdown)(?:[?#].*)?$/i.test(source.trim().replace(/"$/, ""))
                ? markdownToHtml(await rpc<string>("mail.compose.readText", { source }))
                : await rpc<string>("mail.compose.webPage", { source, script: inlinePageScript() });
              // on a line of its own: inserted in the middle of a line, its first paragraph would
              // join that line
              restoreSel();
              const at = window.getSelection()?.anchorNode;
              const line = (at instanceof Element ? at : at?.parentElement)?.closest("p, div, li, h1, h2, h3, h4, td");
              if (line && pageRef.current?.contains(line) && line !== pageRef.current && (line.textContent ?? "").replace(/\u200b/g, "").trim()) exec("insertParagraph");
              exec("insertHTML", html);
            } catch (e) {
              errorBox(e, "Insert Web Page");
              throw e;
            }
          }}
        />
      )}
      {link && (
        <HyperlinkDialog
          initialText={link.text}
          initialUrl={link.a?.getAttribute("href") ?? ""}
          onClose={() => setLink(null)}
          onRemove={link.a ? () => linkTo(null) : undefined}
          onOk={(text, url) => (link.a || link.selected ? linkTo(url) : exec("insertHTML", `<a href="${escapeHtml(url)}">${escapeHtml(text)}</a>`))}
        />
      )}
      {optionsOpen && (
        <MessageOptionsDialog
          value={opts}
          onClose={() => setOptionsOpen(false)}
          onOk={(v) => {
            setOpts(v);
            markDirty();
          }}
        />
      )}
      {selectNames && (
        <SelectNamesDialog
          initial={{ to: strip(to), cc: strip(cc), bcc: strip(bcc) }}
          onOk={(v) => {
            addSelectNames(v);
            setSelectNames(false);
          }}
          onClose={() => setSelectNames(false)}
        />
      )}
      {checkNames && <CheckNamesDialog name={checkNames.name} candidates={checkNames.candidates} onPick={(s) => checkNames.resolve(s)} onClose={() => setCheckNames(null)} />}
      {sigOpen && <SignaturesDialog onClose={() => setSigOpen(false)} />}
      {propsOpen && (
        <PropertiesDialog
          onClose={() => setPropsOpen(false)}
          rows={[
            ["Subject", subject || "(none)"],
            ["From", fromChoice ? `${fromChoice.name} <${fromChoice.email}>` : (from ?? "")],
            ["Recipients", String(to.length + cc.length + bcc.length)],
            ["Attachments", attachments.length ? `${attachments.length} (${formatSize(attachments.reduce((s, a) => s + a.size, 0))})` : "None"],
            ["Format", markdown ? "Markdown" : plainText ? "Plain Text" : "HTML"],
            ["Importance", opts.importance[0].toUpperCase() + opts.importance.slice(1)],
            ["Saved", draftId.current ? "In Drafts" : "Not saved"],
          ]}
        />
      )}
    </div>
  );
}
