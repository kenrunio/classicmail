// Printing a page of our own (Quick Print of an appointment, a task list) from a hidden frame.
import { escapeHtml } from "../mail/format";

/**
 * Prints `body` (HTML, already escaped by the caller) as a page titled `title` and styled by
 * `css`. The frame's CSP allows inline styles only: no scripts, no remote content.
 */
export function printPage(title: string, css: string, body: string) {
  const html =
    `<!doctype html><html><head><meta charset="utf-8">` +
    `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'">` +
    `<title>${escapeHtml(title)}</title><style>${css}</style></head><body>${body}</body></html>`;
  const f = document.createElement("iframe");
  f.style.cssText = "position:fixed;right:0;bottom:0;width:0;height:0;border:0;visibility:hidden";
  document.body.appendChild(f);
  const doc = f.contentDocument!;
  doc.open();
  doc.write(html);
  doc.close();
  setTimeout(() => {
    try {
      f.contentWindow?.focus();
      f.contentWindow?.print();
    } finally {
      setTimeout(() => f.remove(), 2000);
    }
  }, 100);
}
