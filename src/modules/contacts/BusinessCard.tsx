// Outlook 2007-style electronic business card (Business Cards view, contact form preview).
import { addressLines, mainAddress, phoneLabel, type Contact } from "../../api/contacts";

const PHONE_ORDER = ["work", "mobile", "home", "main", "workFax"];
const SHORT: Record<string, string> = { work: "Work", mobile: "Mobile", home: "Home", main: "Main", workFax: "Work Fax" };

/** Lines below name / company / title, in the order Outlook's default card shows them. */
export function cardLines(c: Contact, max = 6): string[] {
  const lines: string[] = [];
  const phones = [...c.phones].sort((a, b) => rank(a.type) - rank(b.type)).slice(0, 3);
  for (const p of phones) lines.push(`${p.value} ${SHORT[p.type] ?? phoneLabel(p.type)}`);
  if (c.emails[0]) lines.push(c.emails[0].value);
  const addr = addressLines(mainAddress(c));
  if (addr.length) lines.push(...addr.slice(0, 2));
  if (c.urls[0]) lines.push(c.urls[0].value.replace(/^https?:\/\//, ""));
  return lines.slice(0, max);
}

function rank(t: string) {
  const i = PHONE_ORDER.indexOf(t);
  return i < 0 ? 99 : i;
}

export function Silhouette({ size = 64 }: { size?: number }) {
  return (
    <svg className="cm-contacts-silhouette" width={size} height={size} viewBox="0 0 64 64" aria-hidden>
      <rect x="0.5" y="0.5" width="63" height="63" fill="var(--panel-alt)" stroke="var(--frame-light)" />
      <circle cx="32" cy="25" r="11" fill="var(--frame-light)" />
      <path d="M12 60c2-13 10-19 20-19s18 6 20 19z" fill="var(--frame-light)" />
    </svg>
  );
}

export function BusinessCard({
  contact,
  selected,
  focused,
  className,
  onMouseDown,
  onDoubleClick,
  onContextMenu,
}: {
  contact: Contact;
  selected?: boolean;
  focused?: boolean;
  className?: string;
  onMouseDown?: (e: React.MouseEvent) => void;
  onDoubleClick?: (e: React.MouseEvent) => void;
  onContextMenu?: (e: React.MouseEvent) => void;
}) {
  const c = contact;
  const name = c.displayName || c.fileAs;
  const showCompany = c.company && c.company !== name;
  return (
    <div
      className={`cm-contacts-bcard${selected ? " selected" : ""}${focused ? " focused" : ""} ${className ?? ""}`}
      data-id={c.id}
      draggable
      onMouseDown={onMouseDown}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      <div className="cm-contacts-bcard-band" />
      <div className="cm-contacts-bcard-body">
        <div className="cm-contacts-bcard-pic">{c.photoUrl ? <img src={c.photoUrl} alt="" draggable={false} /> : <Silhouette />}</div>
        <div className="cm-contacts-bcard-text" dir="auto">
          <div className="cm-contacts-bcard-name cm-ellipsis">{name}</div>
          {showCompany && <div className="cm-contacts-bcard-company cm-ellipsis">{c.company}</div>}
          {c.jobTitle && <div className="cm-ellipsis">{c.jobTitle}</div>}
          <div className="cm-contacts-bcard-gap" />
          {cardLines(c, 4 + (showCompany ? 0 : 1) + (c.jobTitle ? 0 : 1)).map((l, i) => (
            <div key={i} className="cm-ellipsis cm-contacts-bcard-line">
              {l}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
