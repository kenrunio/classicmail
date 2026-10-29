// Pictures don't go to the AI (Translate, AI Review): a pasted screenshot is tens of kilobytes of
// base64 that the model would have to read and write back unchanged — slow, costly, and it can
// run past the answer's length limit. Each picture's src becomes a short "cid:" reference (one the
// back end's HTML cleaning keeps) and the real picture goes back in afterwards.

const REF = "cid:cm-img-";

function parse(html: string): HTMLElement {
  const d = document.createElement("div");
  d.innerHTML = html;
  return d;
}

/** The HTML with every picture's source replaced, and how to put them back in the AI's answer. */
export function withoutImages(html: string): { html: string; restore: (answer: string) => string } {
  const root = parse(html);
  const srcs: string[] = [];
  root.querySelectorAll("img").forEach((img) => {
    srcs.push(img.getAttribute("src") ?? "");
    img.setAttribute("src", `${REF}${srcs.length - 1}`);
    img.removeAttribute("srcset");
  });
  if (!srcs.length) return { html, restore: (a) => a };
  return {
    html: root.innerHTML,
    restore: (answer) => {
      const out = parse(answer);
      const used = new Set<number>();
      out.querySelectorAll("img").forEach((img) => {
        const src = img.getAttribute("src") ?? "";
        const n = src.startsWith(REF) ? Number(src.slice(REF.length)) : NaN;
        if (Number.isInteger(n) && srcs[n] !== undefined) {
          img.setAttribute("src", srcs[n]);
          used.add(n);
        }
      });
      // a picture the model left out isn't lost: it goes at the end
      for (let i = 0; i < srcs.length; i++) {
        if (used.has(i)) continue;
        const p = document.createElement("p");
        const img = document.createElement("img");
        img.setAttribute("src", srcs[i]);
        p.appendChild(img);
        out.appendChild(p);
      }
      return out.innerHTML;
    },
  };
}
