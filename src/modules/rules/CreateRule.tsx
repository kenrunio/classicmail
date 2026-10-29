// Outlook 2007 "Create Rule" dialog for one message (Actions › Create Rule…), filling the window.
import { useEffect, useState } from "react";
import { closeCurrentWindow } from "../../api/windows";
import { rulesApi, type Action, type Condition, type MailFolder, type MessageInfo, type Rule } from "../../api/rules";
import type { Account } from "../../api/types";
import { alertBox, Dialog, errorBox, Icon } from "../../ui";
import { personLabel, suggestName, type DescribeCtx } from "./describe";
import { editValue } from "./editors";
import { runSummary } from "./RulesWindow";
import { RulesWizard } from "./Wizard";

type SentTo = "meOnly" | "meToOrCc" | `p${number}`;

/** The Create Rule window opens dialog-sized; the Rules Wizard needs more room. */
async function growWindow(w: number, h: number) {
  try {
    const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const { LogicalSize } = await import("@tauri-apps/api/dpi");
    const win = getCurrentWebviewWindow();
    const f = await win.scaleFactor();
    const s = (await win.innerSize()).toLogical(f);
    if (s.width < w || s.height < h) await win.setSize(new LogicalSize(Math.max(s.width, w), Math.max(s.height, h)));
  } catch {
    /* not in Tauri */
  }
}

export function CreateRulePanel({ messageId, accounts, folders, describeCtx }: { messageId: number; accounts: Account[]; folders: MailFolder[]; describeCtx: DescribeCtx }) {
  const [info, setInfo] = useState<MessageInfo | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [useFrom, setUseFrom] = useState(true);
  const [useSubject, setUseSubject] = useState(false);
  const [subject, setSubject] = useState("");
  const [useSentTo, setUseSentTo] = useState(false);
  const [sentTo, setSentTo] = useState<SentTo>("meOnly");
  const [alert, setAlert] = useState(false);
  const [sound, setSound] = useState(false);
  const [useMove, setUseMove] = useState(false);
  const [folder, setFolder] = useState<{ id: number; path: string } | null>(null);
  const [wizard, setWizard] = useState<Rule | null>(null);
  const [done, setDone] = useState<{ rule: Rule; id: number } | null>(null);

  useEffect(() => {
    rulesApi
      .messageInfo(messageId)
      .then((i) => {
        setInfo(i);
        setSubject(i.subject.replace(/^(\s*(re|fw|fwd|aw|wg|השב|הועבר)\s*:\s*)+/i, ""));
      })
      .catch((e) => setErr(String(e?.message ?? e)));
  }, [messageId]);

  const build = (): Rule => {
    const conditions: Condition[] = [];
    if (info && useFrom) conditions.push({ type: "from", people: [info.from] });
    if (useSubject && subject.trim()) conditions.push({ type: "subjectContains", words: [subject.trim()] });
    if (info && useSentTo) {
      if (sentTo === "meOnly") conditions.push({ type: "sentOnlyToMe" });
      else if (sentTo === "meToOrCc") conditions.push({ type: "myNameInToOrCc" });
      else conditions.push({ type: "sentTo", people: [info.to[Number(sentTo.slice(1))]] });
    }
    const actions: Action[] = [];
    if (alert) actions.push({ type: "displayDesktopAlert" });
    if (sound) actions.push({ type: "playSound", sound: "Notify" });
    if (useMove && folder) actions.push({ type: "moveToFolder", folderId: folder.id, folderPath: folder.path });
    const rule: Rule = { id: -1, name: "", enabled: true, position: 0, accountId: info?.accountId ?? null, conditions, exceptions: [], actions };
    return { ...rule, name: suggestName(rule) };
  };

  const pickFolder = async () => {
    const r = (await editValue("folder", { type: "moveToFolder", folderId: folder?.id ?? 0, folderPath: "" }, { accountId: info?.accountId ?? null })) as Action | null;
    if (r && r.type === "moveToFolder") {
      setFolder({ id: r.folderId, path: r.folderPath });
      setUseMove(true);
    }
  };

  const save = async (rule: Rule) => {
    try {
      const { id } = await rulesApi.save({ ...rule, id: 0 });
      setDone({ rule, id });
    } catch (e) {
      await errorBox(e, "Create Rule");
    }
  };

  const ok = async () => {
    const rule = build();
    if (!rule.conditions.length) return alertBox("Select at least one condition for this rule.", "Create Rule", "warning");
    if (useMove && !folder) return pickFolder();
    if (!rule.actions.length) return alertBox("Select at least one action for this rule (or use Advanced Options…).", "Create Rule", "warning");
    await save(rule);
  };

  if (err)
    return (
      <div className="cm-error-bar" style={{ margin: 12 }}>
        {err}
      </div>
    );
  if (!info) return <div className="cm-empty">Loading…</div>;

  const fromLabel = personLabel(info.from);
  const folderLabel = folder ? (folders.find((f) => f.id === folder.id)?.name ?? folder.path.split("/").pop()) : null;

  return (
    <>
      <div className="cm-rules-create">
        <fieldset className="cm-fieldset">
          <legend>When I get e-mail with all of the selected conditions</legend>
          <div className="cm-rules-create-grid">
            <label className="cm-rules-check">
              <input type="checkbox" checked={useFrom} onChange={(e) => setUseFrom(e.target.checked)} /> From {fromLabel}
            </label>
            <span />
            <label className="cm-rules-check">
              <input type="checkbox" checked={useSubject} onChange={(e) => setUseSubject(e.target.checked)} /> Subject contains
            </label>
            <input className="cm-input" value={subject} onChange={(e) => (setSubject(e.target.value), setUseSubject(true))} />
            <label className="cm-rules-check">
              <input type="checkbox" checked={useSentTo} onChange={(e) => setUseSentTo(e.target.checked)} /> Sent to
            </label>
            <select className="cm-input" value={sentTo} onChange={(e) => (setSentTo(e.target.value as SentTo), setUseSentTo(true))}>
              <option value="meOnly">me only</option>
              <option value="meToOrCc">me (To or Cc)</option>
              {info.to.map((p, i) => (
                <option key={i} value={`p${i}`}>
                  {personLabel(p)}
                </option>
              ))}
            </select>
          </div>
        </fieldset>
        <fieldset className="cm-fieldset">
          <legend>Do the following</legend>
          <div className="cm-rules-create-grid">
            <label className="cm-rules-check">
              <input type="checkbox" checked={alert} onChange={(e) => setAlert(e.target.checked)} /> Display in the New Item Alert window
            </label>
            <span />
            <label className="cm-rules-check">
              <input type="checkbox" checked={sound} onChange={(e) => setSound(e.target.checked)} /> Play a selected sound:
            </label>
            <span className="cm-muted">Notify</span>
            <label className="cm-rules-check">
              <input type="checkbox" checked={useMove} onChange={(e) => (e.target.checked && !folder ? pickFolder() : setUseMove(e.target.checked))} /> Move e-mail to folder:
            </label>
            <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
              <button className="cm-button" onClick={pickFolder}>
                Select Folder…
              </button>
              {folderLabel && (
                <span className="cm-ellipsis">
                  <Icon name="folder" size={14} /> {folderLabel}
                </span>
              )}
            </div>
          </div>
        </fieldset>
      </div>
      <div className="cm-rules-footer">
        <button className="cm-button default" onClick={ok}>
          OK
        </button>
        <button className="cm-button" onClick={() => closeCurrentWindow()}>
          Cancel
        </button>
        <button
          className="cm-button"
          onClick={() => {
            growWindow(660, 560);
            setWizard(build());
          }}
        >
          Advanced Options…
        </button>
      </div>

      {wizard && (
        <RulesWizard
          initial={wizard}
          startStep={1}
          accounts={accounts}
          inboxName={info.folderName ?? "Inbox"}
          describeCtx={describeCtx}
          prefill={{
            conditions: [
              { type: "from", people: [info.from] },
              { type: "subjectContains", words: subject.trim() ? [subject.trim()] : [] },
              { type: "sentTo", people: info.to },
            ],
          }}
          onCancel={() => setWizard(null)}
          onFinish={async (rule, opts) => {
            setWizard(null);
            try {
              const { id } = await rulesApi.save({ ...rule, id: 0 });
              if (opts.runNow && info.folderId) await alertBox(runSummary(await rulesApi.runNow({ ruleIds: [id], folderId: info.folderId })), "Rules Wizard");
              closeCurrentWindow();
            } catch (e) {
              await errorBox(e, "Rules Wizard");
            }
          }}
        />
      )}
      {done && <SuccessDialog rule={done.rule} ruleId={done.id} folderId={info.folderId} folderName={info.folderName} />}
    </>
  );
}

function SuccessDialog({ rule, ruleId, folderId, folderName }: { rule: Rule; ruleId: number; folderId: number | null; folderName: string | null }) {
  const [run, setRun] = useState(false);
  const [busy, setBusy] = useState(false);
  const finish = async () => {
    if (run && folderId) {
      setBusy(true);
      try {
        await alertBox(runSummary(await rulesApi.runNow({ ruleIds: [ruleId], folderId })), "Success");
      } catch (e) {
        await errorBox(e, "Success");
      }
    }
    closeCurrentWindow();
  };
  return (
    <Dialog
      title="Success"
      onClose={finish}
      width={380}
      footer={
        <button className="cm-button default" disabled={busy} onClick={finish}>
          OK
        </button>
      }
    >
      <div style={{ display: "flex", gap: 12 }}>
        <Icon name="info" size={32} />
        <div>
          <p style={{ marginTop: 0 }}>The rule "{rule.name}" has been created.</p>
          {folderId && (
            <label className="cm-rules-check">
              <input type="checkbox" checked={run} onChange={(e) => setRun(e.target.checked)} /> Run this rule now on messages already in the current folder
              {folderName ? ` (${folderName})` : ""}
            </label>
          )}
        </div>
      </div>
    </Dialog>
  );
}
