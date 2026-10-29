// Mail merge entry point (Tools › Mail Merge…, the toolbar button) and the pre-flight summary.
// The compose window does the rest in merge mode (windows/ComposeWindow.tsx, `merge` param).
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openWindow } from "../../api/windows";
import { mergeApi, type MergeSummary, type Preflight, type RowIssue } from "../../api/mailMerge";
import { errorBox } from "../../ui";

/** Ask for a list file and read it; null when cancelled or unreadable (the error was shown). */
export async function pickMergeList(): Promise<{ path: string; summary: MergeSummary } | null> {
  const path = await openDialog({
    title: "Mail Merge — Select Recipient List",
    filters: [
      { name: "Excel or CSV list", extensions: ["xlsx", "xls", "xlsm", "ods", "csv", "txt"] },
      { name: "All Files", extensions: ["*"] },
    ],
  }).catch(() => null);
  if (typeof path !== "string" || !path) return null;
  try {
    return { path, summary: await mergeApi.load(path) };
  } catch (e) {
    errorBox(e, "Mail Merge");
    return null;
  }
}

export async function startMailMerge() {
  const l = await pickMergeList();
  if (!l) return;
  await openWindow("compose", { mode: "new", merge: l.path }, { title: "Mail Merge - Message" });
}

const MAX_LISTED = 10;

function rows(list: RowIssue[], what: (r: RowIssue) => string): string {
  const shown = list.slice(0, MAX_LISTED).map((r) => `  Row ${r.row}: ${what(r)}`);
  if (list.length > MAX_LISTED) shown.push(`  …and ${list.length - MAX_LISTED} more`);
  return shown.join("\n");
}

/** The confirmation text before Send All / Save All as Drafts. */
export function preflightText(p: Preflight, mode: "send" | "drafts"): string {
  const parts: string[] = [];
  const n = p.messages;
  parts.push(
    mode === "send"
      ? `${n} message${n === 1 ? "" : "s"} will be sent, one every few seconds (about ${Math.max(1, Math.round((n * 6) / 60))} min). You can follow them in the Outbox.`
      : `${n} draft${n === 1 ? "" : "s"} will be saved to Drafts.`,
  );
  if (p.overLimit) parts.push("Warning: Google Workspace allows about 2,000 recipients a day. Messages over the limit will fail and stay in the Outbox.");
  if (p.skipped.length) parts.push(`Skipped — no valid e-mail address:\n${rows(p.skipped, (r) => r.email || "(blank)")}`);
  if (p.duplicates.length) parts.push(`Skipped — address already used in an earlier row:\n${rows(p.duplicates, (r) => r.email)}`);
  if (p.blankFields.length) parts.push(`Blank fields (they will be left empty):\n${rows(p.blankFields, (r) => `${r.email} — ${(r.fields ?? []).join(", ")}`)}`);
  return parts.join("\n\n");
}
