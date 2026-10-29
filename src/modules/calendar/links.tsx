// Web addresses in appointment text (notes, Zoom / Meet details, Location) as links that open in
// the browser, as in Outlook.
import { rpc } from "../../api/rpc";
import { errorBox } from "../../ui";

const URL_RE = /\b(?:https?:\/\/|www\.)[^\s<>"]+/gi;

/** Text split around its web addresses; a link's trailing punctuation stays text. */
export function linkParts(text: string): { text: string; url?: string }[] {
  const out: { text: string; url?: string }[] = [];
  let last = 0;
  for (const m of text.matchAll(URL_RE)) {
    const raw = m[0].replace(/[.,;:!?)\]'’]+$/, "");
    const at = m.index ?? 0;
    if (at > last) out.push({ text: text.slice(last, at) });
    out.push({ text: raw, url: /^www\./i.test(raw) ? `https://${raw}` : raw });
    last = at + raw.length;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}

/** The first web address in the text (a Location "https://zoom.us/j/…"), if any. */
export const firstUrl = (text: string): string | undefined => linkParts(text).find((p) => p.url)?.url;

export const hasUrl = (text: string) => firstUrl(text) !== undefined;

export const openLink = (url: string) => void rpc("app.openUrl", { url }).catch(errorBox);

/** The text with clickable links (a click on a link opens it, and doesn't start editing). */
export function Linkified({ text }: { text: string }) {
  return (
    <>
      {linkParts(text).map((p, i) =>
        p.url ? (
          <a
            key={i}
            href={p.url}
            className="cm-cal-link"
            title={`Open ${p.url}`}
            onClick={(e) => {
              e.preventDefault();
              e.stopPropagation();
              openLink(p.url!);
            }}
          >
            {p.text}
          </a>
        ) : (
          p.text
        ),
      )}
    </>
  );
}
