// Compose-only icons (16×16 viewBox, drawn in the Office 2007 spirit). Falls back to the shared set.
import type { JSX } from "react";
import { Icon } from "../../../ui";

const GRID = <path d="M1.5 2.5h13v11h-13zM1.5 6.2h13M1.5 9.8h13M5.8 2.5v11M10.2 2.5v11" fill="none" stroke="#5b7fb5" />;

const LOCAL: Record<string, JSX.Element> = {
  // ---- tables (Format Text › Table) ----
  shading: (
    <>
      <path d="M3 9.5 8 4.5l4 4-5 5z" fill="#fff" stroke="#555" />
      <path d="M8 4.5 6.5 3" stroke="#555" />
      <path d="M12.5 10.5c.8 1.2 1.2 1.8 1.2 2.4a1.2 1.2 0 0 1-2.4 0c0-.6.4-1.2 1.2-2.4z" fill="#4472c4" />
    </>
  ),
  borders: (
    <>
      <path d="M2.5 2.5h11v11h-11z" fill="none" stroke="#c8c8c8" strokeDasharray="1 1" />
      <path d="M8 2.5v11M2.5 8h11" fill="none" stroke="#c8c8c8" strokeDasharray="1 1" />
      <path d="M2.5 13.5h11" stroke="#222" strokeWidth="1.6" />
    </>
  ),
  "border-color": (
    <>
      <path d="M3 12.5 10.5 5l1.5 1.5L4.5 14H3z" fill="#f6d36b" stroke="#8a6d1d" />
      <path d="M10.5 5l1.2-1.2a1 1 0 0 1 1.5 1.5L12 6.5" fill="#d63c34" stroke="#8a1d1d" />
    </>
  ),
  "table-rows": (
    <>
      {GRID}
      <path d="M12 8.5v6M9 11.5h6" stroke="#2a8a2a" strokeWidth="1.8" />
    </>
  ),
  "merge-cells": (
    <>
      <path d="M1.5 2.5h13v11h-13z" fill="#fff" stroke="#5b7fb5" />
      <path d="M8 2.5v3M8 10.5v3" stroke="#5b7fb5" />
      <path d="M3.5 8h9M5.5 6 3.5 8l2 2M10.5 6l2 2-2 2" fill="none" stroke="#1d3f7a" strokeWidth="1.2" />
    </>
  ),
  "split-cells": (
    <>
      <path d="M1.5 2.5h13v11h-13zM8 2.5v11" fill="#fff" stroke="#5b7fb5" />
      <path d="M6.5 8h-4M4.5 6l-2 2 2 2M9.5 8h4M11.5 6l2 2-2 2" fill="none" stroke="#1d3f7a" strokeWidth="1.2" />
    </>
  ),
  "cell-align": (
    <>
      <path d="M1.5 2.5h13v11h-13z" fill="#fff" stroke="#5b7fb5" />
      <path d="M4 6.5h8M4 8h8M4 9.5h8" stroke="#334" />
    </>
  ),
  "table-even": (
    <>
      <path d="M1.5 2.5h13v11h-13zM1.5 6.2h13M1.5 9.8h13M5.8 2.5v11M10.2 2.5v11" fill="#fff" stroke="#5b7fb5" />
      <path d="M6.6 1v14M9.4 1v14" stroke="#1d3f7a" strokeWidth="0.9" strokeDasharray="1.2 0.8" />
      <path d="M7 8h2" stroke="#2a8a2a" strokeWidth="1.4" />
    </>
  ),
  "header-row": (
    <>
      <path d="M1.5 2.5h13v11h-13z" fill="#fff" />
      <rect x="1.5" y="2.5" width="13" height="3.7" fill="#1f497d" />
      {GRID}
    </>
  ),
  "format-painter": (
    <>
      <rect x="2" y="1.5" width="10" height="4" rx="0.8" fill="#f6d36b" stroke="#a37c1d" />
      <path d="M12 3.5h1.5v4H8v2" fill="none" stroke="#6b6b6b" strokeWidth="1.2" />
      <rect x="6.8" y="9.3" width="2.4" height="5.5" rx="0.6" fill="#8a5a2b" />
    </>
  ),
  "clear-format": (
    <>
      <path d="M3 3h8M7 3l-2.5 10" stroke="#1d3f7a" strokeWidth="1.6" fill="none" />
      <path d="M8.5 9.5l5 5M13.5 9.5l-5 5" stroke="#d63c34" strokeWidth="1.6" />
    </>
  ),
  "align-justify": <path d="M2 3h12M2 6h12M2 9h12M2 12h12" stroke="#334" strokeWidth="1.2" />,
  hr: (
    <>
      <path d="M2 4h12M2 12h12" stroke="#9aa" strokeWidth="1" />
      <path d="M1 8h14" stroke="#335" strokeWidth="2" />
    </>
  ),
  symbol: (
    <text x="8" y="13" textAnchor="middle" fontSize="13" fontFamily="Times New Roman, serif" fill="#1d3f7a">
      Ω
    </text>
  ),
  // Format › Markdown: the Markdown mark (M and a down arrow)
  "format-markdown": (
    <>
      <rect x="1" y="3.5" width="14" height="9" rx="1.5" fill="#fff" stroke="#555" />
      <path d="M3.2 10.5v-5l2 2.4 2-2.4v5" stroke="#333" strokeWidth="1.2" fill="none" strokeLinejoin="round" />
      <path d="M11.3 5.5v4.6M9.4 8.4l1.9 2 1.9-2" stroke="#333" strokeWidth="1.2" fill="none" strokeLinejoin="round" />
    </>
  ),
  // Format › View Source: </> on a page
  "format-source": (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" fill="#fff" stroke="#6593cf" />
      <path d="M5.5 5.5 3 8l2.5 2.5M10.5 5.5 13 8l-2.5 2.5M9 4.8 7 11.2" stroke="#c0504d" strokeWidth="1.1" fill="none" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  // Format Text › View Gridlines: a table drawn in dashes
  "table-gridlines": (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="#fff" stroke="#9aa6b8" strokeDasharray="1.5 1.2" />
      <path d="M1.5 6.5h13M1.5 10h13M6 2.5v11M10.5 2.5v11" stroke="#9aa6b8" strokeDasharray="1.5 1.2" />
    </>
  ),
  // Include › Web Page: a browser window with a globe
  "web-page": (
    <>
      <rect x="1.5" y="1.5" width="13" height="13" rx="1" fill="#fff" stroke="#6593cf" />
      <path d="M1.5 4.5h13" stroke="#6593cf" />
      <circle cx="8" cy="9.5" r="3.6" fill="#dbe8f7" stroke="#2f6db5" />
      <path d="M4.4 9.5h7.2M8 5.9c-1.4 1.8-1.4 5.4 0 7.2M8 5.9c1.4 1.8 1.4 5.4 0 7.2" stroke="#2f6db5" strokeWidth="0.8" fill="none" />
    </>
  ),
  "format-html": (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" fill="#fff" stroke="#6593cf" />
      <text x="8" y="10.5" textAnchor="middle" fontSize="6" fontFamily="Tahoma" fontWeight="bold" fill="#c0504d">
        Aa
      </text>
    </>
  ),
  "format-text": (
    <>
      <rect x="2.5" y="1.5" width="11" height="13" fill="#fff" stroke="#8d8d8d" />
      <path d="M4.5 4.5h7M4.5 7h7M4.5 9.5h7M4.5 12h4" stroke="#7a7a7a" />
    </>
  ),
  delay: (
    <>
      <path d="M1.5 5.5 6 2.5l4.5 3v6h-9z" fill="#fdfdfd" stroke="#8d8d8d" />
      <circle cx="11" cy="10.5" r="4" fill="#fff" stroke="#3a6fc4" />
      <path d="M11 8v2.6l1.8 1" stroke="#3a6fc4" fill="none" />
    </>
  ),
  "direct-replies": (
    <>
      <path d="M1.5 6.5 6 3.5l4.5 3v6h-9z" fill="#fdfdfd" stroke="#8d8d8d" />
      <path d="M15 4 11 1v2c-3 0-4.5 1.5-5 4 1.3-1.2 2.6-1.6 5-1.6V7z" fill="#7a3fb4" />
    </>
  ),
  "receipt-read": (
    <>
      <path d="M1.5 6.5 7 3l5.5 3.5v7h-11z" fill="#fdfdfd" stroke="#8d8d8d" />
      <path d="M9 11l2 2 4-5" stroke="#2f9a2f" strokeWidth="1.6" fill="none" />
    </>
  ),
  "receipt-delivery": (
    <>
      <path d="M1.5 4.5h9v7h-9z" fill="#fdfdfd" stroke="#8d8d8d" />
      <path d="M1.8 4.8 6 8l4.2-3.2" fill="none" stroke="#8d8d8d" />
      <path d="M9 11l2 2 4-5" stroke="#2f9a2f" strokeWidth="1.6" fill="none" />
    </>
  ),
  bcc: (
    <>
      <rect x="1" y="3" width="14" height="10" rx="1" fill="#fff" stroke="#6593cf" />
      <text x="8" y="10.5" textAnchor="middle" fontSize="6" fontFamily="Tahoma" fontWeight="bold" fill="#15428b">
        Bcc
      </text>
    </>
  ),
  "from-field": (
    <>
      <rect x="1" y="3" width="14" height="10" rx="1" fill="#fff" stroke="#6593cf" />
      <text x="8" y="10.5" textAnchor="middle" fontSize="5.5" fontFamily="Tahoma" fontWeight="bold" fill="#15428b">
        From
      </text>
    </>
  ),
  "grow-font": (
    <>
      <text x="6" y="13" textAnchor="middle" fontSize="12" fontFamily="Tahoma" fill="#1d3f7a">
        A
      </text>
      <path d="M11 6l2-3 2 3" fill="none" stroke="#2f9a2f" strokeWidth="1.4" />
    </>
  ),
  "shrink-font": (
    <>
      <text x="6" y="13" textAnchor="middle" fontSize="10" fontFamily="Tahoma" fill="#1d3f7a">
        A
      </text>
      <path d="M11 3l2 3 2-3" fill="none" stroke="#d63c34" strokeWidth="1.4" />
    </>
  ),
  "follow-up": (
    <>
      <path d="M3.5 1.5v13" stroke="#555" strokeWidth="1.2" />
      <path d="M4 2h9l-2.2 3.2L13 8.5H4z" fill="#d63c34" stroke="#9a2620" strokeWidth="0.7" />
    </>
  ),
  "attach-line": <path d="M10.5 4.5 5.2 9.8a1.6 1.6 0 0 0 2.3 2.3l5.6-5.6a3 3 0 0 0-4.2-4.2L3.2 8a4.3 4.3 0 0 0 6.1 6.1l4.4-4.4" fill="none" stroke="#555" strokeWidth="1.1" />,
  paperclip: <path d="M10.5 4.5 5.2 9.8a1.6 1.6 0 0 0 2.3 2.3l5.6-5.6a3 3 0 0 0-4.2-4.2L3.2 8a4.3 4.3 0 0 0 6.1 6.1l4.4-4.4" fill="none" stroke="#555" strokeWidth="1.1" />,
  "file-generic": (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#7d7d7d" />
      <path d="M9.5 1.5v3h3" fill="#e6e6e6" stroke="#7d7d7d" />
    </>
  ),
  "file-image": (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#7d7d7d" />
      <path d="M5 12l2-3 1.5 2 1-1.2L11.5 12z" fill="#4b9e45" />
      <circle cx="9.5" cy="7" r="1" fill="#e9b52e" />
    </>
  ),
  "file-pdf": (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#7d7d7d" />
      <rect x="2" y="8" width="9" height="4.5" fill="#c0392b" />
      <text x="6.5" y="11.6" textAnchor="middle" fontSize="3.6" fontFamily="Tahoma" fontWeight="bold" fill="#fff">
        PDF
      </text>
    </>
  ),
  "file-doc": (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#7d7d7d" />
      <rect x="2" y="8" width="7" height="5" fill="#2b5797" />
      <text x="5.5" y="12" textAnchor="middle" fontSize="4" fontFamily="Tahoma" fontWeight="bold" fill="#fff">
        W
      </text>
    </>
  ),
  "file-xls": (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#7d7d7d" />
      <rect x="2" y="8" width="7" height="5" fill="#1e7145" />
      <text x="5.5" y="12" textAnchor="middle" fontSize="4" fontFamily="Tahoma" fontWeight="bold" fill="#fff">
        X
      </text>
    </>
  ),
  "file-ppt": (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#7d7d7d" />
      <rect x="2" y="8" width="7" height="5" fill="#c55a11" />
      <text x="5.5" y="12" textAnchor="middle" fontSize="4" fontFamily="Tahoma" fontWeight="bold" fill="#fff">
        P
      </text>
    </>
  ),
  "file-zip": (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#f7e08a" stroke="#a88b2b" />
      <path d="M7 2v10" stroke="#555" strokeDasharray="1 1" />
    </>
  ),
};

/** Icon by name: local compose icons first, then the shared set. */
export function CIcon({ name, size = 16 }: { name: string; size?: number }) {
  const g = LOCAL[name];
  if (!g) return <Icon name={name} size={size} />;
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden style={{ flex: "none", display: "block" }}>
      {g}
    </svg>
  );
}

export function fileIcon(name: string, mime = ""): string {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (mime.startsWith("image/") || ["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg"].includes(ext)) return "file-image";
  if (ext === "pdf") return "file-pdf";
  if (["doc", "docx", "rtf", "odt"].includes(ext)) return "file-doc";
  if (["xls", "xlsx", "csv", "ods"].includes(ext)) return "file-xls";
  if (["ppt", "pptx", "odp"].includes(ext)) return "file-ppt";
  if (["zip", "7z", "rar", "gz"].includes(ext)) return "file-zip";
  return "file-generic";
}
