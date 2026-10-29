// The Rules Wizard: template → conditions → actions → exceptions → finish (Outlook 2007).
import { useEffect, useMemo, useState } from "react";
import type { Action, ActionType, Condition, ConditionType, Rule } from "../../api/rules";
import type { Account } from "../../api/types";
import { alertBox, confirmBox, Dialog, Icon } from "../../ui";
import {
  ACTION_ORDER,
  CONDITION_ORDER,
  defaultAction,
  defaultCondition,
  describeRule,
  isUnset,
  itemLine,
  suggestName,
  type DescribeCtx,
  type Line,
  type ValueTarget,
} from "./describe";
import { editValue } from "./editors";
import { RuleDescription } from "./RuleDescription";

type Slot<T> = { on: boolean; item: T };
type CondMap = Record<ConditionType, Slot<Condition>>;
type ActMap = Record<ActionType, Slot<Action>>;

function condMap(list: Condition[]): CondMap {
  const m = {} as CondMap;
  for (const t of CONDITION_ORDER) m[t] = { on: false, item: defaultCondition(t) };
  for (const c of list) {
    const cur = m[c.type];
    if (cur.on && "people" in c && "people" in cur.item) cur.item = { ...c, people: [...cur.item.people, ...c.people] } as Condition;
    else if (cur.on && "words" in c && "words" in cur.item) cur.item = { ...c, words: [...cur.item.words, ...c.words] } as Condition;
    else m[c.type] = { on: true, item: c };
  }
  return m;
}

function actMap(list: Action[]): ActMap {
  const m = {} as ActMap;
  for (const t of ACTION_ORDER) m[t] = { on: false, item: defaultAction(t) };
  for (const a of list) m[a.type] = { on: true, item: a };
  return m;
}

/** Prefill values (unchecked) — e.g. from the message "Create Rule" was started on. */
export interface WizardPrefill {
  conditions?: Condition[];
  checked?: ConditionType[];
}

interface Template {
  label: string;
  group: string;
  icon: string;
  conds: ConditionType[];
  acts: ActionType[];
  disabled?: boolean;
}

const TEMPLATES: Template[] = [
  { group: "Stay Organized", label: "Move messages from someone to a folder", icon: "move", conds: ["from"], acts: ["moveToFolder"] },
  { group: "Stay Organized", label: "Move messages with specific words in the subject to a folder", icon: "move", conds: ["subjectContains"], acts: ["moveToFolder"] },
  { group: "Stay Organized", label: "Move messages sent to a distribution list to a folder", icon: "move", conds: ["sentTo"], acts: ["moveToFolder"] },
  { group: "Stay Organized", label: "Delete messages with specific words in the subject", icon: "delete", conds: ["subjectContains"], acts: ["delete"] },
  { group: "Stay Organized", label: "Flag messages from someone for follow-up", icon: "flag", conds: ["from"], acts: ["flag"] },
  { group: "Stay Up to Date", label: "Display mail from someone in the New Item Alert Window", icon: "new-mail", conds: ["from"], acts: ["displayDesktopAlert"] },
  { group: "Stay Up to Date", label: "Play a sound when I get messages from someone", icon: "reminder", conds: ["from"], acts: ["playSound"] },
  { group: "Start from a blank rule", label: "Check messages when they arrive", icon: "mail", conds: [], acts: [] },
  { group: "Start from a blank rule", label: "Check messages after sending", icon: "send", conds: [], acts: [], disabled: true },
];

const STEP_TITLES = [
  "Start from a template or from a blank rule",
  "Which condition(s) do you want to check?",
  "What do you want to do with the message?",
  "Are there any exceptions?",
  "Finish rule setup.",
];

export function RulesWizard({
  initial,
  startStep,
  accounts,
  inboxName,
  describeCtx,
  prefill,
  onFinish,
  onCancel,
}: {
  initial: Rule;
  startStep: 0 | 1;
  accounts: Account[];
  inboxName: string;
  describeCtx: DescribeCtx;
  prefill?: WizardPrefill;
  onFinish: (rule: Rule, opts: { runNow: boolean }) => void;
  onCancel: () => void;
}) {
  const [step, setStep] = useState<number>(startStep);
  const [tpl, setTpl] = useState(0);
  const [conds, setConds] = useState<CondMap>(() => {
    const m = condMap(initial.conditions);
    for (const c of prefill?.conditions ?? []) if (!m[c.type].on) m[c.type] = { on: !!prefill?.checked?.includes(c.type), item: c };
    return m;
  });
  const [excs, setExcs] = useState<CondMap>(() => condMap(initial.exceptions));
  const [acts, setActs] = useState<ActMap>(() => actMap(initial.actions));
  const [name, setName] = useState(initial.name);
  const [enabled, setEnabled] = useState(initial.enabled);
  const [runNow, setRunNow] = useState(false);
  const [allAccounts, setAllAccounts] = useState(initial.accountId == null && accounts.length > 1);

  const rule: Rule = useMemo(
    () => ({
      ...initial,
      name,
      enabled,
      accountId: allAccounts ? null : (initial.accountId ?? accounts[0]?.id ?? null),
      conditions: CONDITION_ORDER.filter((t) => conds[t].on).map((t) => conds[t].item),
      exceptions: CONDITION_ORDER.filter((t) => excs[t].on).map((t) => excs[t].item),
      actions: ACTION_ORDER.filter((t) => acts[t].on).map((t) => acts[t].item),
    }),
    [initial, name, enabled, allAccounts, conds, excs, acts, accounts],
  );
  const lines = useMemo(() => describeRule(rule, describeCtx), [rule, describeCtx]);

  const applyTemplate = (i: number) => {
    const t = TEMPLATES[i];
    if (t.disabled) return;
    setTpl(i);
    setConds((m) => {
      const n = { ...m };
      for (const k of CONDITION_ORDER) n[k] = { ...n[k], on: t.conds.includes(k) };
      return n;
    });
    setActs((m) => {
      const n = { ...m };
      for (const k of ACTION_ORDER) n[k] = { ...n[k], on: t.acts.includes(k) };
      return n;
    });
  };

  // a new rule starts on the first template, like Outlook
  useEffect(() => {
    if (startStep === 0 && !initial.conditions.length && !initial.actions.length) applyTemplate(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // editing an underlined value (in the description or in a list)
  const edit = async (t: ValueTarget) => {
    if (t.list === "actions") {
      const item = rule.actions[t.index];
      const next = (await editValue(t.kind, item, { accountId: rule.accountId })) as Action | null;
      if (next) setActs((m) => ({ ...m, [item.type]: { on: true, item: next } }));
    } else {
      const src = t.list === "conditions" ? rule.conditions : rule.exceptions;
      const item = src[t.index];
      const next = (await editValue(t.kind, item, { accountId: rule.accountId })) as Condition | null;
      const set = t.list === "conditions" ? setConds : setExcs;
      if (next) set((m) => ({ ...m, [item.type]: { on: true, item: next } }));
    }
  };

  const checkValues = async (items: (Condition | Action)[]) => {
    if (items.some(isUnset)) {
      await alertBox("You must specify a value for each underlined value in the rule description.", "Rules Wizard", "warning");
      return false;
    }
    return true;
  };

  const leave = async (from: number): Promise<boolean> => {
    if (from === 1) {
      if (!(await checkValues(rule.conditions))) return false;
      if (!rule.conditions.length && !(await confirmBox("This rule will be applied to every message you receive. Is this correct?", "Rules Wizard", "help", "rules.allMessages")))
        return false;
    }
    if (from === 2) {
      if (!rule.actions.some((a) => a.type !== "stopProcessing")) {
        await alertBox("You must select at least one action for this rule.", "Rules Wizard", "warning");
        return false;
      }
      if (!(await checkValues(rule.actions))) return false;
    }
    if (from === 3 && !(await checkValues(rule.exceptions))) return false;
    return true;
  };

  const next = async () => {
    if (!(await leave(step))) return;
    if (step === 3 && !name.trim()) setName(suggestName(rule));
    setStep((s) => Math.min(4, s + 1));
  };

  const finish = async () => {
    for (let s = Math.max(1, step); s <= 3; s++) if (!(await leave(s))) return void setStep(s);
    const final = { ...rule, name: name.trim() || suggestName(rule) };
    onFinish(final, { runNow });
  };

  const w = Math.min(620, window.innerWidth - 12);
  const h = Math.min(520, window.innerHeight - 12);

  return (
    <Dialog
      title="Rules Wizard"
      onClose={onCancel}
      width={w}
      height={h}
      footer={
        <>
          <button className="cm-button" onClick={onCancel}>
            Cancel
          </button>
          <button className="cm-button" disabled={step <= startStep} onClick={() => setStep((s) => Math.max(0, s - 1))}>
            &lt; Back
          </button>
          <button className={`cm-button${step < 4 ? " default" : ""}`} disabled={step === 4} onClick={next}>
            Next &gt;
          </button>
          <button className={`cm-button${step === 4 ? " default" : ""}`} disabled={step === 0 && !TEMPLATES[tpl].acts.length} onClick={finish}>
            Finish
          </button>
        </>
      }
    >
      <div className="cm-rules-wiz">
        <div className="cm-rules-wiz-title">{STEP_TITLES[step]}</div>

        {step === 0 && (
          <>
            <div className="cm-rules-wiz-step">Step 1: Select a template</div>
            <div className="cm-rules-listbox cm-rules-wiz-list">
              {TEMPLATES.map((t, i) => (
                <div key={i}>
                  {(i === 0 || TEMPLATES[i - 1].group !== t.group) && <div className="cm-rules-wiz-group">{t.group}</div>}
                  <div
                    className={`cm-rules-listbox-item cm-rules-tpl${i === tpl ? " sel" : ""}${t.disabled ? " disabled" : ""}`}
                    onMouseDown={() => applyTemplate(i)}
                    onDoubleClick={() => !t.disabled && next()}
                    title={t.disabled ? "Not available: ClassicMail runs rules on arriving mail" : undefined}
                  >
                    <Icon name={t.icon} />
                    <span>{t.label}</span>
                  </div>
                </div>
              ))}
            </div>
          </>
        )}

        {(step === 1 || step === 3) && (
          <>
            <div className="cm-rules-wiz-step">{step === 1 ? "Step 1: Select condition(s)" : "Step 1: Select exception(s) (if necessary)"}</div>
            <CheckList
              list={step === 1 ? "conditions" : "exceptions"}
              order={CONDITION_ORDER}
              map={step === 1 ? conds : excs}
              ctx={describeCtx}
              onToggle={(t, on) => (step === 1 ? setConds : setExcs)((m) => ({ ...m, [t]: { ...m[t as ConditionType], on } }))}
              onEditValue={async (t) => {
                const set = step === 1 ? setConds : setExcs;
                const cur = (step === 1 ? conds : excs)[t as ConditionType];
                const kind = itemLine("conditions", cur.item, 0).find((s) => "value" in s);
                if (!kind || !("target" in kind)) return;
                const nextItem = (await editValue(kind.target.kind, cur.item, { accountId: rule.accountId })) as Condition | null;
                if (nextItem) set((m) => ({ ...m, [t]: { on: true, item: nextItem } }));
              }}
            />
          </>
        )}

        {step === 2 && (
          <>
            <div className="cm-rules-wiz-step">Step 1: Select action(s)</div>
            <CheckList
              list="actions"
              order={ACTION_ORDER}
              map={acts}
              ctx={describeCtx}
              onToggle={(t, on) =>
                setActs((m) => {
                  const n = { ...m, [t]: { ...m[t as ActionType], on } };
                  // "delete it" and "move it to" exclude each other
                  if (on && t === "delete") n.moveToFolder = { ...n.moveToFolder, on: false };
                  if (on && t === "moveToFolder") n.delete = { ...n.delete, on: false };
                  return n;
                })
              }
              onEditValue={async (t) => {
                const cur = acts[t as ActionType];
                const seg = itemLine("actions", cur.item, 0).find((s) => "value" in s);
                if (!seg || !("target" in seg)) return;
                const nextItem = (await editValue(seg.target.kind, cur.item, { accountId: rule.accountId })) as Action | null;
                if (nextItem) setActs((m) => ({ ...m, [t]: { on: true, item: nextItem } }));
              }}
            />
          </>
        )}

        {step === 4 && (
          <div className="cm-rules-wiz-finish">
            <div className="cm-rules-wiz-step">Step 1: Specify a name for this rule</div>
            <input className="cm-input" style={{ width: "100%", boxSizing: "border-box" }} value={name} autoFocus onChange={(e) => setName(e.target.value)} />
            <div className="cm-rules-wiz-step" style={{ marginTop: 10 }}>
              Step 2: Setup rule options
            </div>
            <label className="cm-rules-check">
              <input type="checkbox" checked={runNow} onChange={(e) => setRunNow(e.target.checked)} /> Run this rule now on messages already in "{inboxName}"
            </label>
            <label className="cm-rules-check">
              <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Turn on this rule
            </label>
            {accounts.length > 1 && (
              <label className="cm-rules-check">
                <input type="checkbox" checked={allAccounts} onChange={(e) => setAllAccounts(e.target.checked)} /> Create this rule on all accounts
              </label>
            )}
          </div>
        )}

        <div className="cm-rules-wiz-step" style={{ marginTop: 8 }}>
          {step === 4 ? "Step 3: Review rule description (click an underlined value to edit)" : "Step 2: Edit the rule description (click an underlined value)"}
        </div>
        <div className="cm-rules-wiz-desc">
          <RuleDescription lines={lines} onEdit={edit} />
        </div>
      </div>
    </Dialog>
  );
}

function CheckList<T extends string>({
  list,
  order,
  map,
  ctx,
  onToggle,
  onEditValue,
}: {
  list: ValueTarget["list"];
  order: T[];
  map: Record<T, Slot<Condition | Action>>;
  ctx: DescribeCtx;
  onToggle: (t: T, on: boolean) => void;
  onEditValue: (t: T) => void;
}) {
  const [sel, setSel] = useState<T | null>(null);
  return (
    <div className="cm-rules-listbox cm-rules-wiz-list" tabIndex={0}>
      {order.map((t) => {
        const slot = map[t];
        const line: Line = itemLine(list, slot.item, 0, ctx);
        return (
          <div key={t} className={`cm-rules-listbox-item cm-rules-checkitem${sel === t ? " sel" : ""}`} onMouseDown={() => setSel(t)}>
            <input type="checkbox" checked={slot.on} onChange={(e) => onToggle(t, e.target.checked)} />
            <span onDoubleClick={() => onToggle(t, !slot.on)}>
              {line.map((s, j) =>
                "text" in s ? (
                  <span key={j}>{s.text}</span>
                ) : (
                  <a
                    key={j}
                    className={`cm-rules-val${s.unset ? " unset" : ""}`}
                    onClick={(e) => {
                      e.preventDefault();
                      onEditValue(t);
                    }}
                  >
                    {s.value}
                  </a>
                ),
              )}
            </span>
          </div>
        );
      })}
    </div>
  );
}
