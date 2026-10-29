// TipTap set-up for the Signatures dialog's editor (Word-mail look: no paragraph spacing, paragraphs
// `dir="auto"` so Hebrew lines run right-to-left), plus the font lists and text helpers the compose
// window shares. The compose page itself is a plain contentEditable (see body.ts).
import { Extension, type Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { TextStyleKit } from "@tiptap/extension-text-style";
import Highlight from "@tiptap/extension-highlight";
import TextAlign from "@tiptap/extension-text-align";
import Image from "@tiptap/extension-image";
import { TableKit } from "@tiptap/extension-table";
import { escapeHtml } from "../format";

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    indentBlock: {
      indent: () => ReturnType;
      outdent: () => ReturnType;
    };
  }
}

const INDENT_TYPES = ["paragraph", "heading"];
const INDENT_STEP_PT = 36; // 0.5"

/** dir="auto" on blocks + Word-like paragraph indent (Increase/Decrease Indent outside lists). */
export const BlockExtras = Extension.create({
  name: "blockExtras",
  addGlobalAttributes() {
    return [
      {
        types: INDENT_TYPES,
        attributes: {
          dir: {
            default: "auto",
            parseHTML: (el) => el.getAttribute("dir") || "auto",
            renderHTML: (attrs) => ({ dir: attrs.dir || "auto" }),
          },
          indent: {
            default: 0,
            parseHTML: (el) => {
              const m = /margin-(?:left|inline-start):\s*([\d.]+)pt/.exec(el.getAttribute("style") ?? "");
              return m ? Math.round(parseFloat(m[1]) / INDENT_STEP_PT) : 0;
            },
            renderHTML: (attrs) => (attrs.indent ? { style: `margin-left:${attrs.indent * INDENT_STEP_PT}pt` } : {}),
          },
        },
      },
    ];
  },
  addCommands() {
    const change =
      (delta: number) =>
      () =>
      ({ editor, chain }: { editor: Editor; chain: () => ReturnType<Editor["chain"]> }) => {
        if (editor.isActive("listItem")) {
          return delta > 0 ? chain().sinkListItem("listItem").run() : chain().liftListItem("listItem").run();
        }
        const { from, to } = editor.state.selection;
        let tr = editor.state.tr;
        let changed = false;
        editor.state.doc.nodesBetween(from, to, (node, pos) => {
          if (INDENT_TYPES.includes(node.type.name)) {
            const cur = (node.attrs.indent as number) || 0;
            const next = Math.max(0, Math.min(10, cur + delta));
            if (next !== cur) {
              tr = tr.setNodeMarkup(pos, undefined, { ...node.attrs, indent: next });
              changed = true;
            }
            return false;
          }
          return true;
        });
        if (changed) editor.view.dispatch(tr);
        return changed;
      };
    return {
      indent: change(1),
      outdent: change(-1),
    } as never;
  },
  addKeyboardShortcuts() {
    return {
      // Word / Outlook alignment shortcuts
      "Mod-l": () => this.editor.chain().focus().setTextAlign("left").run(),
      "Mod-e": () => this.editor.chain().focus().setTextAlign("center").run(),
      "Mod-r": () => this.editor.chain().focus().setTextAlign("right").run(),
      "Mod-j": () => this.editor.chain().focus().setTextAlign("justify").run(),
      "Mod-m": () => this.editor.commands.indent(),
      "Mod-Shift-m": () => this.editor.commands.outdent(),
      "Mod-]": () => stepFont(this.editor, 1),
      "Mod-[": () => stepFont(this.editor, -1),
      "Mod-Space": () => this.editor.chain().focus().unsetAllMarks().run(),
    };
  },
});

export const FONT_SIZES = [8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 26, 28, 36, 48, 72];
export const FONTS = [
  "Aptos",
  "Calibri",
  "Arial",
  "Arial Black",
  "Cambria",
  "Candara",
  "Comic Sans MS",
  "Consolas",
  "Constantia",
  "Corbel",
  "Courier New",
  "David",
  "Georgia",
  "Miriam",
  "Segoe UI",
  "Tahoma",
  "Times New Roman",
  "Trebuchet MS",
  "Verdana",
];

/** Ctrl+] / Ctrl+[ in the signature editor: the next size up or down the list. */
function stepFont(editor: Editor, dir: 1 | -1): boolean {
  const fs = editor.getAttributes("textStyle").fontSize as string | undefined;
  const m = /([\d.]+)(pt|px)?/.exec(fs ?? "");
  const cur = !m ? 11 : m[2] === "px" ? Math.round((parseFloat(m[1]) * 3) / 4) : parseFloat(m[1]);
  const next = dir > 0 ? (FONT_SIZES.find((s) => s > cur) ?? cur) : ([...FONT_SIZES].reverse().find((s) => s < cur) ?? cur);
  return editor.chain().focus().setFontSize(`${next}pt`).run();
}

/** A signature's table cells keep their inline formatting (shading, text colour and weight,
 *  vertical alignment, height, borders) through the editor. */
const CELL_STYLES = ["background-color", "color", "font-weight", "vertical-align", "height", "border-top", "border-right", "border-bottom", "border-left"];
const CellStyles = Extension.create({
  name: "cellStyles",
  addGlobalAttributes() {
    return [
      {
        types: ["tableCell", "tableHeader"],
        attributes: Object.fromEntries(
          CELL_STYLES.map((css) => {
            const attr = css.replace(/-(\w)/g, (_, c: string) => c.toUpperCase());
            return [
              attr,
              {
                default: null,
                parseHTML: (el: HTMLElement) => el.style.getPropertyValue(css) || null,
                renderHTML: (attrs: Record<string, unknown>) => (attrs[attr] ? { style: `${css}: ${String(attrs[attr])}` } : {}),
              },
            ];
          }),
        ),
      },
    ];
  },
});

/** The Signatures dialog's editor. */
export function extensions() {
  return [
    StarterKit.configure({
      link: { openOnClick: false, autolink: true, linkOnPaste: true, HTMLAttributes: { rel: null, target: null } },
      heading: { levels: [1, 2, 3] },
    }),
    TextStyleKit.configure({ lineHeight: false }),
    Highlight.configure({ multicolor: true }),
    TextAlign.configure({ types: ["heading", "paragraph"] }),
    Image.configure({ inline: true, allowBase64: true }),
    TableKit.configure({ table: { resizable: true, cellMinWidth: 20, HTMLAttributes: { border: "1", cellpadding: "4", style: "border-collapse:collapse" } } }),
    BlockExtras,
    CellStyles,
  ];
}

/** Plain text of HTML (for Plain Text format switch). */
export function htmlToText(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.querySelectorAll("br").forEach((br) => br.replaceWith("\n"));
  doc.querySelectorAll("td, th").forEach((cell) => cell.append("\t"));
  doc.querySelectorAll("p, div, li, tr, h1, h2, h3").forEach((el) => el.append("\n"));
  return (doc.body.textContent ?? "")
    .replace(/\u00a0/g, " ")
    .replace(/\n{3,}/g, "\n\n")
    .trimEnd();
}

export function textToHtml(text: string): string {
  return text
    .split("\n")
    .map((l) => `<p>${escapeHtml(l)}</p>`)
    .join("");
}

// also here for the modules that import it with the other text helpers
export { escapeHtml };
