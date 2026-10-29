// Outlook 2007 wording for rule conditions / actions / exceptions, and the "rule description"
// lines with their underlined (clickable) values. Pure — no React.
import type { Action, ActionType, Condition, ConditionType, FlagValue, ImportanceLevel, Person, Rule } from "../../api/rules";

export type ValueKind = "people" | "words" | "domains" | "importance" | "account" | "folder" | "flag" | "sound";

/** Where a clickable value lives in the rule. */
export interface ValueTarget {
  list: "conditions" | "exceptions" | "actions";
  index: number;
  kind: ValueKind;
}

export type Seg = { text: string } | { value: string; unset: boolean; target: ValueTarget };
export type Line = Seg[];

export interface DescribeCtx {
  accountEmail?: (id: number) => string | undefined;
  folderName?: (id: number) => string | undefined;
}

type Tpl = { before: string; kind?: ValueKind; placeholder?: string; after?: string };

/** Condition wording, in the order the wizard lists them. */
export const CONDITION_ORDER: ConditionType[] = [
  "from",
  "subjectContains",
  "throughAccount",
  "sentOnlyToMe",
  "myNameInTo",
  "importance",
  "sentTo",
  "myNameInToOrCc",
  "myNameNotInTo",
  "bodyContains",
  "subjectOrBodyContains",
  "headerContains",
  "fromDomain",
  "hasAttachment",
];

const COND: Record<ConditionType, Tpl> = {
  from: { before: "from ", kind: "people", placeholder: "people or distribution list" },
  subjectContains: { before: "with ", kind: "words", placeholder: "specific words", after: " in the subject" },
  throughAccount: { before: "through the ", kind: "account", placeholder: "specified", after: " account" },
  sentOnlyToMe: { before: "sent only to me" },
  myNameInTo: { before: "where my name is in the To box" },
  importance: { before: "marked as ", kind: "importance", placeholder: "importance" },
  sentTo: { before: "sent to ", kind: "people", placeholder: "people or distribution list" },
  myNameInToOrCc: { before: "where my name is in the To or Cc box" },
  myNameNotInTo: { before: "where my name is not in the To box" },
  bodyContains: { before: "with ", kind: "words", placeholder: "specific words", after: " in the body" },
  subjectOrBodyContains: { before: "with ", kind: "words", placeholder: "specific words", after: " in the subject or body" },
  headerContains: { before: "with ", kind: "words", placeholder: "specific words", after: " in the message header" },
  fromDomain: { before: "from senders at ", kind: "domains", placeholder: "specific domains" },
  hasAttachment: { before: "which has an attachment" },
};

const EXC: Record<ConditionType, Tpl> = {
  from: { before: "except if from ", kind: "people", placeholder: "people or distribution list" },
  subjectContains: { before: "except if the subject contains ", kind: "words", placeholder: "specific words" },
  throughAccount: { before: "except through the ", kind: "account", placeholder: "specified", after: " account" },
  sentOnlyToMe: { before: "except if sent only to me" },
  myNameInTo: { before: "except where my name is in the To box" },
  importance: { before: "except if it is marked as ", kind: "importance", placeholder: "importance" },
  sentTo: { before: "except if sent to ", kind: "people", placeholder: "people or distribution list" },
  myNameInToOrCc: { before: "except if my name is in the To or Cc box" },
  myNameNotInTo: { before: "except if my name is not in the To box" },
  bodyContains: { before: "except if the body contains ", kind: "words", placeholder: "specific words" },
  subjectOrBodyContains: { before: "except if the subject or body contains ", kind: "words", placeholder: "specific words" },
  headerContains: { before: "except if the message header contains ", kind: "words", placeholder: "specific words" },
  fromDomain: { before: "except if from senders at ", kind: "domains", placeholder: "specific domains" },
  hasAttachment: { before: "except if it has an attachment" },
};

export const ACTION_ORDER: ActionType[] = [
  "moveToFolder",
  "delete",
  "copyToFolder",
  "forwardTo",
  "setImportance",
  "flag",
  "markRead",
  "playSound",
  "displayDesktopAlert",
  "stopProcessing",
];

const ACT: Record<ActionType, Tpl> = {
  moveToFolder: { before: "move it to the ", kind: "folder", placeholder: "specified", after: " folder" },
  delete: { before: "delete it" },
  copyToFolder: { before: "move a copy to the ", kind: "folder", placeholder: "specified", after: " folder" },
  forwardTo: { before: "forward it to ", kind: "people", placeholder: "people or distribution list" },
  setImportance: { before: "mark it as ", kind: "importance", placeholder: "importance" },
  flag: { before: "flag message for ", kind: "flag", placeholder: "follow up at this time" },
  markRead: { before: "mark it as read" },
  playSound: { before: "play ", kind: "sound", placeholder: "a sound" },
  displayDesktopAlert: { before: "display a Desktop Alert" },
  stopProcessing: { before: "stop processing more rules" },
};

export const FLAG_LABELS: Record<FlagValue, string> = {
  today: "Follow up Today",
  tomorrow: "Follow up Tomorrow",
  thisWeek: "Follow up This Week",
  nextWeek: "Follow up Next Week",
  noDate: "Follow up (no date)",
  complete: "Mark Complete",
  none: "Clear Flag",
};

export const IMPORTANCE_LABELS: Record<ImportanceLevel, string> = { low: "Low", normal: "Normal", high: "High" };

export function personLabel(p: Person) {
  return p.name?.trim() || p.email?.trim() || "";
}

const quoted = (ws: string[]) =>
  ws
    .map((w) => w.trim())
    .filter(Boolean)
    .map((w) => `"${w}"`)
    .join(" or ");

/** Display text of an item's editable value, or null when unset. */
export function valueText(item: Condition | Action, ctx: DescribeCtx = {}): string | null {
  switch (item.type) {
    case "from":
    case "sentTo":
    case "forwardTo": {
      const names = item.people.map(personLabel).filter(Boolean);
      return names.length ? names.join(" or ") : null;
    }
    case "subjectContains":
    case "bodyContains":
    case "subjectOrBodyContains":
    case "headerContains":
      return quoted(item.words) || null;
    case "fromDomain":
      return quoted(item.domains) || null;
    case "importance":
    case "setImportance":
      return IMPORTANCE_LABELS[item.level];
    case "throughAccount":
      return item.accountId > 0 ? (ctx.accountEmail?.(item.accountId) ?? `account ${item.accountId}`) : null;
    case "moveToFolder":
    case "copyToFolder":
      if (!(item.folderId > 0)) return null;
      return ctx.folderName?.(item.folderId) ?? (item.folderPath.split("/").pop() || item.folderPath || `folder ${item.folderId}`);
    case "flag":
      return FLAG_LABELS[item.flag] ?? item.flag;
    case "playSound":
      return item.sound ? item.sound : null;
    default:
      return null;
  }
}

function segs(tpl: Tpl, item: Condition | Action, target: Omit<ValueTarget, "kind">, ctx: DescribeCtx, prefix = ""): Line {
  const out: Line = [{ text: prefix + tpl.before }];
  if (tpl.kind) {
    const v = valueText(item, ctx);
    out.push({ value: v ?? tpl.placeholder ?? "value", unset: v === null && tpl.kind !== "sound", target: { ...target, kind: tpl.kind } });
  }
  if (tpl.after) out.push({ text: tpl.after });
  return out;
}

/** Wording of one catalog entry with its value (used for the wizard's checkbox lists). */
export function itemLine(list: ValueTarget["list"], item: Condition | Action, index: number, ctx: DescribeCtx = {}): Line {
  const tpl = list === "actions" ? ACT[item.type as ActionType] : list === "exceptions" ? EXC[item.type as ConditionType] : COND[item.type as ConditionType];
  return segs(tpl, item, { list, index }, ctx);
}

/** The full Outlook-style rule description. */
export function describeRule(rule: Pick<Rule, "conditions" | "exceptions" | "actions">, ctx: DescribeCtx = {}): Line[] {
  const lines: Line[] = [[{ text: "Apply this rule after the message arrives" }]];
  rule.conditions.forEach((c, i) => lines.push(segs(COND[c.type], c, { list: "conditions", index: i }, ctx, i === 0 ? "" : "and ")));
  rule.actions.forEach((a, i) => lines.push(segs(ACT[a.type], a, { list: "actions", index: i }, ctx, i === 0 ? "" : "and ")));
  rule.exceptions.forEach((c, i) => lines.push(segs(EXC[c.type], c, { list: "exceptions", index: i }, ctx, i === 0 ? "" : "or ")));
  return lines;
}

export function lineText(l: Line) {
  return l.map((s) => ("text" in s ? s.text : s.value)).join("");
}

// ---- defaults / validation ------------------------------------------------------------------

export function defaultCondition(t: ConditionType): Condition {
  switch (t) {
    case "from":
    case "sentTo":
      return { type: t, people: [] };
    case "subjectContains":
    case "bodyContains":
    case "subjectOrBodyContains":
    case "headerContains":
      return { type: t, words: [] };
    case "fromDomain":
      return { type: t, domains: [] };
    case "importance":
      return { type: t, level: "high" };
    case "throughAccount":
      return { type: t, accountId: 0 };
    default:
      return { type: t } as Condition;
  }
}

export function defaultAction(t: ActionType): Action {
  switch (t) {
    case "moveToFolder":
    case "copyToFolder":
      return { type: t, folderId: 0, folderPath: "" };
    case "forwardTo":
      return { type: t, people: [] };
    case "setImportance":
      return { type: t, level: "high" };
    case "flag":
      return { type: t, flag: "noDate" };
    case "playSound":
      return { type: t, sound: "" };
    default:
      return { type: t } as Action;
  }
}

/** True when an item still has an unset underlined value. */
export function isUnset(item: Condition | Action): boolean {
  switch (item.type) {
    case "from":
    case "sentTo":
    case "forwardTo":
      return !item.people.some((p) => p.email.trim() || p.name.trim());
    case "subjectContains":
    case "bodyContains":
    case "subjectOrBodyContains":
    case "headerContains":
      return !item.words.some((w) => w.trim());
    case "fromDomain":
      return !item.domains.some((w) => w.trim());
    case "throughAccount":
      return !(item.accountId > 0);
    case "moveToFolder":
    case "copyToFolder":
      return !(item.folderId > 0);
    default:
      return false;
  }
}

/** Outlook-ish default rule name. */
export function suggestName(rule: Pick<Rule, "conditions" | "actions">): string {
  for (const c of rule.conditions) {
    if ((c.type === "from" || c.type === "sentTo") && c.people.length) return personLabel(c.people[0]);
    if ((c.type === "subjectContains" || c.type === "subjectOrBodyContains" || c.type === "bodyContains") && c.words.length) return c.words[0];
    if (c.type === "fromDomain" && c.domains.length) return c.domains[0];
  }
  return "New rule";
}

/** Small icon names summarising a rule's actions (the list's "Actions" column). */
export function actionIcons(rule: Rule): { icon: string; title: string }[] {
  const out: { icon: string; title: string }[] = [];
  for (const a of rule.actions) {
    const m: Partial<Record<ActionType, [string, string]>> = {
      moveToFolder: ["move", "Move to folder"],
      copyToFolder: ["copy", "Copy to folder"],
      delete: ["delete", "Delete"],
      forwardTo: ["forward", "Forward"],
      flag: ["flag", "Flag"],
      markRead: ["mark-read", "Mark as read"],
      setImportance: ["importance-high", "Set importance"],
      displayDesktopAlert: ["new-mail", "Desktop Alert"],
      playSound: ["reminder", "Play a sound"],
    };
    const v = m[a.type];
    if (v) out.push({ icon: v[0], title: v[1] });
  }
  return out;
}
