// Our own 16×16 icon set in the spirit of Office 2007 (no Microsoft artwork).
// <Icon name="reply" /> — `size` scales the same drawing (e.g. 24/32 for big buttons).
import type { JSX } from "react";

const env = (flap = "#7f9fd0", body = "#fdfdfd", border = "#5579b8") => (
  <>
    <rect x="1.5" y="3.5" width="13" height="9" rx="0.5" fill={body} stroke={border} />
    <path d="M1.8 3.8 8 9l6.2-5.2" fill="none" stroke={flap} />
  </>
);

const folder = (fill = "#f6d77a", stroke = "#c19a33") => (
  <>
    <path d="M1.5 4.5v8h13v-7H7.5l-1.5-1.5h-4.5z" fill={fill} stroke={stroke} />
    <path d="M1.5 6.5h13" stroke="#fff3c4" />
  </>
);

const arrowLeft = (color: string) => (
  <path d="M7 3 2.5 7.5 7 12V9.3c3 0 5 .8 6.5 3.2-.4-3.8-2.6-6-6.5-6.4V3z" fill={color} stroke="#1d5c1d" strokeWidth=".6" strokeLinejoin="round" />
);

export const ICONS: Record<string, JSX.Element> = {
  mail: env(),
  "mail-unread": env("#5579b8", "#fffbe8", "#b08d2b"),
  "mail-read": (
    <>
      <path d="M1.5 7.5 8 3l6.5 4.5v6h-13z" fill="#fdfdfd" stroke="#8d8d8d" />
      <path d="M1.8 7.8 8 11.5l6.2-3.7" fill="none" stroke="#a6a6a6" />
    </>
  ),
  "mail-replied": (
    <>
      <path d="M3.5 7.5 8 4.5l6.5 4v5h-11z" fill="#fdfdfd" stroke="#8d8d8d" />
      <path d="M5 2 1 5l4 3V6.2c2 0 3.2.5 4.2 1.8C9 5.6 7.5 4.3 5 4z" fill="#7a3fb4" />
    </>
  ),
  "mail-forwarded": (
    <>
      <path d="M1.5 7.5 6 4.5l6.5 4v5h-11z" fill="#fdfdfd" stroke="#8d8d8d" />
      <path d="M11 2l4 3-4 3V6.2c-2 0-3.2.5-4.2 1.8C7 5.6 8.5 4.3 11 4z" fill="#1f7ac8" />
    </>
  ),
  "new-mail": (
    <>
      {env()}
      <path d="M12 0.5l.9 2 2 .9-2 .9-.9 2-.9-2-2-.9 2-.9z" fill="#f2b51b" stroke="#c48a07" strokeWidth=".5" />
    </>
  ),
  // mail merge: a stack of envelopes with a list
  "mail-merge": (
    <>
      <rect x="4.5" y="1.5" width="10" height="7" rx="0.5" fill="#eef3fb" stroke="#7f9fd0" />
      <rect x="3" y="3.5" width="10" height="7" rx="0.5" fill="#f6f9fd" stroke="#6b8cc4" />
      <rect x="1.5" y="5.5" width="10" height="7" rx="0.5" fill="#fdfdfd" stroke="#5579b8" />
      <path d="M1.8 5.8 6.5 9.5l4.7-3.7" fill="none" stroke="#7f9fd0" />
      <rect x="10.5" y="9.5" width="5" height="6" fill="#fff" stroke="#3c8a3c" />
      <path d="M11.5 11.5h3M11.5 13h3M11.5 14.5h2" stroke="#3c8a3c" strokeWidth=".8" />
    </>
  ),
  calendar: (
    <>
      <rect x="1.5" y="2.5" width="13" height="12" fill="#fff" stroke="#5579b8" />
      <rect x="1.5" y="2.5" width="13" height="3" fill="#cc3b3b" stroke="#a42424" />
      <path d="M4 8h2M7 8h2M10 8h2M4 10.5h2M7 10.5h2M10 10.5h2M4 13h2M7 13h2" stroke="#7d93b5" />
      <rect x="4" y="1" width="1.4" height="3" fill="#6c6c6c" />
      <rect x="10.6" y="1" width="1.4" height="3" fill="#6c6c6c" />
    </>
  ),
  contacts: (
    <>
      <rect x="2.5" y="1.5" width="11" height="13" rx="1" fill="#f6efdc" stroke="#9c7c3c" />
      <circle cx="8" cy="6.2" r="2.3" fill="#e9b984" stroke="#a46d36" strokeWidth=".6" />
      <path d="M4 12.5c.5-2.4 2-3.3 4-3.3s3.5.9 4 3.3z" fill="#3c76c4" />
    </>
  ),
  contact: (
    <>
      <circle cx="8" cy="5.5" r="3" fill="#e9b984" stroke="#a46d36" strokeWidth=".7" />
      <path d="M2.5 15c.4-3.6 2.6-5.3 5.5-5.3s5.1 1.7 5.5 5.3z" fill="#3c76c4" stroke="#24518e" strokeWidth=".6" />
    </>
  ),
  "contact-group": (
    <>
      <circle cx="5.5" cy="5" r="2.3" fill="#e9b984" stroke="#a46d36" strokeWidth=".6" />
      <circle cx="10.5" cy="5.5" r="2.3" fill="#f0c999" stroke="#a46d36" strokeWidth=".6" />
      <path d="M1 13c.3-2.7 2-4 4.5-4s4.2 1.3 4.5 4z" fill="#3c76c4" />
      <path d="M6.5 14c.3-2.7 1.8-4 4-4s3.7 1.3 4 4z" fill="#59a14a" />
    </>
  ),
  tasks: (
    <>
      <rect x="2.5" y="1.5" width="11" height="13" fill="#fff" stroke="#5579b8" />
      <path d="M4 5l1.2 1.2L7.5 3.8" fill="none" stroke="#2f9a2f" strokeWidth="1.3" />
      <path d="M4 10l1.2 1.2 2.3-2.4" fill="none" stroke="#2f9a2f" strokeWidth="1.3" />
      <path d="M9 5.5h3M9 10.5h3" stroke="#7d93b5" />
    </>
  ),
  // clipboard (not a ticked box — that would read as "completed")
  task: (
    <>
      <rect x="2.5" y="2.5" width="11" height="12" rx="1" fill="#c99a52" stroke="#7a5a26" />
      <rect x="4" y="4" width="8" height="9.5" fill="#fff" />
      <rect x="5.5" y="1.5" width="5" height="2.5" rx=".8" fill="#d9d9d9" stroke="#666" strokeWidth=".7" />
      <path d="M5 8.5l1.4 1.4 3.2-3.3" fill="none" stroke="#2f9a2f" strokeWidth="1.3" />
      <path d="M5 12h5" stroke="#9aa8bd" />
    </>
  ),
  notes: (
    <>
      <path d="M2.5 2.5h11v8l-3 3h-8z" fill="#fff4a8" stroke="#bfa431" />
      <path d="M10.5 13.5v-3h3" fill="#f0de6e" stroke="#bfa431" />
    </>
  ),
  "folder-list": (
    <>
      {folder()}
      <path d="M4 9h8M4 11h6" stroke="#c19a33" />
    </>
  ),
  shortcuts: <path d="M3 13 11 5M11 5H6M11 5v5" fill="none" stroke="#2c62b4" strokeWidth="2" />,
  folder: folder(),
  "folder-open": (
    <>
      <path d="M1.5 4.5v8h11l2-6h-7.5l-1.5-1.5h-4z" fill="#f6d77a" stroke="#c19a33" />
      <path d="M1.5 12.5l2-5.5h11" fill="#fbe7a4" stroke="#c19a33" />
    </>
  ),
  inbox: (
    <>
      <path d="M1.5 9.5l2-6h9l2 6v4h-13z" fill="#fff" stroke="#5579b8" />
      <path d="M1.5 9.5h4l1 1.5h3l1-1.5h4" fill="#cfe0f7" stroke="#5579b8" />
    </>
  ),
  sent: (
    <>
      {folder()}
      <path d="M6 7.5h6l-2.2-1.8M12 7.5l-2.2 1.8" fill="none" stroke="#2c62b4" strokeWidth="1.2" />
    </>
  ),
  drafts: (
    <>
      {folder()}
      <path d="M6 11.5l5-5 1 1-5 5H6z" fill="#e7c063" stroke="#6b5a2a" strokeWidth=".7" />
    </>
  ),
  deleted: (
    <>
      <path d="M3.5 4.5h9l-1 10h-7z" fill="#e9eef5" stroke="#6d7c92" />
      <rect x="2.5" y="2.5" width="11" height="2" fill="#cdd6e3" stroke="#6d7c92" />
      <path d="M6 6.5v6M8 6.5v6M10 6.5v6" stroke="#8b99ad" />
    </>
  ),
  junk: (
    <>
      {folder()}
      <circle cx="9.5" cy="9.5" r="2.8" fill="#fff" stroke="#c43030" />
      <path d="M7.5 11.5l4-4" stroke="#c43030" />
    </>
  ),
  outbox: (
    <>
      <path d="M1.5 9.5l2-6h9l2 6v4h-13z" fill="#fff" stroke="#5579b8" />
      <path d="M8 2v6M5.5 4.5 8 2l2.5 2.5" fill="none" stroke="#2f9a2f" strokeWidth="1.3" />
    </>
  ),
  archive: (
    <>
      <rect x="1.5" y="2.5" width="13" height="3" fill="#d8c08a" stroke="#8c7340" />
      <path d="M2.5 5.5h11v8h-11z" fill="#eadbb2" stroke="#8c7340" />
      <path d="M6 8h4" stroke="#8c7340" strokeWidth="1.4" />
    </>
  ),
  "search-folder": (
    <>
      {folder()}
      <circle cx="9" cy="9" r="2.3" fill="#e8f2ff" stroke="#2c62b4" />
      <path d="M10.6 10.6l2.4 2.4" stroke="#2c62b4" strokeWidth="1.5" />
    </>
  ),
  mailbox: (
    <>
      <rect x="1.5" y="4.5" width="13" height="9" fill="#dbe7f8" stroke="#5579b8" />
      <path d="M1.5 4.5 4 1.5h8l2.5 3" fill="#c5d8f3" stroke="#5579b8" />
      <rect x="5" y="7" width="6" height="3" fill="#fff" stroke="#5579b8" />
    </>
  ),
  new: (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#5579b8" />
      <path d="M9.5 1.5v3h3" fill="#dbe7f8" stroke="#5579b8" />
    </>
  ),
  print: (
    <>
      <rect x="4.5" y="1.5" width="7" height="4" fill="#fff" stroke="#5c5c5c" />
      <rect x="1.5" y="5.5" width="13" height="6" rx="1" fill="#b8c4d4" stroke="#5c5c5c" />
      <rect x="4.5" y="9.5" width="7" height="5" fill="#fff" stroke="#5c5c5c" />
      <circle cx="12.5" cy="7.5" r=".8" fill="#2f9a2f" />
    </>
  ),
  move: (
    <>
      {folder()}
      <path d="M5 10h5.5M8.5 8l2.2 2-2.2 2" fill="none" stroke="#2c62b4" strokeWidth="1.3" />
    </>
  ),
  delete: <path d="M3 3l10 10M13 3 3 13" stroke="#2b2b2b" strokeWidth="2.4" strokeLinecap="round" />,
  "delete-red": <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" stroke="#d13434" strokeWidth="2.2" strokeLinecap="round" />,
  reply: <>{arrowLeft("#4caf50")}</>,
  // AI: a four-point sparkle
  ai: (
    <>
      <path d="M8 1.2 9.4 6.6 14.8 8 9.4 9.4 8 14.8 6.6 9.4 1.2 8 6.6 6.6z" fill="#8a5cf6" stroke="#5b35c4" strokeWidth=".6" strokeLinejoin="round" />
      <path d="M13 1.5l.5 1.5 1.5.5-1.5.5-.5 1.5-.5-1.5-1.5-.5 1.5-.5z" fill="#f2b51b" />
    </>
  ),
  "reply-ai": (
    <>
      {arrowLeft("#4caf50")}
      <path d="M12.5 .8l.8 2.4 2.4.8-2.4.8-.8 2.4-.8-2.4-2.4-.8 2.4-.8z" fill="#8a5cf6" stroke="#5b35c4" strokeWidth=".4" />
    </>
  ),
  // translate: "A" and a Hebrew letter on two overlapping cards
  translate: (
    <>
      <rect x="1.5" y="1.5" width="8" height="8" rx="1" fill="#e8f0fb" stroke="#5579b8" />
      <path d="M3.3 7.6 5.5 3l2.2 4.6M4.1 6h2.8" fill="none" stroke="#2858a5" strokeWidth=".9" />
      <rect x="6.5" y="6.5" width="8" height="8" rx="1" fill="#fff" stroke="#3c8a3c" />
      <path d="M8.6 8.6h3.6v4.2M8.6 12.8h3.9" fill="none" stroke="#2e7d32" strokeWidth=".9" />
    </>
  ),
  "reply-all": (
    <>
      <path d="M6 3 1.5 7.5 6 12" fill="none" stroke="#2e7d32" strokeWidth="1.2" strokeLinejoin="round" />
      <path d="M9 3 4.5 7.5 9 12V9.3c3 0 4.6.8 5.8 3.2-.4-3.8-2.2-6-5.8-6.4V3z" fill="#4caf50" stroke="#1d5c1d" strokeWidth=".6" strokeLinejoin="round" />
    </>
  ),
  forward: <path d="M9 3l4.5 4.5L9 12V9.3c-3 0-5 .8-6.5 3.2.4-3.8 2.6-6 6.5-6.4V3z" fill="#2f7fd3" stroke="#174c8a" strokeWidth=".6" strokeLinejoin="round" />,
  "send-receive": (
    <>
      <path d="M2 6.5 6 2.5v2.5h5v3H6v2.5z" fill="#2f9a2f" stroke="#1d5c1d" strokeWidth=".6" />
      <path d="M14 9.5 10 13.5V11H5V8h5V5.5z" fill="#2f7fd3" stroke="#174c8a" strokeWidth=".6" />
    </>
  ),
  find: (
    <>
      <circle cx="6.5" cy="6.5" r="4" fill="#e8f2ff" stroke="#2c62b4" strokeWidth="1.3" />
      <path d="M9.5 9.5l4.5 4.5" stroke="#6b4a1e" strokeWidth="2.4" strokeLinecap="round" />
    </>
  ),
  "address-book": (
    <>
      <rect x="2.5" y="1.5" width="11" height="13" fill="#c8483b" stroke="#86271e" />
      <rect x="4.5" y="3.5" width="7" height="9" fill="#fff6e6" />
      <circle cx="8" cy="6.5" r="1.6" fill="#a46d36" />
      <path d="M5.5 11c.3-1.6 1.2-2.4 2.5-2.4s2.2.8 2.5 2.4z" fill="#3c76c4" />
    </>
  ),
  flag: (
    <>
      <path d="M3.5 1.5v13" stroke="#555" strokeWidth="1.2" />
      <path d="M4 2h9l-2 3 2 3H4z" fill="#e0332b" stroke="#a11d17" strokeWidth=".6" />
    </>
  ),
  "flag-outline": (
    <>
      <path d="M3.5 1.5v13" stroke="#b5b5b5" strokeWidth="1.2" />
      <path d="M4 2h9l-2 3 2 3H4z" fill="none" stroke="#c4c4c4" strokeWidth=".8" />
    </>
  ),
  "flag-complete": <path d="M2.5 8.5l3.5 3.5 7.5-8" fill="none" stroke="#2f9a2f" strokeWidth="2.2" />,
  attachment: <path d="M10.5 4.5v6a2.5 2.5 0 0 1-5 0v-7a1.6 1.6 0 0 1 3.2 0v6.5a.7.7 0 0 1-1.4 0V5" fill="none" stroke="#555" strokeWidth="1.1" strokeLinecap="round" />,
  "importance-high": <path d="M7 2h2l-.3 8H7.3zM7 12h2v2H7z" fill="#d13434" />,
  "importance-low": <path d="M8 13 4 8h2.5V3h3v5H12z" fill="#2c62b4" />,
  appointment: (
    <>
      <rect x="1.5" y="2.5" width="13" height="12" fill="#fff" stroke="#5579b8" />
      <rect x="1.5" y="2.5" width="13" height="3" fill="#5f8fd1" stroke="#3a64a3" />
      <circle cx="10" cy="10.5" r="3" fill="#fff" stroke="#3a64a3" />
      <path d="M10 8.8v1.8l1.2.8" fill="none" stroke="#3a64a3" />
    </>
  ),
  meeting: (
    <>
      <rect x="1.5" y="2.5" width="13" height="12" fill="#fff" stroke="#5579b8" />
      <rect x="1.5" y="2.5" width="13" height="3" fill="#5f8fd1" stroke="#3a64a3" />
      <circle cx="6" cy="9" r="1.6" fill="#e9b984" />
      <circle cx="10.5" cy="9" r="1.6" fill="#e9b984" />
      <path d="M3.5 13.5c.3-1.6 1.2-2.4 2.5-2.4s2.2.8 2.5 2.4zM8 13.5c.3-1.6 1.2-2.4 2.5-2.4s2.2.8 2.5 2.4z" fill="#3c76c4" />
    </>
  ),
  "meeting-request": (
    <>
      {env()}
      <rect x="8.5" y="7.5" width="7" height="7" fill="#fff" stroke="#3a64a3" />
      <rect x="8.5" y="7.5" width="7" height="2" fill="#5f8fd1" />
    </>
  ),
  recurring: <path d="M12.5 6A5 5 0 0 0 3.6 5M3.5 10a5 5 0 0 0 8.9 1M3 2.5V5.5h3M13 13.5v-3h-3" fill="none" stroke="#2c62b4" strokeWidth="1.3" />,
  reminder: (
    <>
      <path d="M8 2a4 4 0 0 1 4 4v3.5l1.5 2h-11L4 9.5V6a4 4 0 0 1 4-4z" fill="#f4c542" stroke="#a3801d" />
      <circle cx="8" cy="13" r="1.4" fill="#a3801d" />
    </>
  ),
  private: (
    <>
      <rect x="3.5" y="7.5" width="9" height="7" rx="1" fill="#e2b33a" stroke="#946d12" />
      <path d="M5.5 7.5V5.5a2.5 2.5 0 0 1 5 0v2" fill="none" stroke="#7a7a7a" strokeWidth="1.4" />
    </>
  ),
  "show-as": (
    <>
      <rect x="2.5" y="3.5" width="11" height="9" fill="#fff" stroke="#6b6b6b" />
      <rect x="3" y="4" width="3" height="8" fill="#3d2a8c" />
      <path d="M7.5 6.5h4.5M7.5 9.5h3" stroke="#9a9a9a" strokeWidth="1.2" />
    </>
  ),
  today: (
    <>
      <rect x="1.5" y="2.5" width="13" height="12" fill="#fff" stroke="#5579b8" />
      <rect x="1.5" y="2.5" width="13" height="3" fill="#e1a72a" stroke="#a3801d" />
      <text x="8" y="13" textAnchor="middle" fontSize="7" fontWeight="700" fill="#3a64a3" fontFamily="Segoe UI, Tahoma">
        {new Date().getDate()}
      </text>
    </>
  ),
  "view-day": (
    <>
      <rect x="2.5" y="2.5" width="11" height="11" fill="#fff" stroke="#5579b8" />
      <path d="M2.5 5.5h11" stroke="#5579b8" />
      <rect x="5" y="7" width="6" height="4" fill="#8fb3e6" />
    </>
  ),
  "view-week": (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#fff" stroke="#5579b8" />
      <path d="M1.5 5.5h13M4.5 5.5v8M7.5 5.5v8M10.5 5.5v8" stroke="#9fb6d8" />
      <rect x="5" y="7" width="2" height="3" fill="#8fb3e6" />
      <rect x="11" y="9" width="2" height="3" fill="#8fb3e6" />
    </>
  ),
  "view-month": (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#fff" stroke="#5579b8" />
      <path d="M1.5 5.5h13M1.5 8.2h13M1.5 10.9h13M4.7 5.5v8M8 5.5v8M11.3 5.5v8" stroke="#9fb6d8" />
    </>
  ),
  "arrow-left": <path d="M10 3 5 8l5 5" fill="none" stroke="#15428b" strokeWidth="2" />,
  "arrow-right": <path d="M6 3l5 5-5 5" fill="none" stroke="#15428b" strokeWidth="2" />,
  "arrow-up": <path d="M3 10l5-5 5 5" fill="none" stroke="#15428b" strokeWidth="2" />,
  "arrow-down": <path d="M3 6l5 5 5-5" fill="none" stroke="#15428b" strokeWidth="2" />,
  "chevron-down": <path d="M4.5 6.5 8 10l3.5-3.5" fill="none" stroke="#333" strokeWidth="1.3" />,
  "chevron-double": <path d="M4 4l4 4 4-4M4 8.5l4 4 4-4" fill="none" stroke="#15428b" strokeWidth="1.4" />,
  "chevron-left-double": <path d="M8 3 3 8l5 5M13 3 8 8l5 5" fill="none" stroke="#15428b" strokeWidth="1.4" />,
  "chevron-right-double": <path d="M3 3l5 5-5 5M8 3l5 5-5 5" fill="none" stroke="#15428b" strokeWidth="1.4" />,
  "tree-collapsed": <path d="M6 4l4 4-4 4z" fill="#555" />,
  "tree-expanded": <path d="M4 6h8l-4 4z" fill="#555" />,
  close: <path d="M4 4l8 8M12 4l-8 8" stroke="#333" strokeWidth="1.6" />,
  check: <path d="M3 8.5 6.5 12 13 4.5" fill="none" stroke="#1e1e1e" strokeWidth="1.8" />,
  save: (
    <>
      <rect x="1.5" y="1.5" width="13" height="13" rx="1" fill="#5c87c8" stroke="#2a4f8a" />
      <rect x="4" y="1.5" width="8" height="5" fill="#fff" stroke="#2a4f8a" />
      <rect x="4" y="9.5" width="8" height="5" fill="#dbe7f8" stroke="#2a4f8a" />
      <rect x="9" y="2.5" width="2" height="3" fill="#2a4f8a" />
    </>
  ),
  "save-close": (
    <>
      <rect x="1.5" y="1.5" width="11" height="11" rx="1" fill="#5c87c8" stroke="#2a4f8a" />
      <rect x="3.5" y="1.5" width="7" height="4" fill="#fff" stroke="#2a4f8a" />
      <path d="M9 9.5l6 6M15 9.5l-6 6" stroke="#d13434" strokeWidth="1.8" />
    </>
  ),
  send: (
    <>
      {env("#7f9fd0", "#fdfdfd", "#5579b8")}
      <path d="M9 9h6M12.5 6.5 15 9l-2.5 2.5" fill="none" stroke="#2f9a2f" strokeWidth="1.6" />
    </>
  ),
  "check-names": (
    <>
      <path d="M2 10l3 3 5-7" fill="none" stroke="#2f9a2f" strokeWidth="1.8" />
      <circle cx="11.5" cy="4.5" r="2" fill="#e9b984" stroke="#a46d36" strokeWidth=".6" />
      <path d="M8.5 10c.3-2 1.3-3 3-3s2.7 1 3 3z" fill="#3c76c4" />
    </>
  ),
  "attach-file": (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#5579b8" />
      <path d="M9 6v4.5a1.8 1.8 0 0 1-3.6 0V5.3a1.2 1.2 0 0 1 2.4 0V10" fill="none" stroke="#555" strokeWidth=".9" />
    </>
  ),
  "attach-item": (
    <>
      {env()}
      <path d="M12 7v5a1.6 1.6 0 0 1-3.2 0V7.8a1 1 0 0 1 2 0V12" fill="none" stroke="#555" strokeWidth=".9" />
    </>
  ),
  signature: <path d="M1.5 12c2-1 3-5 4.5-5S5 12 7 12s2-4 3.5-4-.5 4 1 4 2-1.5 3-2M1.5 14.5h13" fill="none" stroke="#2c4a8a" strokeWidth="1.1" />,
  "address-check": <path d="M2 8.5 5.5 12 14 3.5" fill="none" stroke="#2f9a2f" strokeWidth="2" />,
  bold: <path d="M4 2.5h4.5a2.7 2.7 0 0 1 0 5.4H4zM4 7.9h5.2a2.8 2.8 0 0 1 0 5.6H4z" fill="none" stroke="#111" strokeWidth="1.8" />,
  italic: <path d="M7 2.5h5M4 13.5h5M10 2.5 6.5 13.5" stroke="#111" strokeWidth="1.4" />,
  underline: <path d="M4.5 2v5.5a3.5 3.5 0 0 0 7 0V2M3.5 14h9" fill="none" stroke="#111" strokeWidth="1.4" />,
  strike: <path d="M11.5 4.5C11 3 9.8 2.5 8 2.5c-2 0-3.5 1-3.5 2.6 0 3.3 7 2 7 5.6 0 1.8-1.6 2.8-3.6 2.8-2 0-3.3-.8-3.8-2.3M2 8h12" fill="none" stroke="#111" strokeWidth="1.2" />,
  bullets: (
    <>
      <circle cx="3" cy="4" r="1.2" fill="#2c62b4" />
      <circle cx="3" cy="8" r="1.2" fill="#2c62b4" />
      <circle cx="3" cy="12" r="1.2" fill="#2c62b4" />
      <path d="M6 4h8M6 8h8M6 12h8" stroke="#333" />
    </>
  ),
  numbering: (
    <>
      <text x="1" y="5.5" fontSize="5" fill="#2c62b4" fontFamily="Segoe UI">
        1
      </text>
      <text x="1" y="9.5" fontSize="5" fill="#2c62b4" fontFamily="Segoe UI">
        2
      </text>
      <text x="1" y="13.5" fontSize="5" fill="#2c62b4" fontFamily="Segoe UI">
        3
      </text>
      <path d="M6 4h8M6 8h8M6 12h8" stroke="#333" />
    </>
  ),
  "align-left": <path d="M2 3h12M2 6h8M2 9h12M2 12h8" stroke="#333" />,
  "align-center": <path d="M2 3h12M4 6h8M2 9h12M4 12h8" stroke="#333" />,
  "align-right": <path d="M2 3h12M6 6h8M2 9h12M6 12h8" stroke="#333" />,
  indent: <path d="M6 3h8M6 6h8M6 9h8M6 12h8M1.5 5.5 4 7.5l-2.5 2" fill="none" stroke="#333" />,
  outdent: <path d="M6 3h8M6 6h8M6 9h8M6 12h8M4 5.5 1.5 7.5l2.5 2" fill="none" stroke="#333" />,
  "font-color": (
    <>
      <path d="M4 11 8 2l4 9M5.5 8h5" fill="none" stroke="#111" strokeWidth="1.3" />
      <rect x="2" y="12.5" width="12" height="2.5" fill="#e0332b" />
    </>
  ),
  highlight: (
    <>
      <path d="M5 10 10.5 3l2.5 2-5.5 7H5z" fill="#fff" stroke="#333" />
      <rect x="2" y="12.5" width="12" height="2.5" fill="#ffe600" />
    </>
  ),
  link: (
    <path
      d="M7 9a2.5 2.5 0 0 0 3.5 0l2.5-2.5a2.5 2.5 0 0 0-3.5-3.5L8.4 4.1M9 7a2.5 2.5 0 0 0-3.5 0L3 9.5A2.5 2.5 0 0 0 6.5 13l1.1-1.1"
      fill="none"
      stroke="#2c62b4"
      strokeWidth="1.3"
    />
  ),
  picture: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#e8f2ff" stroke="#5579b8" />
      <circle cx="5" cy="6" r="1.4" fill="#f4c542" />
      <path d="M2 13l4-4.5 3 3 2-2 3.5 3.5" fill="#6aa84f" stroke="#38761d" strokeWidth=".6" />
    </>
  ),
  table: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#fff" stroke="#5579b8" />
      <path d="M1.5 5.5h13M1.5 9.5h13M6 2.5v11M10.5 2.5v11" stroke="#5579b8" />
    </>
  ),
  paste: (
    <>
      <rect x="2.5" y="2.5" width="9" height="12" rx="1" fill="#c99a52" stroke="#7a5a26" />
      <rect x="5" y="1.5" width="4" height="2.5" fill="#ccc" stroke="#555" />
      <rect x="6.5" y="6.5" width="8" height="8" fill="#fff" stroke="#5579b8" />
    </>
  ),
  cut: <path d="M4 14a2 2 0 1 0 0-.1M12 14a2 2 0 1 0 0-.1M5.2 12.4 11 2M10.8 12.4 5 2" fill="none" stroke="#333" strokeWidth="1.2" />,
  copy: (
    <>
      <rect x="1.5" y="1.5" width="8" height="10" fill="#fff" stroke="#5579b8" />
      <rect x="6.5" y="4.5" width="8" height="10" fill="#fff" stroke="#5579b8" />
    </>
  ),
  undo: <path d="M5 3 2 6l3 3M2.5 6H10a4 4 0 0 1 0 8H6" fill="none" stroke="#2c62b4" strokeWidth="1.5" />,
  redo: <path d="M11 3l3 3-3 3M13.5 6H6a4 4 0 0 0 0 8h4" fill="none" stroke="#2c62b4" strokeWidth="1.5" />,
  categorize: (
    <>
      <rect x="1.5" y="1.5" width="6" height="6" fill="#e7a1a2" stroke="#a65" />
      <rect x="8.5" y="1.5" width="6" height="6" fill="#9fdc9a" stroke="#585" />
      <rect x="1.5" y="8.5" width="6" height="6" fill="#a0c0ec" stroke="#558" />
      <rect x="8.5" y="8.5" width="6" height="6" fill="#f7dd8f" stroke="#a85" />
    </>
  ),
  rules: (
    <>
      {env()}
      <path d="M10 9.5h5M10 12h5M10 14.5h3" stroke="#d13434" strokeWidth="1.3" />
    </>
  ),
  options: (
    <>
      <circle cx="8" cy="8" r="2.3" fill="none" stroke="#555" strokeWidth="1.4" />
      <path d="M8 1v2.5M8 12.5V15M1 8h2.5M12.5 8H15M3 3l1.8 1.8M11.2 11.2 13 13M3 13l1.8-1.8M11.2 4.8 13 3" stroke="#555" strokeWidth="1.6" />
    </>
  ),
  accounts: (
    <>
      <circle cx="6" cy="5" r="2.5" fill="#e9b984" stroke="#a46d36" strokeWidth=".6" />
      <path d="M1.5 13.5c.3-3 2-4.5 4.5-4.5s4.2 1.5 4.5 4.5z" fill="#3c76c4" />
      <rect x="9.5" y="7.5" width="6" height="5" fill="#fff" stroke="#5579b8" />
      <path d="M9.6 7.7 12.5 10l2.9-2.3" fill="none" stroke="#5579b8" />
    </>
  ),
  "mark-read": (
    <>
      <path d="M1.5 7.5 8 3l6.5 4.5v6h-13z" fill="#fdfdfd" stroke="#8d8d8d" />
      <path d="M1.8 7.8 8 11.5l6.2-3.7" fill="none" stroke="#a6a6a6" />
    </>
  ),
  "mark-unread": env("#5579b8", "#fffbe8", "#b08d2b"),
  "reading-pane": (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#fff" stroke="#5579b8" />
      <rect x="8" y="3" width="6" height="10" fill="#cfe0f7" />
      <path d="M3 5h3.5M3 7h3.5M3 9h3.5M3 11h3.5" stroke="#9fb6d8" />
    </>
  ),
  "todo-bar": (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#fff" stroke="#5579b8" />
      <rect x="10" y="3" width="4" height="10" fill="#cfe0f7" />
    </>
  ),
  "nav-pane": (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#fff" stroke="#5579b8" />
      <rect x="2" y="3" width="4" height="10" fill="#cfe0f7" />
    </>
  ),
  help: (
    <>
      <circle cx="8" cy="8" r="6.5" fill="#2c62b4" stroke="#174c8a" />
      <path d="M6 6.3a2 2 0 1 1 2.8 1.8c-.5.3-.8.6-.8 1.4" fill="none" stroke="#fff" strokeWidth="1.5" />
      <circle cx="8" cy="11.8" r=".9" fill="#fff" />
    </>
  ),
  info: (
    <>
      <circle cx="8" cy="8" r="6.5" fill="#2c62b4" stroke="#174c8a" />
      <path d="M8 7v4.5" stroke="#fff" strokeWidth="1.6" />
      <circle cx="8" cy="4.8" r="1" fill="#fff" />
    </>
  ),
  warning: (
    <>
      <path d="M8 1.5 15 14H1z" fill="#f4c542" stroke="#a3801d" />
      <path d="M8 6v4" stroke="#333" strokeWidth="1.6" />
      <circle cx="8" cy="12" r=".9" fill="#333" />
    </>
  ),
  error: (
    <>
      <circle cx="8" cy="8" r="6.5" fill="#d13434" stroke="#8f1d1d" />
      <path d="M5.5 5.5l5 5M10.5 5.5l-5 5" stroke="#fff" strokeWidth="1.6" />
    </>
  ),
  accept: <path d="M2.5 8.5l3.5 3.5 7.5-8" fill="none" stroke="#2f9a2f" strokeWidth="2.2" />,
  tentative: (
    <text x="8" y="12.5" textAnchor="middle" fontSize="12" fontWeight="700" fill="#2c62b4" fontFamily="Segoe UI">
      ?
    </text>
  ),
  decline: <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" stroke="#d13434" strokeWidth="2.2" strokeLinecap="round" />,
  "propose-time": (
    <>
      <circle cx="8" cy="8" r="6" fill="#fff" stroke="#2c62b4" />
      <path d="M8 4.5V8l2.5 1.5" fill="none" stroke="#2c62b4" strokeWidth="1.3" />
    </>
  ),
  "time-zones": (
    <>
      <circle cx="7" cy="7" r="5.5" fill="#dbe8f8" stroke="#2c62b4" />
      <ellipse cx="7" cy="7" rx="2.3" ry="5.5" fill="none" stroke="#2c62b4" strokeWidth=".9" />
      <path d="M1.5 7h11M2.4 4.2h9.2M2.4 9.8h9.2" fill="none" stroke="#2c62b4" strokeWidth=".8" />
      <circle cx="11.5" cy="11.5" r="3.6" fill="#fff" stroke="#c07a12" />
      <path d="M11.5 9.6v2l1.3.8" fill="none" stroke="#c07a12" strokeWidth="1.1" />
    </>
  ),
  scheduling: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#fff" stroke="#5579b8" />
      <path d="M1.5 5.5h13M1.5 8.5h13M1.5 11.5h13" stroke="#9fb6d8" />
      <rect x="6" y="6" width="4" height="2" fill="#3a6fc4" />
      <rect x="4" y="9" width="5" height="2" fill="#3a6fc4" />
    </>
  ),
  "complete-task": <path d="M2.5 8.5l3.5 3.5 7.5-8" fill="none" stroke="#2f9a2f" strokeWidth="2.2" />,
  "business-card": (
    <>
      <rect x="1.5" y="3.5" width="13" height="9" rx="1" fill="#fff" stroke="#5579b8" />
      <rect x="2.5" y="5" width="4" height="5" fill="#dbe7f8" />
      <path d="M8 5.5h5M8 7.5h4M8 9.5h5" stroke="#9fb6d8" />
    </>
  ),
  "phone-list": (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#fff" stroke="#5579b8" />
      <path d="M1.5 5.5h13M1.5 8h13M1.5 10.5h13M6 2.5v11" stroke="#9fb6d8" />
    </>
  ),
  phone: <path d="M4 1.5l2.5 3-1.5 1.5c.8 1.8 2.2 3.2 4 4l1.5-1.5 3 2.5-1.8 2.5C6.5 13.8 2.2 9.5 1.5 3.3z" fill="#4d4d4d" />,
  globe: (
    <>
      <circle cx="8" cy="8" r="6.5" fill="#cfe7fb" stroke="#2c62b4" />
      <path d="M1.5 8h13M8 1.5c-3 3.5-3 9.5 0 13M8 1.5c3 3.5 3 9.5 0 13" fill="none" stroke="#2c62b4" />
    </>
  ),
  "new-folder": (
    <>
      {folder()}
      <path d="M11.5 0.8v5M9 3.3h5" stroke="#2f9a2f" strokeWidth="1.5" />
    </>
  ),
  rename: <path d="M2 12.5h12M3.5 10l6.5-6.5 2 2L5.5 12H3.5z" fill="#fff" stroke="#333" />,
  refresh: <path d="M13 8a5 5 0 1 1-1.5-3.6M13 1.5v3.5H9.5" fill="none" stroke="#2c62b4" strokeWidth="1.6" />,
  "new-window": (
    <>
      <rect x="1.5" y="3.5" width="10" height="10" fill="#fff" stroke="#5579b8" />
      <rect x="1.5" y="3.5" width="10" height="2" fill="#5579b8" />
      <path d="M9 1.5h5.5V7M14.5 1.5 9 7" fill="none" stroke="#2f9a2f" strokeWidth="1.3" />
    </>
  ),
  dot: <circle cx="8" cy="8" r="2.5" fill="#333" />,
  sort: <path d="M5 2v12M2.5 11.5 5 14l2.5-2.5M11 14V2M8.5 4.5 11 2l2.5 2.5" fill="none" stroke="#333" strokeWidth="1.2" />,
  spell: <path d="M2 11 4.5 4 7 11M2.8 9h3.4M9 7.5l2 2.5 3.5-5.5" fill="none" stroke="#333" strokeWidth="1.2" />,
  "download-pictures": (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#e8f2ff" stroke="#5579b8" />
      <path d="M8 4v6M5.5 7.5 8 10l2.5-2.5" fill="none" stroke="#2f9a2f" strokeWidth="1.4" />
    </>
  ),
};

export type IconName = keyof typeof ICONS;

export function Icon({ name, size = 16, className, title }: { name: string; size?: number; className?: string; title?: string }) {
  const g = ICONS[name] ?? ICONS["dot"];
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" className={className} aria-hidden={title ? undefined : true} style={{ flex: "none", display: "block" }}>
      {title && <title>{title}</title>}
      {g}
    </svg>
  );
}
