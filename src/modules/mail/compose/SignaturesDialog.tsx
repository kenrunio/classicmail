// Outlook 2007 "Signatures and Stationery" (Tools › Options › Mail Format › Signatures…), plus the
// default compose font (Stationery and Fonts). Exported for the Options window.
import { useEffect, useMemo, useState } from "react";
import { useEditor, EditorContent } from "@tiptap/react";
import { useQueryClient } from "@tanstack/react-query";
import { Dialog, confirmBox, errorBox } from "../../../ui";
import { rpc } from "../../../api/rpc";
import { useRpc } from "../../../api/query";
import type { Account } from "../../../api/types";
import { composeApi, type Signature, type SignatureDefaults } from "../../../api/mailCompose";
import { extensions, FONTS, FONT_SIZES } from "./editor";
import { CIcon } from "./icons";
import { ColorPalette, HyperlinkDialog } from "./dialogs";
import "./compose.css";

export interface ComposeFont {
  family: string;
  size: number;
}
/** Is a (system) font installed? Canvas text width differs from the generic fallbacks if so. */
export function fontInstalled(family: string): boolean {
  try {
    const ctx = document.createElement("canvas").getContext("2d");
    if (!ctx) return false;
    const sample = "mmmmmmmmmmwwwwwlliI10@#ab";
    return ["monospace", "serif", "sans-serif"].some((g) => {
      ctx.font = `72px ${g}`;
      const base = ctx.measureText(sample).width;
      ctx.font = `72px '${family}', ${g}`;
      return ctx.measureText(sample).width !== base;
    });
  } catch {
    return false;
  }
}

/** New messages: Aptos 12pt (current Outlook default) when installed, else Calibri 11pt. A font the
 * user picked in Stationery and Fonts (setting mail.composeFont) always wins. */
export const DEFAULT_COMPOSE_FONT: ComposeFont = fontInstalled("Aptos") ? { family: "Aptos", size: 12 } : { family: "Calibri", size: 11 };
export const COMPOSE_FONT_KEY = "mail.composeFont";

function NameDialog({ title, initial, onOk, onClose }: { title: string; initial: string; onOk: (n: string) => void; onClose: () => void }) {
  const [v, setV] = useState(initial);
  const ok = () => {
    if (!v.trim()) return;
    onOk(v.trim());
    onClose();
  };
  return (
    <Dialog
      title={title}
      onClose={onClose}
      width={340}
      footer={
        <>
          <button className="cm-button default" onClick={ok}>
            OK
          </button>
          <button className="cm-button" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <div style={{ marginBottom: 6 }}>Type a name for this signature:</div>
      <input
        className="cm-input"
        style={{ width: "100%" }}
        value={v}
        onChange={(e) => setV(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            ok();
          }
        }}
      />
    </Dialog>
  );
}

/** The dialog needs ~680×620; a smaller host window (Tools › Options) grows to fit it. */
async function fitWindow(w: number, h: number) {
  try {
    const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const { LogicalSize } = await import("@tauri-apps/api/dpi");
    const win = getCurrentWebviewWindow();
    if (window.innerWidth >= w && window.innerHeight >= h) return;
    // setSize is the inner (client) size, which is what window.inner* measure
    await win.setSize(new LogicalSize(Math.max(window.innerWidth, w), Math.max(window.innerHeight, h)));
  } catch {
    /* not in Tauri */
  }
}

export function SignaturesDialog({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    fitWindow(690, 640);
  }, []);
  const qc = useQueryClient();
  const accounts = useRpc<Account[]>(["accounts"], "accounts.list");
  const [sigs, setSigs] = useState<Signature[]>([]);
  const [defaults, setDefaults] = useState<SignatureDefaults[]>([]);
  const [sel, setSel] = useState<number | null>(null);
  const [dirty, setDirty] = useState(false);
  const [acct, setAcct] = useState<number | null>(null);
  const [naming, setNaming] = useState<null | "new" | "rename">(null);
  const [palette, setPalette] = useState<DOMRect | null>(null);
  const [linkOpen, setLinkOpen] = useState(false);

  const reload = async (select?: number) => {
    const r = await composeApi.signatures();
    setSigs(r.signatures);
    setDefaults(r.defaults);
    setSel((s) => select ?? s ?? r.signatures[0]?.id ?? null);
    setAcct((a) => a ?? r.defaults[0]?.accountId ?? null);
  };
  useEffect(() => {
    reload().catch(errorBox);
  }, []);

  const current = sigs.find((s) => s.id === sel) ?? null;
  const editor = useEditor({
    extensions: extensions(),
    content: current?.html ?? "",
    editable: !!current,
    onUpdate: () => setDirty(true),
    editorProps: { attributes: { class: "cm-compose-sigedit-pm", spellcheck: "true" } },
  });
  useEffect(() => {
    if (!editor) return;
    editor.commands.setContent(current?.html ?? "", { emitUpdate: false });
    editor.setEditable(!!current);
    setDirty(false);
  }, [editor, current?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  const saveCurrent = async () => {
    if (!current || !editor || !dirty) return;
    await composeApi.saveSignature({ id: current.id, name: current.name, html: editor.getHTML() });
    setDirty(false);
    await reload(current.id);
  };

  const switchTo = async (id: number) => {
    if (dirty && current) {
      if (await confirmBox(`Do you want to save changes to the signature "${current.name}"?`)) await saveCurrent().catch(errorBox);
    }
    setSel(id);
  };

  const d = defaults.find((x) => x.accountId === acct);
  const setDefault = (field: "newSignatureId" | "replySignatureId", v: number | null) => {
    setDefaults((all) => all.map((x) => (x.accountId === acct ? { ...x, [field]: v } : x)));
  };

  const ok = async () => {
    try {
      await saveCurrent();
      const orig = (await composeApi.signatures()).defaults;
      for (const x of defaults) {
        const o = orig.find((y) => y.accountId === x.accountId);
        if (!o || o.newSignatureId !== x.newSignatureId || o.replySignatureId !== x.replySignatureId) await composeApi.setSignatureDefaults(x);
      }
      qc.invalidateQueries({ queryKey: ["mail", "signatures"] });
      onClose();
    } catch (e) {
      errorBox(e);
    }
  };

  const accountName = (id: number) => accounts.data?.find((a) => a.id === id)?.email ?? `Account ${id}`;
  const fontFamily = (editor?.getAttributes("textStyle").fontFamily as string | undefined) ?? "";

  return (
    <Dialog
      title="Signatures and Stationery"
      onClose={onClose}
      width={660}
      footer={
        <>
          <button className="cm-button default" onClick={ok}>
            OK
          </button>
          <button className="cm-button" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-compose-sigs">
        <div className="cm-compose-sigs-top">
          <fieldset className="cm-fieldset cm-compose-sigs-list">
            <legend>Select signature to edit</legend>
            <div className="cm-compose-list" style={{ height: 112 }}>
              {sigs.map((s) => (
                <div key={s.id} className={`cm-compose-list-row${s.id === sel ? " sel" : ""}`} onClick={() => switchTo(s.id)}>
                  {s.name}
                </div>
              ))}
            </div>
            <div className="cm-compose-sigs-btns">
              <button
                className="cm-button"
                disabled={!current}
                onClick={async () => {
                  if (!current) return;
                  if (!(await confirmBox(`Are you sure you want to delete this signature?`, "Signatures", "warning", "signatures.delete"))) return;
                  await composeApi.deleteSignature(current.id).catch(errorBox);
                  setSel(null);
                  await reload();
                }}
              >
                Delete
              </button>
              <button className="cm-button" onClick={() => setNaming("new")}>
                New
              </button>
              <button className="cm-button" disabled={!current || !dirty} onClick={() => saveCurrent().catch(errorBox)}>
                Save
              </button>
              <button className="cm-button" disabled={!current} onClick={() => setNaming("rename")}>
                Rename
              </button>
            </div>
          </fieldset>
          <fieldset className="cm-fieldset cm-compose-sigs-defaults">
            <legend>Choose default signature</legend>
            <div className="cm-form">
              <label>E-mail account:</label>
              <select className="cm-input" value={acct ?? ""} onChange={(e) => setAcct(+e.target.value)}>
                {defaults.map((x) => (
                  <option key={x.accountId} value={x.accountId}>
                    {accountName(x.accountId)}
                  </option>
                ))}
              </select>
              <label>New messages:</label>
              <select className="cm-input" value={d?.newSignatureId ?? ""} onChange={(e) => setDefault("newSignatureId", e.target.value ? +e.target.value : null)}>
                <option value="">(none)</option>
                {sigs.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name}
                  </option>
                ))}
              </select>
              <label>Replies/forwards:</label>
              <select className="cm-input" value={d?.replySignatureId ?? ""} onChange={(e) => setDefault("replySignatureId", e.target.value ? +e.target.value : null)}>
                <option value="">(none)</option>
                {sigs.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name}
                  </option>
                ))}
              </select>
            </div>
          </fieldset>
        </div>
        <fieldset className="cm-fieldset" style={{ marginBottom: 0 }}>
          <legend>Edit signature</legend>
          <div className="cm-compose-sigs-tb">
            <select
              className="cm-input"
              style={{ width: 150 }}
              disabled={!current}
              value={FONTS.includes(fontFamily) ? fontFamily : ""}
              onChange={(e) => editor?.chain().focus().setFontFamily(e.target.value).run()}
            >
              <option value="">Calibri (Body)</option>
              {FONTS.map((f) => (
                <option key={f}>{f}</option>
              ))}
            </select>
            <select
              className="cm-input"
              style={{ width: 52 }}
              disabled={!current}
              onChange={(e) => editor?.chain().focus().setFontSize(`${e.target.value}pt`).run()}
              defaultValue="11"
            >
              {FONT_SIZES.map((s) => (
                <option key={s}>{s}</option>
              ))}
            </select>
            {(
              [
                ["bold", () => editor?.chain().focus().toggleBold().run(), "bold"],
                ["italic", () => editor?.chain().focus().toggleItalic().run(), "italic"],
                ["underline", () => editor?.chain().focus().toggleUnderline().run(), "underline"],
              ] as const
            ).map(([icon, fn, mark]) => (
              <button key={icon} className={`cm-rb-small${editor?.isActive(mark) ? " pressed" : ""}`} disabled={!current} onMouseDown={(e) => e.preventDefault()} onClick={fn}>
                <CIcon name={icon} />
              </button>
            ))}
            <button className="cm-rb-small" disabled={!current} onMouseDown={(e) => e.preventDefault()} onClick={(e) => setPalette(e.currentTarget.getBoundingClientRect())}>
              <CIcon name="font-color" />
            </button>
            {(["left", "center", "right"] as const).map((a) => (
              <button
                key={a}
                className={`cm-rb-small${editor?.isActive({ textAlign: a }) ? " pressed" : ""}`}
                disabled={!current}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => editor?.chain().focus().setTextAlign(a).run()}
              >
                <CIcon name={`align-${a}`} />
              </button>
            ))}
            <button className="cm-rb-small" disabled={!current} title="Insert Hyperlink" onMouseDown={(e) => e.preventDefault()} onClick={() => setLinkOpen(true)}>
              <CIcon name="link" />
            </button>
          </div>
          <div className={`cm-compose-sigedit${current ? "" : " disabled"}`} onClick={() => editor?.commands.focus()}>
            <EditorContent editor={editor} />
          </div>
        </fieldset>
      </div>
      {palette && (
        <ColorPalette
          kind="font"
          anchor={palette}
          onClose={() => setPalette(null)}
          onPick={(c) => (c ? editor?.chain().focus().setColor(c).run() : editor?.chain().focus().unsetColor().run())}
        />
      )}
      {linkOpen && editor && (
        <HyperlinkDialog
          initialText={editor.state.doc.textBetween(editor.state.selection.from, editor.state.selection.to, " ")}
          initialUrl={(editor.getAttributes("link").href as string) ?? ""}
          onClose={() => setLinkOpen(false)}
          onOk={(text, url) => {
            const { empty } = editor.state.selection;
            if (empty)
              editor
                .chain()
                .focus()
                .insertContent({ type: "text", text, marks: [{ type: "link", attrs: { href: url } }] })
                .run();
            else editor.chain().focus().extendMarkRange("link").setLink({ href: url }).run();
          }}
        />
      )}
      {naming && (
        <NameDialog
          title={naming === "new" ? "New Signature" : "Rename Signature"}
          initial={naming === "rename" ? (current?.name ?? "") : ""}
          onClose={() => setNaming(null)}
          onOk={async (name) => {
            try {
              if (naming === "new") {
                const s = await composeApi.saveSignature({ name, html: "" });
                await reload(s.id);
              } else if (current) {
                await composeApi.saveSignature({ id: current.id, name, html: editor?.getHTML() ?? current.html });
                setDirty(false);
                await reload(current.id);
              }
            } catch (e) {
              errorBox(e);
            }
          }}
        />
      )}
    </Dialog>
  );
}

/** "Stationery and Fonts": the default font for new messages (settings key mail.composeFont). */
export function ComposeFontDialog({ onClose }: { onClose: () => void }) {
  const [f, setF] = useState<ComposeFont>(DEFAULT_COMPOSE_FONT);
  useEffect(() => {
    rpc<ComposeFont | null>("settings.get", { key: COMPOSE_FONT_KEY })
      .then((v) => v && setF({ ...DEFAULT_COMPOSE_FONT, ...v }))
      .catch(() => {});
  }, []);
  const sample = useMemo(() => ({ fontFamily: `'${f.family}', sans-serif`, fontSize: `${f.size}pt` }), [f]);
  return (
    <Dialog
      title="Font"
      onClose={onClose}
      width={380}
      footer={
        <>
          <button className="cm-button default" onClick={() => rpc("settings.set", { key: COMPOSE_FONT_KEY, value: f }).then(onClose).catch(errorBox)}>
            OK
          </button>
          <button className="cm-button" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-form">
        <label>Font (new mail messages):</label>
        <select className="cm-input" value={f.family} onChange={(e) => setF({ ...f, family: e.target.value })}>
          {FONTS.map((x) => (
            <option key={x}>{x}</option>
          ))}
        </select>
        <label>Size:</label>
        <select className="cm-input" value={f.size} onChange={(e) => setF({ ...f, size: +e.target.value })} style={{ width: 70 }}>
          {FONT_SIZES.map((x) => (
            <option key={x} value={x}>
              {x}
            </option>
          ))}
        </select>
      </div>
      <fieldset className="cm-fieldset" style={{ marginTop: 10 }}>
        <legend>Preview</legend>
        <div style={{ ...sample, background: "var(--panel-bg)", padding: 8, minHeight: 40 }}>Sample text — טקסט לדוגמה</div>
      </fieldset>
    </Dialog>
  );
}

/**
 * Options › "Mail Format" tab body (for OPTION_EXTRA_TABS): Stationery and Fonts, Signatures,
 * and the New Mail Desktop Alert switch (settings `mail.desktopAlert`).
 */
export function MailFormatOptionsTab() {
  const [sigs, setSigs] = useState(false);
  const [fonts, setFonts] = useState(false);
  const [alert, setAlert] = useState(true);
  useEffect(() => {
    rpc<boolean | null>("settings.get", { key: "mail.desktopAlert" })
      .then((v) => setAlert(v !== false))
      .catch(() => {});
  }, []);
  return (
    <div>
      <fieldset className="cm-fieldset">
        <legend>Stationery and Fonts</legend>
        <div className="cm-compose-optrow">
          <span className="cm-fill">Change the default font for new messages.</span>
          <button className="cm-button" onClick={() => setFonts(true)}>
            Stationery and Fonts…
          </button>
        </div>
      </fieldset>
      <fieldset className="cm-fieldset">
        <legend>Signatures</legend>
        <div className="cm-compose-optrow">
          <span className="cm-fill">Create and edit signatures for outgoing messages, replies and forwards.</span>
          <button className="cm-button" onClick={() => setSigs(true)}>
            Signatures…
          </button>
        </div>
      </fieldset>
      <fieldset className="cm-fieldset">
        <legend>Message arrival</legend>
        <label className="cm-compose-check">
          <input
            type="checkbox"
            checked={alert}
            onChange={(e) => {
              setAlert(e.target.checked);
              rpc("settings.set", { key: "mail.desktopAlert", value: e.target.checked }).catch(errorBox);
            }}
          />
          Display a New Mail Desktop Alert
        </label>
      </fieldset>
      {sigs && <SignaturesDialog onClose={() => setSigs(false)} />}
      {fonts && <ComposeFontDialog onClose={() => setFonts(false)} />}
    </div>
  );
}
