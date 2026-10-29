// The Outlook 2007 contact form: Ribbon (Contact tab) + General / Details / All Fields pages.
// Params: ?id=<contact id> to open one, else a new contact (?accountId, ?group, ?fromCompany).
import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Dialog, Icon, Ribbon, RibbonBig, RibbonSmall, RibbonStack, alertBox, confirmBox, errorBox, messageBox, showMenu, useHotkeys, type MenuItem } from "../../../ui";
import { rpc } from "../../../api/rpc";
import { closeCurrentWindow, openWindow, setWindowTitle, windowParams } from "../../../api/windows";
import {
  ADDRESS_TYPES,
  PHONE_TYPES,
  SUFFIXES,
  TITLES,
  addressLabel,
  contactsApi,
  emailDisplayAs,
  emptyContact,
  fileAsOptions,
  formatAddress,
  fullName,
  parseAddress,
  parseFullName,
  phoneLabel,
  phonesByLabel,
  phonesFromLabels,
  type Contact,
  type PostalAddress,
} from "../../../api/contacts";
import { BusinessCard } from "../BusinessCard";
import { callNumber, newMeetingWith, newMessageTo } from "../actions";
import "../contacts.css";

type Page = "general" | "details" | "all";

interface EmailSlot {
  value: string;
  displayName: string;
  type: string;
  /** "Display as" typed by the user (else it follows the name) */
  displayTouched: boolean;
}

interface FormState {
  c: Contact;
  fullNameText: string;
  fileAsTouched: boolean;
  emails: EmailSlot[];
  phones: Record<string, string>;
  addrText: Record<string, string>;
  addrStruct: Record<string, PostalAddress>;
}

const DEFAULT_PHONE_SLOTS = ["Business", "Home", "Business Fax", "Mobile"];

function toForm(c: Contact): FormState {
  const emails: EmailSlot[] = [0, 1, 2].map((i) => {
    const e = c.emails[i];
    const name = c.displayName;
    return {
      value: e?.value ?? "",
      displayName: e?.displayName || (e ? emailDisplayAs(name, e.value) : ""),
      type: e?.type || (i === 0 ? "work" : "other"),
      displayTouched: !!e?.displayName && e.displayName !== emailDisplayAs(name, e.value),
    };
  });
  const addrText: Record<string, string> = {};
  const addrStruct: Record<string, PostalAddress> = {};
  for (const a of c.addresses) {
    addrText[a.type || "other"] = formatAddress(a);
    addrStruct[a.type || "other"] = a;
  }
  return {
    c,
    fullNameText: fullName(c),
    fileAsTouched: !!c.fileAs && c.fileAs !== fileAsOptions(c)[0],
    emails,
    phones: phonesByLabel(c.phones),
    addrText,
    addrStruct,
  };
}

/** The contact the form currently describes (what Save sends). */
function fromForm(f: FormState): Contact {
  const c = { ...f.c };
  const name = fullName(c) || c.company;
  c.displayName = name || f.emails.find((e) => e.value)?.value || "";
  c.emails = f.emails
    .filter((e) => e.value.trim())
    .map((e) => {
      const auto = emailDisplayAs(name, e.value.trim());
      return { value: e.value.trim(), type: e.type, displayName: e.displayTouched && e.displayName !== auto ? e.displayName : "" };
    });
  c.phones = phonesFromLabels(f.phones);
  c.addresses = ADDRESS_TYPES.map((t) => t.type)
    .concat(Object.keys(f.addrText).filter((k) => !ADDRESS_TYPES.some((t) => t.type === k)))
    .filter((t) => (f.addrText[t] ?? "").trim())
    .map((t) => parseAddress(f.addrText[t], t, f.addrStruct[t]));
  if (!f.fileAsTouched || !c.fileAs.trim()) c.fileAs = fileAsOptions(c)[0] ?? "";
  if (c.mailingAddress && !c.addresses.some((a) => a.type === c.mailingAddress)) c.mailingAddress = "";
  return c;
}

const BIRTHDAY_NO_YEAR = "1604";

function dateToInput(d: string | null): string {
  if (!d) return "";
  return d.startsWith("--") ? `${BIRTHDAY_NO_YEAR}${d.slice(1)}` : d;
}

function inputToDate(v: string): string | null {
  if (!v) return null;
  return v.startsWith(`${BIRTHDAY_NO_YEAR}-`) ? `-${v.slice(4)}` : v;
}

export default function ContactWindow() {
  const params = useMemo(() => windowParams(), []);
  const id = Number(params.get("id")) || null;
  const accountId = Number(params.get("accountId")) || null;
  const [contactId, setContactId] = useState<number | null>(id);
  const [form, setForm] = useState<FormState>(() => {
    const c = emptyContact();
    const g = params.get("group");
    if (g) c.groups = [g];
    return toForm(c);
  });
  const [loaded, setLoaded] = useState(!id);
  const [dirty, setDirty] = useState(false);
  const [page, setPage] = useState<Page>("general");
  const [emailSlot, setEmailSlot] = useState(0);
  const [phoneSlots, setPhoneSlots] = useState(DEFAULT_PHONE_SLOTS);
  const [addrType, setAddrType] = useState("work");
  const [pendingPhoto, setPendingPhoto] = useState<string | null>(null);
  const [dialog, setDialog] = useState<null | "name" | "address" | "card">(null);
  const [saving, setSaving] = useState(false);
  const [serverDirty, setServerDirty] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const dirtyRef = useRef(false);
  dirtyRef.current = dirty;

  const current = useMemo(() => fromForm(form), [form]);
  const preview = useMemo(() => ({ ...current, photoUrl: pendingPhoto ?? current.photoUrl }), [current, pendingPhoto]);

  // load an existing contact, or prefill "New Contact from Same Company"
  useEffect(() => {
    if (id) {
      contactsApi
        .get(id)
        .then((c) => {
          setForm(toForm(c));
          setServerDirty(c.dirty);
          if (c.mailingAddress) setAddrType(c.mailingAddress);
          else if (!c.addresses.some((a) => a.type === "work") && c.addresses[0]) setAddrType(c.addresses[0].type);
          setLoaded(true);
        })
        .catch((e) => {
          errorBox(e).then(() => closeCurrentWindow());
        });
    } else {
      const from = Number(params.get("fromCompany")) || null;
      if (from)
        contactsApi
          .get(from)
          .then((src) => {
            const c = emptyContact();
            c.company = src.company;
            c.addresses = src.addresses.filter((a) => a.type === "work");
            c.phones = src.phones.filter((p) => p.type === "work" || p.type === "workFax" || p.type === "main");
            c.urls = src.urls.slice(0, 1);
            c.groups = src.groups;
            setForm(toForm(c));
          })
          .catch((e) => errorBox(e));
    }
  }, [id, params]);

  const title = `${current.displayName || "Untitled"} - Contact`;
  useEffect(() => {
    if (loaded) setWindowTitle(title);
  }, [title, loaded]);

  // ask before closing with unsaved changes (Outlook: "Do you want to save changes?").
  // Registered once, so it calls the latest save() through a ref (not the first render's).
  const saveRef = useRef<() => Promise<boolean>>(async () => false);
  useEffect(() => {
    const un = getCurrentWebviewWindow().onCloseRequested(async (ev) => {
      if (!dirtyRef.current) return;
      ev.preventDefault();
      const r = await messageBox({ title: "ClassicMail", text: "Do you want to save changes?", icon: "warning", buttons: ["Yes", "No", "Cancel"], defaultButton: "Yes" });
      if (r === "Yes") {
        if (await saveRef.current()) getCurrentWebviewWindow().destroy();
      } else if (r === "No") getCurrentWebviewWindow().destroy();
    });
    // unlisten through the promise: a cleanup that runs before it resolves still removes the handler
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  const update = (fn: (f: FormState) => FormState) => {
    setForm((f) => fn(f));
    setDirty(true);
  };
  const setC = (patch: Partial<Contact>) => update((f) => ({ ...f, c: { ...f.c, ...patch } }));

  const setNameParts = (parts: ReturnType<typeof parseFullName>) =>
    update((f) => {
      const c = { ...f.c, ...parts };
      return { ...f, c, fullNameText: fullName(c) };
    });

  async function save(): Promise<boolean> {
    if (saving) return false;
    const c = fromForm(form);
    if (!c.displayName && !c.phones.length) {
      await alertBox("Type a name, company or e-mail address for this contact before saving it.", "ClassicMail", "warning");
      return false;
    }
    setSaving(true);
    try {
      const what = `"${c.displayName || "Untitled"}"`;
      let saved = contactId ? await contactsApi.update(contactId, c, `Edit ${what}`) : await contactsApi.create(c, accountId, `New Contact ${what}`);
      if (pendingPhoto) saved = await contactsApi.setPhoto(saved.id, pendingPhoto);
      setContactId(saved.id);
      setPendingPhoto(null);
      setForm(toForm(saved));
      setServerDirty(saved.dirty);
      setDirty(false);
      dirtyRef.current = false;
      setWindowTitle(`${saved.displayName || "Untitled"} - Contact`);
      return true;
    } catch (e) {
      errorBox(e, "ClassicMail");
      return false;
    } finally {
      setSaving(false);
    }
  }
  saveRef.current = save;

  const saveAndClose = async () => {
    if (!dirty && contactId) return closeCurrentWindow();
    if (await save()) getCurrentWebviewWindow().destroy();
  };
  const saveAndNew = async () => {
    if (dirty || !contactId) {
      if (!(await save())) return;
    }
    await openWindow("contact", { accountId: accountId ?? current.accountId ?? undefined }, { title: "Untitled - Contact" });
    getCurrentWebviewWindow().destroy();
  };
  const del = async () => {
    if (!contactId) {
      getCurrentWebviewWindow().destroy();
      return;
    }
    if (!(await confirmBox(`Are you sure you want to delete "${current.displayName || "Untitled"}"?`, "ClassicMail", "warning", "contacts.delete"))) return;
    try {
      await contactsApi.remove([contactId], `Delete "${current.displayName || "Untitled"}"`);
      getCurrentWebviewWindow().destroy();
    } catch (e) {
      errorBox(e);
    }
  };

  useHotkeys(
    {
      "ctrl+s": () => save(),
      "alt+s": () => saveAndClose(),
      "ctrl+d": () => del(),
      escape: () => closeCurrentWindow(),
      "ctrl+shift+s": () => saveAndNew(),
      "ctrl+p": () => window.print(),
    },
    { allowInInputs: true },
  );

  const pickPicture = () => fileRef.current?.click();
  const onPicture = (file: File | undefined) => {
    if (!file) return;
    if (!/^image\/(png|jpeg|gif)$/.test(file.type)) return alertBox("Choose a PNG, JPEG or GIF picture.", "Add Contact Picture", "warning");
    if (file.size > 3_000_000) return alertBox("The picture is too large. Choose a picture smaller than 3 MB.", "Add Contact Picture", "warning");
    const r = new FileReader();
    r.onload = () => {
      setPendingPhoto(String(r.result));
      setDirty(true);
    };
    r.readAsDataURL(file);
  };

  const web = current.urls[0]?.value ?? "";
  const openWeb = () => {
    if (!web) return alertBox("There is no Web page address for this contact.", "ClassicMail");
    rpc("app.openUrl", { url: /^https?:\/\//i.test(web) ? web : `http://${web}` }).catch(errorBox);
  };
  const mapIt = () => {
    const a = current.addresses.find((x) => x.type === addrType) ?? current.addresses[0];
    if (!a) return alertBox("There is no address to map.", "ClassicMail");
    rpc("app.openUrl", { url: `https://www.google.com/maps/search/?api=1&query=${encodeURIComponent(formatAddress(a).replace(/\n/g, ", "))}` }).catch(errorBox);
  };
  const callItems = (): MenuItem[] =>
    current.phones.length
      ? current.phones.map((p) => ({ label: `${p.value}  ${phoneLabel(p.type)}`, icon: "phone", onClick: () => callNumber(current, p.value, phoneLabel(p.type)) }))
      : [{ label: "(no phone numbers)", disabled: true }];

  const ribbon = [
    {
      id: "contact",
      label: "Contact",
      groups: [
        {
          label: "Actions",
          content: (
            <>
              <RibbonBig icon="save-close" label="Save & Close" onClick={saveAndClose} disabled={saving} />
              <RibbonStack>
                <RibbonSmall icon="save" label="Save & New" onClick={saveAndNew} disabled={saving} />
                <RibbonSmall icon="forward" label="Send" disabled title="Forward as Business Card is not available" />
                <RibbonSmall icon="delete-red" label="Delete" onClick={del} />
              </RibbonStack>
            </>
          ),
        },
        {
          label: "Show",
          content: (
            <>
              <RibbonBig icon="contact" label="General" pressed={page === "general"} onClick={() => setPage("general")} />
              <RibbonBig icon="notes" label="Details" pressed={page === "details"} onClick={() => setPage("details")} />
              <RibbonStack>
                <RibbonSmall icon="table" label="All Fields" pressed={page === "all"} onClick={() => setPage("all")} />
              </RibbonStack>
            </>
          ),
        },
        {
          label: "Communicate",
          content: (
            <>
              <RibbonBig icon="new-mail" label="E-mail" onClick={() => newMessageTo([current])} />
              <RibbonBig icon="meeting" label="Meeting" onClick={() => newMeetingWith([current])} />
              <RibbonBig icon="phone" label="Call" menu={callItems} />
              <RibbonStack>
                <RibbonSmall icon="globe" label="Web Page" onClick={openWeb} />
                <RibbonSmall icon="link" label="Map It" onClick={mapIt} />
              </RibbonStack>
            </>
          ),
        },
        {
          label: "Options",
          content: (
            <>
              <RibbonBig icon="business-card" label="Business Card" onClick={() => setDialog("card")} />
              <RibbonBig
                icon="picture"
                label="Picture"
                menu={[
                  { label: "Add Picture...", icon: "picture", onClick: pickPicture },
                  { label: "Change Picture...", disabled: !preview.photoUrl, onClick: pickPicture },
                  { label: "Remove Picture", disabled: true },
                ]}
              />
              <RibbonStack>
                <RibbonSmall icon="categorize" label="Categorize" disabled />
                <RibbonSmall icon="flag" label="Follow Up" disabled />
                <RibbonSmall icon="private" label="Private" disabled />
              </RibbonStack>
            </>
          ),
        },
      ],
    },
  ];

  const orbMenu: MenuItem[] = [
    { label: "Save", icon: "save", shortcut: "Ctrl+S", onClick: () => save() },
    { label: "Save & New", icon: "save", onClick: saveAndNew },
    { type: "separator" },
    { label: "Delete", icon: "delete-red", shortcut: "Ctrl+D", onClick: del },
    { label: "Print", icon: "print", onClick: () => window.print() },
    { type: "separator" },
    { label: "Close", icon: "close", onClick: () => closeCurrentWindow() },
  ];

  if (!loaded) return <div className="cm-empty">Loading…</div>;

  return (
    <div className="cm-itemwin">
      <Ribbon
        tabs={ribbon}
        orbMenu={orbMenu}
        quickAccess={
          <button className="cm-tb-btn" title="Save (Ctrl+S)" onClick={() => save()}>
            <Icon name="save" />
          </button>
        }
      />
      {(serverDirty || dirty) && (
        <div className="cm-contacts-pending">
          {dirty ? "This contact has unsaved changes." : "Changes to this contact are waiting to be sent to Google (they go with the next Send/Receive)."}
        </div>
      )}
      <input ref={fileRef} type="file" accept="image/png,image/jpeg,image/gif" hidden onChange={(e) => onPicture(e.target.files?.[0])} />
      <div className="cm-contacts-form">
        {page === "general" && (
          <General
            form={form}
            update={update}
            setC={setC}
            setNameParts={setNameParts}
            preview={preview}
            emailSlot={emailSlot}
            setEmailSlot={setEmailSlot}
            phoneSlots={phoneSlots}
            setPhoneSlots={setPhoneSlots}
            addrType={addrType}
            setAddrType={setAddrType}
            openDialog={setDialog}
            pickPicture={pickPicture}
          />
        )}
        {page === "details" && <Details c={form.c} setC={setC} />}
        {page === "all" && <AllFields c={current} />}
      </div>
      {dialog === "name" && <CheckFullName c={form.c} onClose={() => setDialog(null)} onOk={(p) => setNameParts(p)} />}
      {dialog === "address" && (
        <CheckAddress
          type={addrType}
          value={parseAddress(form.addrText[addrType] ?? "", addrType, form.addrStruct[addrType])}
          onClose={() => setDialog(null)}
          onOk={(a) => update((f) => ({ ...f, addrText: { ...f.addrText, [addrType]: formatAddress(a) }, addrStruct: { ...f.addrStruct, [addrType]: a } }))}
        />
      )}
      {dialog === "card" && (
        <Dialog
          title="Business Card"
          width={300}
          onClose={() => setDialog(null)}
          footer={
            <button className="cm-button default" onClick={() => setDialog(null)}>
              Close
            </button>
          }
        >
          <div className="cm-contacts-preview">
            <BusinessCard contact={preview} />
          </div>
        </Dialog>
      )}
    </div>
  );
}

// ---- General page ---------------------------------------------------------------------------

/** Outlook's "label button with a drop-down" (E-mail ▾, Business ▾...). */
function LabelSelect({ label, onMain, menu }: { label: string; onMain?: () => void; menu: () => MenuItem[] }) {
  const open = (e: React.MouseEvent) => {
    const r = (e.currentTarget.closest(".cm-contacts-labelsel") as HTMLElement).getBoundingClientRect();
    showMenu(r.left, r.bottom, menu(), { minWidth: r.width });
  };
  return (
    <div className="cm-contacts-labelsel">
      <button onMouseDown={(e) => e.preventDefault()} onClick={(e) => (onMain ? onMain() : open(e))}>
        {label}
      </button>
      <span className="arrow" onClick={open}>
        <Icon name="chevron-down" size={10} />
      </span>
    </div>
  );
}

function General(p: {
  form: FormState;
  update: (fn: (f: FormState) => FormState) => void;
  setC: (patch: Partial<Contact>) => void;
  setNameParts: (parts: ReturnType<typeof parseFullName>) => void;
  preview: Contact;
  emailSlot: number;
  setEmailSlot: (n: number) => void;
  phoneSlots: string[];
  setPhoneSlots: (s: string[]) => void;
  addrType: string;
  setAddrType: (t: string) => void;
  openDialog: (d: "name" | "address") => void;
  pickPicture: () => void;
}) {
  const { form, update, setC } = p;
  const c = form.c;
  const options = fileAsOptions({ ...c, ...parseFullName(form.fullNameText) });
  const fileAsValue = form.fileAsTouched ? c.fileAs : (options[0] ?? "");
  const email = form.emails[p.emailSlot];
  const nameForDisplay = fullName(c) || c.company;
  const setEmail = (patch: Partial<EmailSlot>) =>
    update((f) => {
      const emails = f.emails.map((e, i) => {
        if (i !== p.emailSlot) return e;
        const n = { ...e, ...patch };
        if (!n.displayTouched) n.displayName = emailDisplayAs(nameForDisplay, n.value.trim());
        return n;
      });
      return { ...f, emails };
    });
  const addrText = form.addrText[p.addrType] ?? "";

  return (
    <div className="cm-contacts-form-cols">
      <div className="cm-contacts-form-left">
        <div className="cm-contacts-row">
          <button className="cm-field-label-btn" onClick={() => p.openDialog("name")}>
            Full Name...
          </button>
          <input
            className="cm-input"
            dir="auto"
            autoFocus
            value={form.fullNameText}
            onChange={(e) => {
              const text = e.target.value;
              update((f) => ({ ...f, fullNameText: text, c: { ...f.c, ...parseFullName(text) } }));
            }}
          />
        </div>
        <div className="cm-contacts-row">
          <label>Company:</label>
          <input className="cm-input" dir="auto" value={c.company} onChange={(e) => setC({ company: e.target.value })} />
        </div>
        <div className="cm-contacts-row">
          <label>Job title:</label>
          <input className="cm-input" dir="auto" value={c.jobTitle} onChange={(e) => setC({ jobTitle: e.target.value })} />
        </div>
        <div className="cm-contacts-row">
          <label>File as:</label>
          <input
            className="cm-input"
            dir="auto"
            list="cm-contacts-fileas"
            value={fileAsValue}
            onChange={(e) => {
              const v = e.target.value;
              update((f) => ({ ...f, fileAsTouched: true, c: { ...f.c, fileAs: v } }));
            }}
          />
          <datalist id="cm-contacts-fileas">
            {options.map((o) => (
              <option key={o} value={o} />
            ))}
          </datalist>
        </div>

        <div className="cm-contacts-section">Internet</div>
        <div className="cm-contacts-row">
          <LabelSelect
            label={p.emailSlot === 0 ? "E-mail..." : `E-mail ${p.emailSlot + 1}...`}
            menu={() => ["E-mail", "E-mail 2", "E-mail 3"].map((l, i) => ({ label: l, checked: p.emailSlot === i, radio: true, onClick: () => p.setEmailSlot(i) }))}
          />
          <input className="cm-input" value={email.value} onChange={(e) => setEmail({ value: e.target.value })} />
        </div>
        <div className="cm-contacts-row">
          <label>Display as:</label>
          <input
            className="cm-input"
            dir="auto"
            value={email.displayTouched ? email.displayName : emailDisplayAs(nameForDisplay, email.value.trim())}
            onChange={(e) => setEmail({ displayName: e.target.value, displayTouched: true })}
          />
        </div>
        <div className="cm-contacts-row">
          <label>Web page address:</label>
          <input
            className="cm-input"
            value={c.urls[0]?.value ?? ""}
            onChange={(e) => setC({ urls: e.target.value ? [{ type: c.urls[0]?.type || "work", value: e.target.value }, ...c.urls.slice(1)] : c.urls.slice(1) })}
          />
        </div>
        <div className="cm-contacts-row">
          <label>IM address:</label>
          <input
            className="cm-input"
            value={c.ims[0]?.username ?? ""}
            onChange={(e) => setC({ ims: e.target.value ? [{ protocol: c.ims[0]?.protocol || "", username: e.target.value }, ...c.ims.slice(1)] : c.ims.slice(1) })}
          />
        </div>

        <div className="cm-contacts-section">Phone numbers</div>
        {p.phoneSlots.map((label, slot) => (
          <div className="cm-contacts-row" key={slot}>
            <LabelSelect
              label={`${label}...`}
              menu={() =>
                PHONE_TYPES.map((t) => ({
                  label: t.label,
                  checked: t.label === label,
                  bold: !!form.phones[t.label],
                  onClick: () => p.setPhoneSlots(p.phoneSlots.map((l, i) => (i === slot ? t.label : l))),
                }))
              }
            />
            <input
              className="cm-input"
              value={form.phones[label] ?? ""}
              onChange={(e) => {
                const v = e.target.value;
                update((f) => ({ ...f, phones: { ...f.phones, [label]: v } }));
              }}
            />
          </div>
        ))}

        <div className="cm-contacts-section">Addresses</div>
        <div className="cm-contacts-addr">
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            <LabelSelect
              label={`${addressLabel(p.addrType)}...`}
              onMain={() => p.openDialog("address")}
              menu={() =>
                ADDRESS_TYPES.map((t) => ({ label: t.label, checked: t.type === p.addrType, bold: !!(form.addrText[t.type] ?? "").trim(), onClick: () => p.setAddrType(t.type) }))
              }
            />
            <label className="cm-contacts-mailing">
              <input
                type="checkbox"
                checked={c.mailingAddress === p.addrType}
                disabled={!addrText.trim()}
                onChange={(e) => setC({ mailingAddress: e.target.checked ? p.addrType : "" })}
              />
              This is the mailing address
            </label>
          </div>
          <textarea
            className="cm-input"
            dir="auto"
            value={addrText}
            onChange={(e) => {
              const v = e.target.value;
              update((f) => ({ ...f, addrText: { ...f.addrText, [p.addrType]: v } }));
            }}
          />
        </div>
      </div>

      <div className="cm-contacts-form-right">
        <div className="cm-contacts-preview" onDoubleClick={p.pickPicture} title="Double-click to add a picture">
          <BusinessCard contact={p.preview} />
        </div>
        <div className="cm-contacts-notes">
          <div className="cm-contacts-section" style={{ marginTop: 0 }}>
            Notes
          </div>
          <textarea className="cm-input" dir="auto" value={c.notes} onChange={(e) => setC({ notes: e.target.value })} />
        </div>
      </div>
    </div>
  );
}

// ---- Details page ---------------------------------------------------------------------------

function Details({ c, setC }: { c: Contact; setC: (patch: Partial<Contact>) => void }) {
  const text = (label: string, key: keyof Contact) => (
    <div className="cm-contacts-row">
      <label>{label}</label>
      <input className="cm-input" dir="auto" value={String(c[key] ?? "")} onChange={(e) => setC({ [key]: e.target.value } as Partial<Contact>)} />
    </div>
  );
  const combo = (label: string, key: "prefix" | "suffix", options: string[]) => (
    <div className="cm-contacts-row">
      <label>{label}</label>
      <input className="cm-input" list={`cm-contacts-${key}`} value={c[key]} onChange={(e) => setC({ [key]: e.target.value } as Partial<Contact>)} />
      <datalist id={`cm-contacts-${key}`}>
        {options.map((o) => (
          <option key={o} value={o} />
        ))}
      </datalist>
    </div>
  );
  const date = (label: string, key: "birthday" | "anniversary") => (
    <div className="cm-contacts-row">
      <label>{label}</label>
      <input className="cm-input" type="date" value={dateToInput(c[key])} onChange={(e) => setC({ [key]: inputToDate(e.target.value) } as Partial<Contact>)} />
    </div>
  );
  return (
    <div className="cm-contacts-details">
      {text("Department:", "department")}
      {text("Manager's name:", "manager")}
      {text("Office:", "office")}
      {text("Assistant's name:", "assistant")}
      {text("Profession:", "profession")}
      <div />
      <div className="cm-contacts-section" style={{ gridColumn: "1 / -1" }}>
        Personal
      </div>
      {text("Nickname:", "nickname")}
      {text("Spouse/Partner:", "spouse")}
      {combo("Title:", "prefix", TITLES)}
      {date("Birthday:", "birthday")}
      {combo("Suffix:", "suffix", SUFFIXES)}
      {date("Anniversary:", "anniversary")}
    </div>
  );
}

// ---- All Fields page ------------------------------------------------------------------------

function AllFields({ c }: { c: Contact }) {
  const phones = phonesByLabel(c.phones);
  const rows: [string, string][] = [
    ["Full Name", fullName(c)],
    ["Title", c.prefix],
    ["First Name", c.givenName],
    ["Middle Name", c.middleName],
    ["Last Name", c.familyName],
    ["Suffix", c.suffix],
    ["Nickname", c.nickname],
    ["File As", c.fileAs],
    ["Company", c.company],
    ["Job Title", c.jobTitle],
    ["Department", c.department],
    ["Office Location", c.office],
    ["Profession", c.profession],
    ["Manager's Name", c.manager],
    ["Assistant's Name", c.assistant],
    ["Spouse/Partner", c.spouse],
    ...c.emails.map((e, i): [string, string] => [i === 0 ? "E-mail" : `E-mail ${i + 1}`, e.value]),
    ...PHONE_TYPES.filter((t) => phones[t.label]).map((t): [string, string] => [`${t.label} Phone`.replace("Fax Phone", "Fax").replace("Phone Phone", "Phone"), phones[t.label]]),
    ...c.addresses.map((a): [string, string] => [`${addressLabel(a.type)} Address`, formatAddress(a)]),
    ["Web Page", c.urls.map((u) => u.value).join(", ")],
    ["IM Address", c.ims.map((i) => i.username).join(", ")],
    ["Birthday", c.birthday ?? "None"],
    ["Anniversary", c.anniversary ?? "None"],
    ["Notes", c.notes],
  ];
  return (
    <div className="cm-contacts-allfields">
      <div className="cm-contacts-allfields-row cm-contacts-allfields-head">
        <span>Name</span>
        <span>Value</span>
      </div>
      {rows.map(([k, v]) => (
        <div className="cm-contacts-allfields-row" key={k}>
          <span>{k}</span>
          <span dir="auto">{v}</span>
        </div>
      ))}
    </div>
  );
}

// ---- dialogs --------------------------------------------------------------------------------

function CheckFullName({ c, onOk, onClose }: { c: Contact; onOk: (p: ReturnType<typeof parseFullName>) => void; onClose: () => void }) {
  const [v, setV] = useState({ prefix: c.prefix, givenName: c.givenName, middleName: c.middleName, familyName: c.familyName, suffix: c.suffix });
  const field = (label: string, key: keyof typeof v, list?: string[]) => (
    <>
      <label>{label}</label>
      <input className="cm-input" dir="auto" list={list ? `cm-contacts-dlg-${key}` : undefined} value={v[key]} onChange={(e) => setV({ ...v, [key]: e.target.value })} />
      {list && (
        <datalist id={`cm-contacts-dlg-${key}`}>
          {list.map((o) => (
            <option key={o} value={o} />
          ))}
        </datalist>
      )}
    </>
  );
  return (
    <Dialog
      title="Check Full Name"
      width={360}
      onClose={onClose}
      footer={
        <>
          <button
            className="cm-button default"
            onClick={() => {
              onOk(v);
              onClose();
            }}
          >
            OK
          </button>
          <button className="cm-button" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <fieldset className="cm-fieldset">
        <legend>Name details</legend>
        <div className="cm-form">
          {field("Title:", "prefix", TITLES)}
          {field("First:", "givenName")}
          {field("Middle:", "middleName")}
          {field("Last:", "familyName")}
          {field("Suffix:", "suffix", SUFFIXES)}
        </div>
      </fieldset>
    </Dialog>
  );
}

function CheckAddress({ type, value, onOk, onClose }: { type: string; value: PostalAddress; onOk: (a: PostalAddress) => void; onClose: () => void }) {
  const [v, setV] = useState<PostalAddress>({ ...value, type });
  const field = (label: string, key: keyof PostalAddress) => (
    <>
      <label>{label}</label>
      <input className="cm-input" dir="auto" value={v[key]} onChange={(e) => setV({ ...v, [key]: e.target.value })} />
    </>
  );
  return (
    <Dialog
      title="Check Address"
      width={380}
      onClose={onClose}
      footer={
        <>
          <button
            className="cm-button default"
            onClick={() => {
              onOk(v);
              onClose();
            }}
          >
            OK
          </button>
          <button className="cm-button" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <fieldset className="cm-fieldset">
        <legend>Address details ({addressLabel(type)})</legend>
        <div className="cm-form" style={{ alignItems: "start" }}>
          <label style={{ paddingTop: 3 }}>Street:</label>
          <textarea className="cm-input" dir="auto" rows={3} value={v.street} onChange={(e) => setV({ ...v, street: e.target.value })} />
          {field("City:", "city")}
          {field("State/Province:", "region")}
          {field("ZIP/Postal code:", "postalCode")}
          {field("Country/Region:", "country")}
        </div>
      </fieldset>
    </Dialog>
  );
}
