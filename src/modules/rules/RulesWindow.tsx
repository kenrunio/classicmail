// Tools › Rules and Alerts (window kind "rules"), Outlook 2007 style.
// Params: none → the Rules and Alerts dialog; { fromMessageId } → the "Create Rule" dialog for
// that message (Actions › Create Rule…), with "Advanced Options…" leading to the Rules Wizard.
import { useCallback, useEffect, useMemo, useState } from "react";
import { open as openFile, save as saveFile } from "@tauri-apps/plugin-dialog";
import { useRpc } from "../../api/query";
import { closeCurrentWindow, openWindow, setWindowTitle, windowParams } from "../../api/windows";
import { rpc } from "../../api/rpc";
import type { Account } from "../../api/types";
import { rulesApi, type Action, type ActionType, type MailFolder, type Rule, type RunStats } from "../../api/rules";
import { alertBox, confirmBox, Dialog, errorBox, Icon, showMenu, useHotkeys, type MenuItem } from "../../ui";
import { actionIcons, defaultAction, describeRule, isUnset, type DescribeCtx, type ValueTarget } from "./describe";
import { EditorHost, PromptDialog, editValue, useFolders } from "./editors";
import { RuleDescription } from "./RuleDescription";
import { RulesWizard, type WizardPrefill } from "./Wizard";
import { CreateRulePanel } from "./CreateRule";
import { applyRules, withSavedIds, type ApplyProgress } from "./apply";
import "./rules.css";

let tempId = -1;
const ALL = 0; // "all mailboxes" in the folder drop-down

export function blankRule(accountId: number | null): Rule {
  return { id: tempId--, name: "", enabled: true, position: 0, accountId, conditions: [], exceptions: [], actions: [] };
}

export function useDescribeCtx(accounts: Account[], folders: MailFolder[]): DescribeCtx {
  return useMemo(
    () => ({
      accountEmail: (id: number) => accounts.find((a) => a.id === id)?.email,
      folderName: (id: number) => folders.find((f) => f.id === id)?.name,
    }),
    [accounts, folders],
  );
}

/** Apply an edited underlined value to a rule. */
export async function editRuleValue(rule: Rule, t: ValueTarget): Promise<Rule | null> {
  const list = rule[t.list] as (Rule["conditions"][number] | Action)[];
  const next = await editValue(t.kind, list[t.index], { accountId: rule.accountId });
  if (!next) return null;
  const copy = [...list];
  copy[t.index] = next;
  return { ...rule, [t.list]: copy };
}

export default function RulesWindow() {
  const params = windowParams();
  const fromMessageId = Number(params.get("fromMessageId")) || 0;
  const accountsQ = useRpc<Account[]>(["accounts"], "accounts.list");
  const accounts = useMemo(() => accountsQ.data ?? [], [accountsQ.data]);
  const { folders } = useFolders();
  const ctx = useDescribeCtx(accounts, folders);

  useEffect(() => {
    setWindowTitle(fromMessageId ? "Create Rule" : "Rules and Alerts");
  }, [fromMessageId]);

  return (
    <div className="cm-rules-win">
      {fromMessageId ? (
        <CreateRulePanel messageId={fromMessageId} accounts={accounts} folders={folders} describeCtx={ctx} />
      ) : (
        <RulesAndAlerts accounts={accounts} folders={folders} describeCtx={ctx} />
      )}
      <EditorHost accounts={accounts} />
    </div>
  );
}

function RulesAndAlerts({ accounts, folders, describeCtx }: { accounts: Account[]; folders: MailFolder[]; describeCtx: DescribeCtx }) {
  const savedQ = useRpc<Rule[]>(["rules", "list"], "rules.list");
  // rules kept as Gmail filters too (rules/gmail_filters.rs)
  const gmail = useRpc<GmailStatus>(["rules", "gmailStatus"], "rules.gmailStatus").data;
  const inGmail = (id: number) => !!gmail?.enabled && !!gmail.rules.find((x) => x.id === id)?.gmail;
  const [work, setWork] = useState<Rule[] | null>(null);
  const [deleted, setDeleted] = useState<number[]>([]);
  const [dirty, setDirty] = useState(false);
  const [tab, setTab] = useState<"rules" | "alerts">("rules");
  // null until the accounts load (then the first mailbox); ALL is a real choice after that
  const [mailboxSel, setMailbox] = useState<number | null>(null);
  const mailbox = mailboxSel ?? accounts[0]?.id ?? ALL;
  const [selId, setSelId] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [wizard, setWizard] = useState<{ rule: Rule; startStep: 0 | 1; prefill?: WizardPrefill } | null>(null);
  const [rename, setRename] = useState<Rule | null>(null);
  const [copyOf, setCopyOf] = useState<Rule | null>(null);
  const [runNow, setRunNow] = useState(false);
  const [options, setOptions] = useState(false);

  // keep the working copy in step with the database until the user changes something
  // (rules.changed refetches savedQ by itself: src/api/events.ts)
  useEffect(() => {
    if (savedQ.data && !dirty) {
      setWork(savedQ.data);
      setDeleted([]);
    }
  }, [savedQ.data, dirty]);

  const rules = useMemo(() => work ?? [], [work]);
  const visible = useMemo(() => rules.filter((r) => mailbox === ALL || r.accountId == null || r.accountId === mailbox), [rules, mailbox]);
  const sel = visible.find((r) => r.id === selId) ?? null;
  useEffect(() => {
    if (!sel && visible.length) setSelId(visible[0].id);
  }, [sel, visible]);

  const change = (next: Rule[]) => {
    setWork(next);
    setDirty(true);
  };
  const replace = (r: Rule) => change(rules.map((x) => (x.id === r.id ? r : x)));

  const inbox = (acct: number | null) => folders.find((f) => f.role === "inbox" && (acct == null || acct === ALL || f.accountId === acct));
  const mailboxName = (acct: number) => {
    const a = accounts.find((x) => x.id === acct);
    return a ? `Inbox [${a.email}]` : "Inbox";
  };

  /** Write a working copy to the database (Apply / OK). Returns temp id → saved id, or null on error. */
  const applyList = useCallback(
    async (list: Rule[], dels: number[]): Promise<Map<number, number> | null> => {
      setBusy(true);
      const progress: ApplyProgress = { ids: new Map(), deletedDone: false };
      const ids = progress.ids;
      try {
        await applyRules(list, dels, savedQ.data ?? [], rulesApi, progress);
        // fresh list first: clearing `dirty` copies savedQ.data into the working list, and the old
        // data would drop a new rule (and move the selection to the first rule)
        await savedQ.refetch();
        setDirty(false);
        setDeleted([]);
        setSelId((cur) => (cur != null ? (ids.get(cur) ?? cur) : cur));
        return ids;
      } catch (e) {
        // keep what did save (still unapplied changes): new rules under their saved ids, deletions
        // done; the fresh saved list makes the next Apply skip the rules already updated
        setWork(withSavedIds(list, ids));
        if (progress.deletedDone) setDeleted([]);
        setSelId((cur) => (cur != null ? (ids.get(cur) ?? cur) : cur));
        await savedQ.refetch();
        await errorBox(e, "Rules and Alerts");
        return null;
      } finally {
        setBusy(false);
      }
    },
    [savedQ],
  );
  const apply = async () => !dirty || (await applyList(rules, deleted)) !== null;

  /** Wizard "Run this rule now on messages already in the Inbox". */
  const runOnInbox = async (ruleId: number, accountId: number | null) => {
    const box = inbox(accountId ?? mailbox);
    if (!box) return alertBox("The rule was saved, but there is no Inbox to run it on yet.", "Rules and Alerts");
    try {
      await alertBox(runSummary(await rulesApi.runNow({ ruleIds: [ruleId], folderId: box.id })), "Rules and Alerts");
    } catch (e) {
      await errorBox(e, "Rules and Alerts");
    }
  };

  const newRule = () => setWizard({ rule: blankRule(mailbox === ALL ? null : mailbox), startStep: 0 });
  const editRule = (r: Rule) => setWizard({ rule: r, startStep: 1 });

  const deleteRule = async (r: Rule) => {
    if (!(await confirmBox(`Are you sure you want to delete the rule "${r.name}"?`, "Rules and Alerts", "warning", "rules.delete"))) return;
    const idx = visible.findIndex((x) => x.id === r.id);
    change(rules.filter((x) => x.id !== r.id));
    if (r.id > 0) setDeleted((d) => [...d, r.id]);
    const nextSel = visible[idx + 1] ?? visible[idx - 1];
    setSelId(nextSel ? nextSel.id : null);
  };

  const move = (r: Rule, dir: -1 | 1) => {
    const vi = visible.findIndex((x) => x.id === r.id);
    const other = visible[vi + dir];
    if (!other) return;
    const a = rules.findIndex((x) => x.id === r.id);
    const b = rules.findIndex((x) => x.id === other.id);
    const next = [...rules];
    [next[a], next[b]] = [next[b], next[a]];
    change(next);
  };

  const toggleAction = async (r: Rule, t: ActionType) => {
    const has = r.actions.find((a) => a.type === t);
    if (has && t !== "moveToFolder" && t !== "copyToFolder" && t !== "forwardTo") {
      return replace({ ...r, actions: r.actions.filter((a) => a.type !== t) });
    }
    const base = has ?? defaultAction(t);
    let item: Action | null = base;
    if (isUnset(base) || t === "moveToFolder" || t === "copyToFolder" || t === "forwardTo") {
      const kind = t === "forwardTo" ? "people" : "folder";
      item = (await editValue(kind, base, { accountId: r.accountId })) as Action | null;
    }
    if (!item) return;
    let actions = has ? r.actions.map((a) => (a.type === t ? item! : a)) : [...r.actions, item];
    if (t === "delete") actions = actions.filter((a) => a.type !== "moveToFolder");
    if (t === "moveToFolder") actions = actions.filter((a) => a.type !== "delete");
    replace({ ...r, actions });
  };

  const changeMenu = (r: Rule): MenuItem[] => {
    const has = (t: ActionType) => r.actions.some((a) => a.type === t);
    return [
      { label: "Edit Rule Settings…", icon: "rules", bold: true, onClick: () => editRule(r) },
      { label: "Rename Rule…", icon: "rename", onClick: () => setRename(r) },
      { type: "separator" },
      { label: "Display in the New Item Alert Window", checked: has("displayDesktopAlert"), onClick: () => toggleAction(r, "displayDesktopAlert") },
      { label: "Play a Sound", checked: has("playSound"), onClick: () => toggleAction(r, "playSound") },
      { label: "Move to Folder…", icon: "move", checked: has("moveToFolder"), onClick: () => toggleAction(r, "moveToFolder") },
      { label: "Copy to Folder…", icon: "copy", checked: has("copyToFolder"), onClick: () => toggleAction(r, "copyToFolder") },
      { label: "Delete the Message", icon: "delete", checked: has("delete"), onClick: () => toggleAction(r, "delete") },
      { label: "Mark as Read", icon: "mark-read", checked: has("markRead"), onClick: () => toggleAction(r, "markRead") },
      { label: "Flag Message for Follow Up", icon: "flag", checked: has("flag"), onClick: () => toggleAction(r, "flag") },
      { label: "Forward to People or Distribution List…", icon: "forward", checked: has("forwardTo"), onClick: () => toggleAction(r, "forwardTo") },
      { label: "Stop Processing More Rules", checked: has("stopProcessing"), onClick: () => toggleAction(r, "stopProcessing") },
    ];
  };

  const onOk = async () => {
    if (await apply()) closeCurrentWindow();
  };
  const onCancel = async () => {
    closeCurrentWindow();
  };

  useHotkeys({
    delete: () => sel && deleteRule(sel),
    enter: () => sel && editRule(sel),
    escape: () => onCancel(),
  });

  const lines = useMemo(() => (sel ? describeRule(sel, describeCtx) : []), [sel, describeCtx]);

  return (
    <>
      <div className="cm-rules-tabs">
        <div className={`cm-rules-tab${tab === "rules" ? " active" : ""}`} onMouseDown={() => setTab("rules")}>
          E-mail Rules
        </div>
        <div className={`cm-rules-tab${tab === "alerts" ? " active" : ""}`} onMouseDown={() => setTab("alerts")}>
          Manage Alerts
        </div>
      </div>
      <div className="cm-rules-page">
        {tab === "alerts" ? (
          <div className="cm-rules-alerts">
            <p>
              Desktop Alerts for particular senders or words are created with the rule action <b>display a Desktop Alert</b>. When a rule with that action matches newly arrived
              mail, ClassicMail shows an alert even if desktop alerts for all new mail are turned off.
            </p>
            <p className="cm-muted">
              Rules with a Desktop Alert:{" "}
              {rules
                .filter((r) => r.actions.some((a) => a.type === "displayDesktopAlert"))
                .map((r) => r.name)
                .join(", ") || "none"}
              .
            </p>
          </div>
        ) : (
          <>
            <div className="cm-rules-row">
              <span>Apply changes to this folder:</span>
              <select className="cm-input cm-fill" value={mailbox} onChange={(e) => setMailbox(Number(e.target.value))}>
                {accounts.map((a) => (
                  <option key={a.id} value={a.id}>
                    {mailboxName(a.id)}
                  </option>
                ))}
                {accounts.length > 1 && <option value={ALL}>All mailboxes</option>}
              </select>
            </div>
            <div className="cm-rules-buttons">
              <button className="cm-rules-btn" onClick={newRule} disabled={busy}>
                <Icon name="rules" />
                New Rule…
              </button>
              <button
                className="cm-rules-btn"
                disabled={!sel || busy}
                onClick={(e) => {
                  const rc = e.currentTarget.getBoundingClientRect();
                  if (sel) showMenu(rc.left, rc.bottom, changeMenu(sel), { minWidth: 260 });
                }}
              >
                Change Rule
                <Icon name="chevron-down" size={10} />
              </button>
              <button className="cm-rules-btn" disabled={!sel || busy} onClick={() => sel && setCopyOf(sel)}>
                <Icon name="copy" />
                Copy…
              </button>
              <button className="cm-rules-btn" disabled={!sel || busy} onClick={() => sel && deleteRule(sel)}>
                <Icon name="delete-red" />
                Delete
              </button>
              <span className="cm-rules-sep" />
              <button className="cm-rules-btn icon" title="Move Up" disabled={!sel || visible[0]?.id === sel?.id} onClick={() => sel && move(sel, -1)}>
                <Icon name="arrow-up" />
              </button>
              <button className="cm-rules-btn icon" title="Move Down" disabled={!sel || visible[visible.length - 1]?.id === sel?.id} onClick={() => sel && move(sel, 1)}>
                <Icon name="arrow-down" />
              </button>
              <span className="cm-rules-sep" />
              <button className="cm-rules-btn" disabled={!rules.length || busy} onClick={async () => (await apply()) && setRunNow(true)}>
                Run Rules Now…
              </button>
              <button className="cm-rules-btn" onClick={async () => (await apply()) && setOptions(true)}>
                Options
              </button>
            </div>
            <div className="cm-rules-table" tabIndex={0}>
              <div className="cm-rules-thead">
                <div className="cm-rules-col-name">Rule (applied in the order shown)</div>
                <div className="cm-rules-col-actions">Actions</div>
              </div>
              <div className="cm-rules-tbody">
                {savedQ.isLoading && (
                  <div className="cm-muted" style={{ padding: 6 }}>
                    Loading…
                  </div>
                )}
                {savedQ.error && <div className="cm-error-bar">{String((savedQ.error as Error).message)}</div>}
                {visible.map((r) => (
                  <div
                    key={r.id}
                    className={`cm-rules-trow${r.id === selId ? " sel" : ""}${r.enabled ? "" : " off"}`}
                    onMouseDown={() => setSelId(r.id)}
                    onDoubleClick={() => editRule(r)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setSelId(r.id);
                      showMenu(e.clientX, e.clientY, changeMenu(r), { minWidth: 260 });
                    }}
                  >
                    <div className="cm-rules-col-name">
                      <input
                        type="checkbox"
                        checked={r.enabled}
                        onMouseDown={(e) => e.stopPropagation()}
                        onChange={(e) => replace({ ...r, enabled: e.target.checked })}
                        aria-label={`Turn on ${r.name}`}
                      />
                      <span className="cm-ellipsis">{r.name}</span>
                      {r.accountId == null && accounts.length > 1 && <span className="cm-muted"> (all mailboxes)</span>}
                      {r.enabled && inGmail(r.id) && (
                        <span className="cm-rules-gmail" title="Also kept in Gmail as a filter, so it works while ClassicMail is closed">
                          Gmail
                        </span>
                      )}
                    </div>
                    <div className="cm-rules-col-actions">
                      {actionIcons(r).map((a, i) => (
                        <span key={i} title={a.title}>
                          <Icon name={a.icon} size={14} />
                        </span>
                      ))}
                    </div>
                  </div>
                ))}
                {!savedQ.isLoading && !visible.length && <div className="cm-muted cm-rules-empty">There are no rules for this folder. Click New Rule… to create one.</div>}
              </div>
            </div>
            <div className="cm-rules-desc-caption">Rule description (click an underlined value to edit):</div>
            <RuleDescription
              lines={lines}
              onEdit={async (t) => {
                if (!sel) return;
                const next = await editRuleValue(sel, t);
                if (next) replace(next);
              }}
            />
            {gmail?.enabled && sel && sel.id > 0 && !inGmail(sel.id) && (
              <div className="cm-muted cm-rules-gmailnote">
                This rule only runs while ClassicMail is open: {gmail.rules.find((x) => x.id === sel.id)?.reason ?? "Gmail can't do what it does"}.
              </div>
            )}
            <GmailOption status={gmail} />
          </>
        )}
      </div>
      <div className="cm-rules-footer">
        {dirty && <span className="cm-muted cm-fill">Changes are saved when you click OK or Apply.</span>}
        <button className="cm-button default" disabled={busy} onClick={onOk}>
          OK
        </button>
        <button className="cm-button" onClick={onCancel}>
          Cancel
        </button>
        <button className="cm-button" disabled={!dirty || busy} onClick={() => apply()}>
          Apply
        </button>
      </div>

      {wizard && (
        <RulesWizard
          initial={wizard.rule}
          startStep={wizard.startStep}
          prefill={wizard.prefill}
          accounts={accounts}
          inboxName={inbox(wizard.rule.accountId ?? mailbox)?.name ?? "Inbox"}
          describeCtx={describeCtx}
          onCancel={() => setWizard(null)}
          onFinish={async (rule, opts) => {
            setWizard(null);
            const exists = rules.some((r) => r.id === rule.id);
            const next = exists ? rules.map((r) => (r.id === rule.id ? rule : r)) : [...rules, rule];
            setWork(next);
            setDirty(true);
            setSelId(rule.id);
            if (opts.runNow) {
              const ids = await applyList(next, deleted);
              const id = ids?.get(rule.id);
              if (id) await runOnInbox(id, rule.accountId);
            }
          }}
        />
      )}
      {rename && (
        <PromptDialog
          title="Rename"
          caption="New name of rule:"
          initial={rename.name}
          onCancel={() => setRename(null)}
          onOk={(name) => {
            replace({ ...rename, name });
            setRename(null);
          }}
        />
      )}
      {copyOf && (
        <CopyRuleDialog
          accounts={accounts}
          initial={mailbox}
          onCancel={() => setCopyOf(null)}
          onOk={(acct) => {
            const c = { ...copyOf, id: tempId--, accountId: acct, name: copyOf.name };
            const i = rules.findIndex((r) => r.id === copyOf.id);
            const next = [...rules];
            next.splice(i + 1, 0, c);
            change(next);
            setMailbox(acct ?? ALL);
            setSelId(c.id);
            setCopyOf(null);
          }}
        />
      )}
      {runNow && (
        <RunRulesNowDialog
          rules={savedQ.data ?? []}
          accounts={accounts}
          folders={folders}
          describeCtx={describeCtx}
          initialFolder={inbox(mailbox)?.id ?? null}
          preselect={sel && sel.id > 0 ? [sel.id] : []}
          onClose={() => setRunNow(false)}
        />
      )}
      {options && <OptionsDialog onClose={() => setOptions(false)} />}
    </>
  );
}

export function runSummary(st: RunStats) {
  const parts = [
    `${st.processed.toLocaleString()} message${st.processed === 1 ? "" : "s"} checked`,
    `${st.matched.toLocaleString()} matched`,
    `${st.actions.toLocaleString()} action${st.actions === 1 ? "" : "s"} carried out`,
  ];
  let s = `Rules completed: ${parts.join(", ")}.`;
  if (st.failed) s += `\n\n${st.failed} action${st.failed === 1 ? "" : "s"} could not be carried out:\n${st.errors.slice(0, 5).join("\n")}`;
  return s;
}

// ---- Copy rule -----------------------------------------------------------------------------

function CopyRuleDialog({ accounts, initial, onOk, onCancel }: { accounts: Account[]; initial: number; onOk: (acct: number | null) => void; onCancel: () => void }) {
  const [v, setV] = useState<number>(initial || accounts[0]?.id || 0);
  return (
    <Dialog
      title="Copy rules to"
      onClose={onCancel}
      width={340}
      footer={
        <>
          <button className="cm-button default" onClick={() => onOk(v === ALL ? null : v)}>
            OK
          </button>
          <button className="cm-button" onClick={onCancel}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-rules-field-caption">Folder:</div>
      <select className="cm-input" style={{ width: "100%" }} value={v} onChange={(e) => setV(Number(e.target.value))} autoFocus>
        {accounts.map((a) => (
          <option key={a.id} value={a.id}>
            Inbox [{a.email}]
          </option>
        ))}
        {accounts.length > 1 && <option value={ALL}>All mailboxes</option>}
      </select>
    </Dialog>
  );
}

// ---- Run Rules Now -------------------------------------------------------------------------

function RunRulesNowDialog({
  rules,
  accounts,
  folders,
  describeCtx,
  initialFolder,
  preselect,
  onClose,
}: {
  rules: Rule[];
  accounts: Account[];
  folders: MailFolder[];
  describeCtx: DescribeCtx;
  initialFolder: number | null;
  preselect: number[];
  onClose: () => void;
}) {
  const [checked, setChecked] = useState<number[]>(preselect);
  const [cur, setCur] = useState<number | null>(preselect[0] ?? rules[0]?.id ?? null);
  const [folderId, setFolderId] = useState<number | null>(initialFolder);
  const [sub, setSub] = useState(false);
  const [applyTo, setApplyTo] = useState<"all" | "unread">("all");
  const [running, setRunning] = useState(false);
  const [status, setStatus] = useState("");
  const folder = folders.find((f) => f.id === folderId);
  const acct = accounts.find((a) => a.id === folder?.accountId);
  const curRule = rules.find((r) => r.id === cur);
  const run = async () => {
    if (!folderId) return alertBox("Choose a folder to run the rules in.", "Run Rules Now");
    setRunning(true);
    setStatus("Running rules…");
    try {
      const ordered = rules.filter((r) => checked.includes(r.id)).map((r) => r.id);
      const st = await rulesApi.runNow({ ruleIds: ordered, folderId, includeSubfolders: sub, onlyUnread: applyTo === "unread" });
      setStatus(runSummary(st));
    } catch (e) {
      setStatus("");
      await errorBox(e, "Run Rules Now");
    } finally {
      setRunning(false);
    }
  };
  return (
    <Dialog
      title="Run Rules Now"
      onClose={onClose}
      width={Math.min(520, window.innerWidth - 12)}
      footer={
        <>
          <button className="cm-button default" disabled={!checked.length || running || !folderId} onClick={run}>
            Run Now
          </button>
          <button className="cm-button" onClick={onClose}>
            Close
          </button>
        </>
      }
    >
      <div className="cm-rules-field-caption">Select rules to run:</div>
      <div className="cm-rules-listbox" style={{ height: 84 }}>
        {rules.map((r) => (
          <div key={r.id} className={`cm-rules-listbox-item cm-rules-checkitem${r.id === cur ? " sel" : ""}`} onMouseDown={() => setCur(r.id)}>
            <input type="checkbox" checked={checked.includes(r.id)} onChange={(e) => setChecked(e.target.checked ? [...checked, r.id] : checked.filter((x) => x !== r.id))} />
            <span className={r.enabled ? "" : "cm-muted"}>{r.name}</span>
          </div>
        ))}
      </div>
      <div style={{ display: "flex", gap: 6, margin: "4px 0 8px" }}>
        <button className="cm-button" onClick={() => setChecked(rules.map((r) => r.id))}>
          Select All
        </button>
        <button className="cm-button" onClick={() => setChecked([])}>
          Unselect All
        </button>
      </div>
      <fieldset className="cm-fieldset">
        <legend>Rule Description</legend>
        <RuleDescription lines={curRule ? describeRule(curRule, describeCtx) : []} />
      </fieldset>
      <div className="cm-form">
        <span className="cm-label">Run in Folder:</span>
        <div style={{ display: "flex", gap: 6 }}>
          <input className="cm-input cm-fill" readOnly value={folder ? `${folder.name}${acct ? ` [${acct.email}]` : ""}` : "(none)"} />
          <button
            className="cm-button"
            onClick={async () => {
              const r = (await editValue("folder", { type: "moveToFolder", folderId: folderId ?? 0, folderPath: "" })) as Action | null;
              if (r && r.type === "moveToFolder") setFolderId(r.folderId);
            }}
          >
            Browse…
          </button>
        </div>
        <span />
        <label className="cm-rules-check">
          <input type="checkbox" checked={sub} onChange={(e) => setSub(e.target.checked)} /> Include subfolders
        </label>
        <span className="cm-label">Apply rules to:</span>
        <select className="cm-input" value={applyTo} onChange={(e) => setApplyTo(e.target.value as "all" | "unread")}>
          <option value="all">All Messages</option>
          <option value="unread">Unread Messages</option>
        </select>
      </div>
      {status && <div className="cm-rules-status">{status}</div>}
    </Dialog>
  );
}

// ---- Options (export / import) -------------------------------------------------------------

function OptionsDialog({ onClose }: { onClose: () => void }) {
  const doExport = async () => {
    try {
      const path = await saveFile({ title: "Export Rules", defaultPath: "ClassicMail rules.json", filters: [{ name: "Rules files", extensions: ["json"] }] });
      if (!path) return;
      const n = await rulesApi.export(path);
      await alertBox(`${n} rule${n === 1 ? "" : "s"} exported to\n${path}`, "Options");
    } catch (e) {
      await errorBox(e, "Export Rules");
    }
  };
  const doImport = async () => {
    try {
      const path = await openFile({ title: "Import Rules", multiple: false, directory: false, filters: [{ name: "Rules files", extensions: ["json"] }] });
      if (!path || Array.isArray(path)) return;
      const res = await rulesApi.import(path);
      let msg = `${res.imported} rule${res.imported === 1 ? "" : "s"} imported.`;
      if (res.disabled.length) msg += `\n\nThese rules were turned off because a mailbox or folder couldn't be found:\n${res.disabled.join("\n")}`;
      if (res.forwards.length) msg += `\n\nThese rules forward your mail and were turned off. Check where they send it before turning them on:\n${res.forwards.join("\n")}`;
      if (res.skipped.length) msg += `\n\nThese rules were not imported because they are not valid:\n${res.skipped.join("\n")}`;
      await alertBox(msg, "Options", res.forwards.length ? "warning" : "info");
    } catch (e) {
      await errorBox(e, "Import Rules");
    }
  };
  return (
    <Dialog
      title="Options"
      onClose={onClose}
      width={340}
      footer={
        <button className="cm-button default" onClick={onClose}>
          OK
        </button>
      }
    >
      <fieldset className="cm-fieldset">
        <legend>Rules Options</legend>
        <div style={{ display: "flex", flexDirection: "column", gap: 8, alignItems: "flex-start" }}>
          <button className="cm-button" onClick={doExport}>
            Export Rules…
          </button>
          <button className="cm-button" onClick={doImport}>
            Import Rules…
          </button>
        </div>
      </fieldset>
    </Dialog>
  );
}

interface GmailStatus {
  enabled: boolean;
  rules: { id: number; gmail: boolean; reason?: string | null }[];
  accounts: { id: number; email: string; state: "ok" | "off" | "signIn" | "error"; message?: string | null; filters: number }[];
}

/** "Also keep these rules in Gmail": takes effect at once (separate from OK / Apply). */
function GmailOption({ status }: { status: GmailStatus | undefined }) {
  const [busy, setBusy] = useState(false);
  if (!status) return null;
  const problems = status.enabled ? status.accounts.filter((a) => a.state === "signIn" || a.state === "error") : [];
  return (
    <div className="cm-rules-gmailopt">
      <label>
        <input
          type="checkbox"
          checked={status.enabled}
          disabled={busy}
          onChange={async (e) => {
            setBusy(true);
            await rpc("rules.setGmailFilters", { on: e.target.checked }).catch(errorBox);
            setBusy(false);
          }}
        />
        Also keep these rules in Gmail, so they work while ClassicMail is closed (and on your phone)
      </label>
      {status.enabled && (
        <div className="cm-muted">Rules Gmail can do are tagged Gmail. Alerts, sounds, importance, forwarding and follow-up dates still only happen while ClassicMail is open.</div>
      )}
      {problems.map((a) => (
        <div key={a.id} className="cm-rules-gmailproblem">
          <Icon name="warning" size={14} />
          <span>
            {a.email}: {a.message}
          </span>
          {a.state === "signIn" && (
            <button className="cm-button" onClick={() => openWindow("accounts", {}, { label: "accounts", title: "Account Settings" })}>
              Account Settings…
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
