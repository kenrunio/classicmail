// File › Print for contacts: a "Phone Directory Style" printout in a hidden frame.
import { indexKey, phonesByLabel, type Contact } from "../../api/contacts";

const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

export function printContacts(contacts: Contact[], title = "Contacts") {
  const groups = new Map<string, Contact[]>();
  for (const c of contacts) {
    const k = indexKey(c.fileAs).toUpperCase();
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k)!.push(c);
  }
  const body = [...groups.entries()]
    .map(
      ([k, list]) =>
        `<h2>${esc(k)}</h2><table>${list
          .map((c) => {
            const phones = Object.entries(phonesByLabel(c.phones))
              .map(([l, v]) => `${esc(v)} <span class="l">${esc(l)}</span>`)
              .join("<br>");
            return `<tr><td class="n" dir="auto">${esc(c.fileAs || c.displayName)}${c.company && c.company !== c.fileAs ? `<br><span class="l">${esc(c.company)}</span>` : ""}</td><td class="p">${phones}</td></tr>`;
          })
          .join("")}</table>`,
    )
    .join("");
  const html = `<!doctype html><html><head><meta charset="utf-8"><title>${esc(title)}</title><style>
    body{font-family:"Segoe UI",Tahoma,sans-serif;font-size:10pt;margin:16px}
    h1{font-size:16pt;border-bottom:2px solid #000;margin:0 0 8px}
    h2{font-size:12pt;background:#ddd;padding:2px 6px;margin:12px 0 4px}
    table{width:100%;border-collapse:collapse} td{vertical-align:top;padding:3px 6px;border-bottom:1px dotted #999}
    td.n{width:45%;font-weight:600} .l{color:#555;font-weight:normal;font-size:9pt}
  </style></head><body><h1>${esc(title)}</h1>${body}</body></html>`;
  const frame = document.createElement("iframe");
  frame.style.cssText = "position:fixed;width:0;height:0;border:0;left:-1000px";
  document.body.appendChild(frame);
  const doc = frame.contentDocument!;
  doc.open();
  doc.write(html);
  doc.close();
  setTimeout(() => {
    frame.contentWindow?.focus();
    frame.contentWindow?.print();
    setTimeout(() => frame.remove(), 1000);
  }, 50);
}
