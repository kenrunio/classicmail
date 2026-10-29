// Markdown in a message: written (Format › Markdown), pasted (an answer copied from an AI chat,
// a README) or inserted from a .md file (Include › Web Page or Markdown). It never goes out as
// Markdown: mail programs show it as raw text with its stars and hashes, so it is turned into
// ordinary e-mail formatting — headings, bold, lists, tables with visible lines, links — and the
// Markdown itself is the message's plain-text version.
import { marked } from "marked";
import TurndownService from "turndown";

/** Markdown signs, one per kind: two kinds (or a table / fenced code alone) make a text Markdown. */
const SIGNS: RegExp[] = [
  /^#{1,6} \S/m, // heading
  /^\s*[-*+] \S.*\n\s*[-*+] \S/m, // bulleted list (two items)
  /^\s*\d+[.)] \S.*\n\s*\d+[.)] \S/m, // numbered list (two items)
  /\*\*[^*\n]+\*\*|__[^_\n]+__/, // bold
  /\[[^\]\n]+\]\((https?:|mailto:|#)[^)\s]*\)/, // link
  /^>\s?\S/m, // quote
  /`[^`\n]+`/, // inline code
];
const STRONG: RegExp[] = [
  /^\|?.*\|.*\n\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$/m, // table
  /^```/m, // fenced code
];

/** Does this pasted / typed text look like Markdown (and not like an ordinary note)? */
export function looksLikeMarkdown(text: string): boolean {
  if (STRONG.some((r) => r.test(text))) return true;
  return SIGNS.filter((r) => r.test(text)).length >= 2;
}

/** Markdown as e-mail HTML: tables with visible lines, code in a monospace box, paragraphs apart,
 *  each block in its own text direction; nothing that runs (scripts, frames, event handlers). */
export function markdownToHtml(md: string): string {
  const raw = marked.parse(md, { gfm: true, breaks: false, async: false }) as string;
  const doc = new DOMParser().parseFromString(`<body>${raw}</body>`, "text/html");
  const body = doc.body;
  body.querySelectorAll("script, style, iframe, object, embed, form, input, button").forEach((e) => e.remove());
  body.querySelectorAll("*").forEach((e) => {
    for (const a of Array.from(e.attributes)) {
      if (/^on/i.test(a.name) || (/^(href|src)$/i.test(a.name) && /^\s*javascript:/i.test(a.value))) e.removeAttribute(a.name);
    }
  });
  const style = (sel: string, css: string) => body.querySelectorAll<HTMLElement>(sel).forEach((e) => e.setAttribute("style", `${css}${e.getAttribute("style") ?? ""}`));
  style("p", "margin:0 0 10px;");
  style("table", "border-collapse:collapse;margin:0 0 10px;");
  style("th, td", "border:1px solid #a6a6a6;padding:3px 7px;vertical-align:top;");
  style("th", "background-color:#f2f2f2;text-align:left;");
  style("pre", "background-color:#f4f4f4;border:1px solid #e0e0e0;padding:8px;font-family:Consolas,'Courier New',monospace;font-size:10pt;white-space:pre-wrap;margin:0 0 10px;");
  body.querySelectorAll<HTMLElement>("code").forEach((c) => {
    if (c.parentElement?.tagName !== "PRE") c.setAttribute("style", "background-color:#f4f4f4;padding:0 3px;font-family:Consolas,'Courier New',monospace;");
  });
  style("blockquote", "margin:0 0 10px;padding:0 0 0 10px;border-left:3px solid #c8c8c8;color:#555;");
  body.querySelectorAll("p, h1, h2, h3, h4, h5, h6, ul, ol, blockquote, table").forEach((b) => b.setAttribute("dir", "auto"));
  return body.innerHTML;
}

/** E-mail HTML as Markdown (switching a message to the Markdown format). */
export function htmlToMarkdown(html: string): string {
  const td = new TurndownService({ headingStyle: "atx", bulletListMarker: "-", codeBlockStyle: "fenced", emDelimiter: "*" });
  td.remove(["style", "script", "title", "meta", "link"]);
  // a table as Markdown rows (one line per row, cells between bars; the first row its header)
  td.addRule("table", {
    filter: "table",
    replacement: (_content, node) => {
      const rows = Array.from((node as HTMLTableElement).rows).map((r) => Array.from(r.cells).map((c) => (c.textContent ?? "").replace(/\s+/g, " ").replace(/\|/g, "\\|").trim()));
      if (!rows.length) return "";
      const width = Math.max(...rows.map((r) => r.length));
      const line = (r: string[]) => `| ${Array.from({ length: width }, (_, i) => r[i] ?? "").join(" | ")} |`;
      return `\n\n${[line(rows[0]), `|${" --- |".repeat(width)}`, ...rows.slice(1).map(line)].join("\n")}\n\n`;
    },
  });
  return (
    td
      .turndown(html)
      // an empty paragraph leaves a line of spaces (or a lone line-break mark): just a blank line
      .replace(/^[ \t\u00a0]+$/gm, "")
      .replace(/\n{3,}/g, "\n\n")
      .trim()
  );
}
