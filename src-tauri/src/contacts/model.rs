//! The contact record (as stored locally and sent to the UI) and its mapping to/from a People API
//! `Person`.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::google::people::{
    Address, Biography, Birthday, ClientData, ContactGroupMembership, Date, EmailAddress, Event, ImClient, Membership, Name, Nickname,
    Occupation, Organization, Person, PhoneNumber, Relation, Url,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Email {
    pub value: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Typed {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PostalAddress {
    #[serde(rename = "type")]
    pub kind: String,
    pub street: String,
    pub city: String,
    pub region: String,
    pub postal_code: String,
    pub country: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Im {
    pub protocol: String,
    pub username: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Contact {
    pub id: i64,
    pub account_id: i64,
    pub resource_name: Option<String>,
    pub etag: Option<String>,
    pub display_name: String,
    pub given_name: String,
    pub middle_name: String,
    pub family_name: String,
    pub prefix: String,
    pub suffix: String,
    pub nickname: String,
    pub file_as: String,
    pub company: String,
    pub job_title: String,
    pub department: String,
    pub office: String,
    pub profession: String,
    pub manager: String,
    pub assistant: String,
    pub spouse: String,
    pub emails: Vec<Email>,
    pub phones: Vec<Typed>,
    pub addresses: Vec<PostalAddress>,
    pub mailing_address: String,
    pub urls: Vec<Typed>,
    pub ims: Vec<Im>,
    pub birthday: Option<String>,
    pub anniversary: Option<String>,
    pub notes: String,
    pub photo_url: Option<String>,
    pub groups: Vec<String>,
    pub starred: bool,
    /// Google's last update time (People `metadata.sources[].updateTime`) or the last local edit.
    pub updated: i64,
    /// When the contact first appeared in ClassicMail ("Date Added"); never touched by sync.
    pub first_seen_at: i64,
    /// local changes not yet on Google
    pub dirty: bool,
}

fn nz(s: &Option<String>) -> String {
    s.clone().unwrap_or_default().trim().to_string()
}

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

fn join_nonempty(parts: &[&str], sep: &str) -> String {
    parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(sep)
}

fn date_to_string(d: &Date) -> Option<String> {
    let (m, day) = (d.month?, d.day?);
    Some(match d.year {
        Some(y) if y > 0 => format!("{y:04}-{m:02}-{day:02}"),
        _ => format!("--{m:02}-{day:02}"),
    })
}

fn string_to_date(s: &str) -> Option<Date> {
    let s = s.trim();
    if let Some(md) = s.strip_prefix("--") {
        let (m, d) = md.split_once('-')?;
        return Some(Date { year: None, month: m.parse().ok(), day: d.parse().ok() });
    }
    let mut it = s.split('-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    Some(Date { year: Some(y), month: Some(m), day: Some(d) })
}

/// For updateContact: the person fields where `local` differs from Google's current copy `remote`,
/// and the Person to send. Google replaces each listed field wholesale, so fields the user didn't
/// touch are left out entirely, and within a changed field the entries ClassicMail doesn't model
/// (other events and relations, other apps' client data, extra organizations / nicknames /
/// occupations) are carried over from `remote` instead of being wiped.
pub fn update_for(local: &Contact, remote: &Person) -> (Vec<&'static str>, Person) {
    let base = Contact::from_person(remote);
    let mut p = local.to_person(remote.etag.clone());
    let mut fields: Vec<&'static str> = Vec::new();
    let differs = |a: &[&str], b: &[&str]| a != b;
    if differs(
        &[&local.given_name, &local.middle_name, &local.family_name, &local.prefix, &local.suffix],
        &[&base.given_name, &base.middle_name, &base.family_name, &base.prefix, &base.suffix],
    ) {
        fields.push("names");
    }
    if local.nickname != base.nickname {
        fields.push("nicknames");
        p.nicknames.extend(remote.nicknames.iter().skip(1).cloned());
    }
    if local.emails != base.emails {
        fields.push("emailAddresses");
    }
    if local.phones != base.phones {
        fields.push("phoneNumbers");
    }
    if local.addresses != base.addresses {
        fields.push("addresses");
    }
    if differs(
        &[&local.company, &local.job_title, &local.department, &local.office],
        &[&base.company, &base.job_title, &base.department, &base.office],
    ) {
        fields.push("organizations");
        p.organizations.extend(remote.organizations.iter().skip(1).cloned());
    }
    if local.profession != base.profession {
        fields.push("occupations");
        p.occupations.extend(remote.occupations.iter().skip(1).cloned());
    }
    if differs(&[&local.manager, &local.assistant, &local.spouse], &[&base.manager, &base.assistant, &base.spouse]) {
        fields.push("relations");
        let ours = ["manager", "assistant", "spouse", "domesticPartner"];
        p.relations.extend(remote.relations.iter().filter(|r| !ours.contains(&r.kind.as_deref().unwrap_or(""))).cloned());
    }
    if local.urls != base.urls {
        fields.push("urls");
    }
    if local.ims != base.ims {
        fields.push("imClients");
    }
    if local.birthday != base.birthday {
        fields.push("birthdays");
    }
    if local.anniversary != base.anniversary {
        fields.push("events");
        p.events.extend(remote.events.iter().filter(|e| e.kind.as_deref() != Some("anniversary")).cloned());
    }
    if local.notes != base.notes {
        fields.push("biographies");
    }
    let mut lg = local.groups.clone();
    let mut bg = base.groups.clone();
    lg.sort();
    bg.sort();
    if local.starred != base.starred || lg != bg {
        fields.push("memberships");
    }
    if local.file_as != base.file_as || local.mailing_address != base.mailing_address {
        fields.push("clientData");
        let ours = ["fileAs", "mailingAddress"];
        p.client_data.extend(remote.client_data.iter().filter(|c| !ours.contains(&c.key.as_deref().unwrap_or(""))).cloned());
    }
    (fields, p)
}

/// System groups we treat as flags rather than folders.
pub const MY_CONTACTS: &str = "contactGroups/myContacts";
pub const STARRED: &str = "contactGroups/starred";

impl Contact {
    /// "Prefix Given Middle Family Suffix", else company, else first e-mail.
    pub fn compute_display_name(&self) -> String {
        let n = join_nonempty(&[&self.prefix, &self.given_name, &self.middle_name, &self.family_name, &self.suffix], " ");
        if !n.is_empty() {
            return n;
        }
        if !self.company.trim().is_empty() {
            return self.company.trim().to_string();
        }
        self.emails.first().map(|e| e.value.clone()).unwrap_or_default()
    }

    /// Outlook's default "File as": "Last, First", else the full name, else company / e-mail.
    pub fn default_file_as(&self) -> String {
        let first = join_nonempty(&[&self.given_name, &self.middle_name], " ");
        let last = join_nonempty(&[&self.family_name, &self.suffix], " ");
        match (first.is_empty(), last.is_empty()) {
            (false, false) => format!("{last}, {first}"),
            (true, false) => last,
            (false, true) => first,
            _ => self.compute_display_name(),
        }
    }

    /// Fill derived fields and tidy input before saving.
    pub fn normalize(&mut self) {
        for s in [
            &mut self.given_name,
            &mut self.middle_name,
            &mut self.family_name,
            &mut self.prefix,
            &mut self.suffix,
            &mut self.nickname,
            &mut self.company,
            &mut self.job_title,
            &mut self.department,
            &mut self.office,
            &mut self.profession,
            &mut self.manager,
            &mut self.assistant,
            &mut self.spouse,
            &mut self.file_as,
        ] {
            *s = s.trim().to_string();
        }
        self.emails.retain(|e| !e.value.trim().is_empty());
        for e in &mut self.emails {
            e.value = e.value.trim().to_string();
            e.display_name = e.display_name.trim().to_string();
        }
        self.phones.retain(|p| !p.value.trim().is_empty());
        self.urls.retain(|u| !u.value.trim().is_empty());
        self.ims.retain(|u| !u.username.trim().is_empty());
        self.addresses.retain(|a| {
            !(a.street.trim().is_empty()
                && a.city.trim().is_empty()
                && a.region.trim().is_empty()
                && a.postal_code.trim().is_empty()
                && a.country.trim().is_empty())
        });
        self.groups.retain(|g| g != MY_CONTACTS && g != STARRED);
        // drop repeats anywhere in the list, keeping the first
        let mut seen = std::collections::HashSet::new();
        self.groups.retain(|g| seen.insert(g.clone()));
        self.birthday = self.birthday.as_deref().and_then(string_to_date).as_ref().and_then(date_to_string);
        self.anniversary = self.anniversary.as_deref().and_then(string_to_date).as_ref().and_then(date_to_string);
        self.display_name = self.compute_display_name();
        if self.file_as.is_empty() {
            self.file_as = self.default_file_as();
        }
    }

    /// Build from a Google person (fields we don't model are ignored).
    pub fn from_person(p: &Person) -> Contact {
        let mut c = Contact { resource_name: p.resource_name.clone(), etag: p.etag.clone(), ..Default::default() };
        if let Some(n) = p.names.first() {
            c.given_name = nz(&n.given_name);
            c.middle_name = nz(&n.middle_name);
            c.family_name = nz(&n.family_name);
            c.prefix = nz(&n.honorific_prefix);
            c.suffix = nz(&n.honorific_suffix);
            c.display_name = nz(&n.display_name);
            if c.given_name.is_empty() && c.family_name.is_empty() {
                // only an unstructured name: treat it as the given name
                c.given_name = nz(&n.unstructured_name).chars().take(200).collect();
                if c.given_name.is_empty() {
                    c.given_name = c.display_name.clone();
                }
            }
        }
        c.nickname = p.nicknames.first().map(|n| nz(&n.value)).unwrap_or_default();
        c.emails = p
            .email_addresses
            .iter()
            .filter_map(|e| Some(Email { value: opt(e.value.as_deref()?)?, kind: nz(&e.kind), display_name: nz(&e.display_name) }))
            .collect();
        c.phones = p.phone_numbers.iter().filter_map(|e| Some(Typed { value: opt(e.value.as_deref()?)?, kind: nz(&e.kind) })).collect();
        c.addresses = p
            .addresses
            .iter()
            .map(|src| {
                let mut a = PostalAddress {
                    kind: nz(&src.kind),
                    street: join_nonempty(&[&nz(&src.street_address), &nz(&src.extended_address), &nz(&src.po_box)], "\n"),
                    city: nz(&src.city),
                    region: nz(&src.region),
                    postal_code: nz(&src.postal_code),
                    country: nz(&src.country),
                };
                // address with only a formatted value (this address's own, not another of the same type)
                if a.street.is_empty()
                    && a.city.is_empty()
                    && a.country.is_empty()
                    && let Some(f) = src.formatted_value.clone()
                {
                    a.street = f;
                }
                a
            })
            .collect();
        if let Some(o) = p.organizations.first() {
            c.company = nz(&o.name);
            c.job_title = nz(&o.title);
            c.department = nz(&o.department);
            c.office = nz(&o.location);
        }
        c.profession = p.occupations.first().map(|o| nz(&o.value)).unwrap_or_default();
        for r in &p.relations {
            match nz(&r.kind).as_str() {
                "manager" => c.manager = nz(&r.person),
                "assistant" => c.assistant = nz(&r.person),
                "spouse" | "domesticPartner" => c.spouse = nz(&r.person),
                _ => {}
            }
        }
        c.urls = p.urls.iter().filter_map(|u| Some(Typed { value: opt(u.value.as_deref()?)?, kind: nz(&u.kind) })).collect();
        c.ims =
            p.im_clients.iter().filter_map(|i| Some(Im { username: opt(i.username.as_deref()?)?, protocol: nz(&i.protocol) })).collect();
        c.birthday = p.birthdays.iter().find_map(|b| b.date.as_ref().and_then(date_to_string));
        c.anniversary =
            p.events.iter().find(|e| e.kind.as_deref() == Some("anniversary")).and_then(|e| e.date.as_ref().and_then(date_to_string));
        c.notes = p.biographies.first().map(|b| b.value.clone().unwrap_or_default()).unwrap_or_default();
        c.photo_url = p.photos.iter().find(|ph| ph.default != Some(true)).and_then(|ph| ph.url.clone());
        let groups: Vec<String> =
            p.memberships.iter().filter_map(|m| m.contact_group_membership.as_ref()?.contact_group_resource_name.clone()).collect();
        c.starred = groups.iter().any(|g| g == STARRED);
        c.groups = groups.into_iter().filter(|g| g != MY_CONTACTS && g != STARRED).collect();
        for cd in &p.client_data {
            match cd.key.as_deref() {
                Some("fileAs") => c.file_as = nz(&cd.value),
                Some("mailingAddress") => c.mailing_address = nz(&cd.value),
                _ => {}
            }
        }
        c.updated = p.update_time_ms().unwrap_or_else(crate::db::util::now_ms);
        c.normalize();
        c
    }

    /// The Google person for create/update (no resourceName; `etag` set for updates).
    pub fn to_person(&self, etag: Option<String>) -> Person {
        let mut p = Person { etag, ..Default::default() };
        if !(self.given_name.is_empty()
            && self.family_name.is_empty()
            && self.middle_name.is_empty()
            && self.prefix.is_empty()
            && self.suffix.is_empty())
        {
            p.names.push(Name {
                given_name: opt(&self.given_name),
                middle_name: opt(&self.middle_name),
                family_name: opt(&self.family_name),
                honorific_prefix: opt(&self.prefix),
                honorific_suffix: opt(&self.suffix),
                ..Default::default()
            });
        }
        if let Some(n) = opt(&self.nickname) {
            p.nicknames.push(Nickname { value: Some(n), ..Default::default() });
        }
        p.email_addresses = self
            .emails
            .iter()
            .map(|e| EmailAddress {
                value: Some(e.value.clone()),
                kind: opt(&e.kind),
                display_name: opt(&e.display_name),
                ..Default::default()
            })
            .collect();
        p.phone_numbers =
            self.phones.iter().map(|t| PhoneNumber { value: Some(t.value.clone()), kind: opt(&t.kind), ..Default::default() }).collect();
        p.addresses = self
            .addresses
            .iter()
            .map(|a| Address {
                kind: opt(&a.kind),
                street_address: opt(&a.street),
                city: opt(&a.city),
                region: opt(&a.region),
                postal_code: opt(&a.postal_code),
                country: opt(&a.country),
                ..Default::default()
            })
            .collect();
        if !(self.company.is_empty() && self.job_title.is_empty() && self.department.is_empty() && self.office.is_empty()) {
            p.organizations.push(Organization {
                name: opt(&self.company),
                title: opt(&self.job_title),
                department: opt(&self.department),
                location: opt(&self.office),
                kind: Some("work".into()),
                ..Default::default()
            });
        }
        if let Some(v) = opt(&self.profession) {
            p.occupations.push(Occupation { value: Some(v), ..Default::default() });
        }
        for (kind, person) in [("manager", &self.manager), ("assistant", &self.assistant), ("spouse", &self.spouse)] {
            if let Some(v) = opt(person) {
                p.relations.push(Relation { person: Some(v), kind: Some(kind.into()), ..Default::default() });
            }
        }
        p.urls = self.urls.iter().map(|u| Url { value: Some(u.value.clone()), kind: opt(&u.kind), ..Default::default() }).collect();
        p.im_clients = self
            .ims
            .iter()
            .map(|i| ImClient { username: Some(i.username.clone()), protocol: opt(&i.protocol), ..Default::default() })
            .collect();
        if let Some(d) = self.birthday.as_deref().and_then(string_to_date) {
            p.birthdays.push(Birthday { date: Some(d), ..Default::default() });
        }
        if let Some(d) = self.anniversary.as_deref().and_then(string_to_date) {
            p.events.push(Event { date: Some(d), kind: Some("anniversary".into()), ..Default::default() });
        }
        if !self.notes.is_empty() {
            p.biographies.push(Biography {
                value: Some(self.notes.clone()),
                content_type: Some("TEXT_PLAIN".into()),
                ..Default::default()
            });
        }
        let mut groups = vec![MY_CONTACTS.to_string()];
        if self.starred {
            groups.push(STARRED.to_string());
        }
        groups.extend(self.groups.iter().cloned());
        p.memberships = groups
            .into_iter()
            .map(|g| Membership {
                contact_group_membership: Some(ContactGroupMembership { contact_group_resource_name: Some(g), contact_group_id: None }),
                ..Default::default()
            })
            .collect();
        // Outlook-only fields ride along as client data
        if self.file_as != self.default_file_as() {
            p.client_data.push(ClientData { key: Some("fileAs".into()), value: Some(self.file_as.clone()), ..Default::default() });
        }
        if !self.mailing_address.is_empty() {
            p.client_data.push(ClientData {
                key: Some("mailingAddress".into()),
                value: Some(self.mailing_address.clone()),
                ..Default::default()
            });
        }
        p
    }
}

// ---- database ---------------------------------------------------------------------------------

pub const COLUMNS: &str = "id, account_id, resource_name, etag, display_name, given_name, middle_name, family_name, prefix, suffix, \
    nickname, file_as, company, job_title, department, office, profession, manager, assistant, spouse, emails, phones, addresses, \
    mailing_address, urls, ims, birthday, anniversary, notes, photo_url, groups, starred, updated, first_seen_at, dirty";

fn json_col<T: serde::de::DeserializeOwned + Default>(r: &Row, name: &str) -> rusqlite::Result<T> {
    let s: String = r.get(name)?;
    Ok(serde_json::from_str(&s).unwrap_or_default())
}

pub fn from_row(r: &Row) -> rusqlite::Result<Contact> {
    Ok(Contact {
        id: r.get("id")?,
        account_id: r.get("account_id")?,
        resource_name: r.get("resource_name")?,
        etag: r.get("etag")?,
        display_name: r.get("display_name")?,
        given_name: r.get("given_name")?,
        middle_name: r.get("middle_name")?,
        family_name: r.get("family_name")?,
        prefix: r.get("prefix")?,
        suffix: r.get("suffix")?,
        nickname: r.get("nickname")?,
        file_as: r.get("file_as")?,
        company: r.get("company")?,
        job_title: r.get("job_title")?,
        department: r.get("department")?,
        office: r.get("office")?,
        profession: r.get("profession")?,
        manager: r.get("manager")?,
        assistant: r.get("assistant")?,
        spouse: r.get("spouse")?,
        emails: json_col(r, "emails")?,
        phones: json_col(r, "phones")?,
        addresses: json_col(r, "addresses")?,
        mailing_address: r.get("mailing_address")?,
        urls: json_col(r, "urls")?,
        ims: json_col(r, "ims")?,
        birthday: r.get("birthday")?,
        anniversary: r.get("anniversary")?,
        notes: r.get("notes")?,
        photo_url: r.get("photo_url")?,
        groups: json_col(r, "groups")?,
        starred: r.get::<_, i64>("starred")? != 0,
        updated: r.get("updated")?,
        first_seen_at: r.get("first_seen_at")?,
        dirty: r.get::<_, i64>("dirty")? != 0,
    })
}

pub fn get(c: &Connection, id: i64) -> anyhow::Result<Option<Contact>> {
    Ok(c.query_row(&format!("SELECT {COLUMNS} FROM contacts WHERE id=?1"), [id], from_row).optional()?)
}

fn j<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

/// Write every editable column of `ct` into row `id`.
fn write_fields(c: &Connection, id: i64, ct: &Contact) -> anyhow::Result<()> {
    c.execute(
        "UPDATE contacts SET display_name=?2, given_name=?3, middle_name=?4, family_name=?5, prefix=?6, suffix=?7, nickname=?8, \
         file_as=?9, company=?10, job_title=?11, department=?12, office=?13, profession=?14, manager=?15, assistant=?16, spouse=?17, \
         emails=?18, phones=?19, addresses=?20, mailing_address=?21, urls=?22, ims=?23, birthday=?24, anniversary=?25, notes=?26, \
         groups=?27, starred=?28 WHERE id=?1",
        params![
            id,
            ct.display_name,
            ct.given_name,
            ct.middle_name,
            ct.family_name,
            ct.prefix,
            ct.suffix,
            ct.nickname,
            ct.file_as,
            ct.company,
            ct.job_title,
            ct.department,
            ct.office,
            ct.profession,
            ct.manager,
            ct.assistant,
            ct.spouse,
            j(&ct.emails),
            j(&ct.phones),
            j(&ct.addresses),
            ct.mailing_address,
            j(&ct.urls),
            j(&ct.ims),
            ct.birthday,
            ct.anniversary,
            ct.notes,
            j(&ct.groups),
            ct.starred as i64,
        ],
    )?;
    Ok(())
}

/// New local contact (not yet on Google). Returns its id.
pub fn insert_local(c: &Connection, account_id: i64, ct: &Contact) -> anyhow::Result<i64> {
    c.execute(
        "INSERT INTO contacts(account_id, updated, first_seen_at, dirty, local_rev, photo_url) VALUES (?1, ?2, ?2, 1, 1, ?3)",
        params![account_id, crate::db::util::now_ms(), ct.photo_url],
    )?;
    let id = c.last_insert_rowid();
    write_fields(c, id, ct)?;
    Ok(id)
}

/// Local edit of an existing contact.
pub fn update_local(c: &Connection, id: i64, ct: &Contact) -> anyhow::Result<()> {
    write_fields(c, id, ct)?;
    c.execute("UPDATE contacts SET dirty=1, local_rev=local_rev+1, updated=?2 WHERE id=?1", params![id, crate::db::util::now_ms()])?;
    Ok(())
}

pub fn local_rev(c: &Connection, id: i64) -> anyhow::Result<Option<i64>> {
    Ok(c.query_row("SELECT local_rev FROM contacts WHERE id=?1", [id], |r| r.get(0)).optional()?)
}

/// Apply what Google returned after a push: identity, etag, server-computed fields.
/// `rev` is the local revision that was pushed; if the user edited again meanwhile, keep dirty.
pub fn apply_pushed(c: &Connection, id: i64, rev: i64, remote: &Contact) -> anyhow::Result<()> {
    let cur = local_rev(c, id)?;
    c.execute(
        "UPDATE contacts SET resource_name=?2, etag=?3, photo_url=coalesce(?4, photo_url), updated=?5 WHERE id=?1",
        params![id, remote.resource_name, remote.etag, remote.photo_url, remote.updated],
    )?;
    if cur == Some(rev) {
        write_fields(c, id, remote)?;
        c.execute("UPDATE contacts SET dirty=0 WHERE id=?1", [id])?;
    }
    Ok(())
}

/// "Date Added" for a contact that first arrives from Google: now, or its Google update time when
/// that is earlier (Google has no creation time; on the first full sync this keeps years-old
/// contacts from all looking added today, and a contact new on Google has updateTime ≈ creation).
pub fn first_seen(remote_updated: i64) -> i64 {
    let now = crate::db::util::now_ms();
    if remote_updated > 0 { remote_updated.min(now) } else { now }
}

/// Insert or refresh a contact pulled from Google. Rows with unpushed local edits are left alone.
/// Returns true if anything was written.
pub fn upsert_remote(c: &Connection, account_id: i64, remote: &Contact) -> anyhow::Result<bool> {
    let rn = remote.resource_name.as_deref().unwrap_or_default();
    let existing: Option<(i64, bool, Option<String>)> = c
        .query_row("SELECT id, dirty, etag FROM contacts WHERE account_id=?1 AND resource_name=?2", params![account_id, rn], |r| {
            Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?))
        })
        .optional()?;
    let id = match existing {
        Some((_, true, _)) => return Ok(false),
        Some((_, false, etag)) if etag.is_some() && etag == remote.etag => return Ok(false),
        Some((id, false, _)) => id,
        None => {
            c.execute(
                "INSERT INTO contacts(account_id, resource_name, updated, first_seen_at) VALUES (?1, ?2, ?3, ?4)",
                params![account_id, rn, remote.updated, first_seen(remote.updated)],
            )?;
            c.last_insert_rowid()
        }
    };
    write_fields(c, id, remote)?;
    c.execute(
        "UPDATE contacts SET etag=?2, photo_url=?3, updated=?4 WHERE id=?1",
        params![id, remote.etag, remote.photo_url, remote.updated],
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_person() {
        let mut c = Contact {
            given_name: "Sarah".into(),
            family_name: "Cohen".into(),
            company: "Harbor Capital".into(),
            job_title: "Portfolio Manager".into(),
            emails: vec![Email { value: "sarah@harbor.example".into(), kind: "work".into(), display_name: String::new() }],
            phones: vec![Typed { kind: "mobile".into(), value: "+972 52 555 0100".into() }],
            addresses: vec![PostalAddress {
                kind: "work".into(),
                street: "1 Main St".into(),
                city: "Tel Aviv".into(),
                country: "Israel".into(),
                ..Default::default()
            }],
            mailing_address: "work".into(),
            birthday: Some("1980-03-04".into()),
            anniversary: Some("--06-07".into()),
            manager: "Dan".into(),
            groups: vec!["contactGroups/abc".into()],
            starred: true,
            file_as: "Harbor Capital (Cohen, Sarah)".into(),
            notes: "hello".into(),
            ..Default::default()
        };
        c.normalize();
        assert_eq!(c.display_name, "Sarah Cohen");
        let p = c.to_person(Some("e1".into()));
        let mut back = Contact::from_person(&p);
        back.updated = c.updated;
        back.etag = c.etag.clone();
        assert_eq!(back.display_name, "Sarah Cohen");
        assert_eq!(back.file_as, c.file_as);
        assert_eq!(back.emails, c.emails);
        assert_eq!(back.phones, c.phones);
        assert_eq!(back.addresses, c.addresses);
        assert_eq!(back.birthday.as_deref(), Some("1980-03-04"));
        assert_eq!(back.anniversary.as_deref(), Some("--06-07"));
        assert_eq!(back.groups, vec!["contactGroups/abc".to_string()]);
        assert!(back.starred);
        assert_eq!(back.manager, "Dan");
        assert_eq!(back.mailing_address, "work");
    }

    #[test]
    fn formatted_only_address_keeps_its_own_text_and_groups_are_unique() {
        let p = Person {
            addresses: vec![
                Address {
                    kind: Some("work".into()),
                    city: Some("Tel Aviv".into()),
                    formatted_value: Some("1 Main St, Tel Aviv".into()),
                    ..Default::default()
                },
                Address { kind: Some("work".into()), formatted_value: Some("PO Box 7, Haifa".into()), ..Default::default() },
            ],
            ..Default::default()
        };
        let c = Contact::from_person(&p);
        assert_eq!(c.addresses[0].street, "");
        assert_eq!(c.addresses[1].street, "PO Box 7, Haifa");
        let mut c = Contact {
            given_name: "Noa".into(),
            groups: vec!["g/a".into(), "g/b".into(), "g/a".into(), MY_CONTACTS.into()],
            ..Default::default()
        };
        c.normalize();
        assert_eq!(c.groups, vec!["g/a".to_string(), "g/b".to_string()]);
    }

    #[test]
    fn default_file_as_variants() {
        let mut c = Contact { company: "Acme".into(), ..Default::default() };
        c.normalize();
        assert_eq!(c.file_as, "Acme");
        let mut c = Contact { given_name: "Noa".into(), family_name: "Shapiro".into(), ..Default::default() };
        c.normalize();
        assert_eq!(c.file_as, "Shapiro, Noa");
    }
}
