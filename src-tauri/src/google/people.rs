//! Typed wrappers for the Google People API (contacts, contact groups, other contacts).
//! https://developers.google.com/people/api/rest

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Api,
    client::{api_reason, api_status},
};

/// Fields we read for a contact.
pub const PERSON_FIELDS: &str = "names,nicknames,emailAddresses,phoneNumbers,addresses,organizations,biographies,birthdays,events,photos,urls,memberships,imClients,relations,occupations,clientData,metadata";

/// Fields we write with updateContact (everything the contact form edits).
pub const UPDATE_FIELDS: &str = "names,nicknames,emailAddresses,phoneNumbers,addresses,organizations,biographies,birthdays,events,urls,memberships,imClients,relations,occupations,clientData";

pub const OTHER_READ_MASK: &str = "names,emailAddresses,metadata";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FieldMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PersonMetadata {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Date {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day: Option<u32>,
}

macro_rules! field_struct {
    ($name:ident { $($(#[$m:meta])* $f:ident : $t:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        pub struct $name {
            #[serde(skip_serializing_if = "Option::is_none")]
            pub metadata: Option<FieldMetadata>,
            $($(#[$m])* #[serde(skip_serializing_if = "Option::is_none")] pub $f: $t,)*
        }
    };
}

field_struct!(Name {
    display_name: Option<String>,
    display_name_last_first: Option<String>,
    unstructured_name: Option<String>,
    given_name: Option<String>,
    middle_name: Option<String>,
    family_name: Option<String>,
    honorific_prefix: Option<String>,
    honorific_suffix: Option<String>,
});
field_struct!(Nickname { value: Option<String> });
field_struct!(EmailAddress {
    value: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    display_name: Option<String>,
});
field_struct!(PhoneNumber {
    value: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
});
field_struct!(Address {
    formatted_value: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    street_address: Option<String>,
    extended_address: Option<String>,
    po_box: Option<String>,
    city: Option<String>,
    region: Option<String>,
    postal_code: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
});
field_struct!(Organization {
    name: Option<String>,
    title: Option<String>,
    department: Option<String>,
    location: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
});
field_struct!(Biography {
    value: Option<String>,
    content_type: Option<String>,
});
field_struct!(Birthday { date: Option<Date>, text: Option<String> });
field_struct!(Event {
    date: Option<Date>,
    #[serde(rename = "type")]
    kind: Option<String>,
});
field_struct!(Photo {
    url: Option<String>,
    default: Option<bool>,
});
field_struct!(Url {
    value: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
});
field_struct!(ImClient {
    username: Option<String>,
    protocol: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
});
field_struct!(Relation {
    person: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
});
field_struct!(Occupation { value: Option<String> });
field_struct!(ClientData { key: Option<String>, value: Option<String> });

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContactGroupMembership {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_group_resource_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_group_id: Option<String>,
}

field_struct!(Membership { contact_group_membership: Option<ContactGroupMembership> });

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Person {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<PersonMetadata>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<Name>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nicknames: Vec<Nickname>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub email_addresses: Vec<EmailAddress>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub phone_numbers: Vec<PhoneNumber>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<Address>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub organizations: Vec<Organization>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub biographies: Vec<Biography>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub birthdays: Vec<Birthday>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub photos: Vec<Photo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<Url>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub memberships: Vec<Membership>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub im_clients: Vec<ImClient>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<Relation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub occupations: Vec<Occupation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub client_data: Vec<ClientData>,
}

impl Person {
    pub fn is_deleted(&self) -> bool {
        self.metadata.as_ref().and_then(|m| m.deleted).unwrap_or(false)
    }

    /// Latest `updateTime` among the sources, as epoch ms.
    pub fn update_time_ms(&self) -> Option<i64> {
        self.metadata
            .as_ref()?
            .sources
            .iter()
            .filter_map(|s| s.update_time.as_deref())
            .filter_map(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|d| d.timestamp_millis())
            .max()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectionsPage {
    pub connections: Vec<Person>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
    pub total_people: Option<i64>,
    pub total_items: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OtherContactsPage {
    pub other_contacts: Vec<Person>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContactGroup {
    pub resource_name: String,
    pub etag: Option<String>,
    pub group_type: Option<String>,
    pub name: String,
    pub formatted_name: Option<String>,
    pub member_count: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContactGroupsPage {
    pub contact_groups: Vec<ContactGroup>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
}

/// Sync token too old → caller must do a full sync. Google: 400 FAILED_PRECONDITION with an
/// ErrorInfo reason EXPIRED_SYNC_TOKEN ("Sync token is expired. Clear local cache and retry call
/// without the sync token."); tokens live 7 days. 410 is accepted too.
pub fn is_expired_sync_token(e: &anyhow::Error) -> bool {
    match api_status(e) {
        Some(410) => true,
        Some(400) => {
            api_reason(e) == Some("EXPIRED_SYNC_TOKEN") || {
                let s = e.to_string().to_lowercase();
                s.contains("sync token") && (s.contains("expired") || s.contains("invalid"))
            }
        }
        _ => false,
    }
}

/// A sync token issued for different request parameters (Google: 400 "Request parameters must
/// match the request that produced the sync token") — e.g. one saved by an older version of the
/// app that asked for other personFields. It can never work again, so sync starts over.
pub fn is_mismatched_sync_token(e: &anyhow::Error) -> bool {
    api_status(e) == Some(400) && {
        let s = e.to_string().to_lowercase();
        s.contains("sync token") && s.contains("must match")
    }
}

/// Stale etag on update (Google: 400 FAILED_PRECONDITION "Request person.etag is different than
/// the current person.etag"; 409 is also accepted).
pub fn is_stale_etag(e: &anyhow::Error) -> bool {
    match api_status(e) {
        Some(409) => true,
        Some(400) => {
            api_reason(e) == Some("FAILED_PRECONDITION") && !is_expired_sync_token(e) || e.to_string().to_lowercase().contains("etag")
        }
        _ => false,
    }
}

/// people.connections.list — one page. `sync_token` for incremental; `request_sync_token` on full.
pub async fn list_connections(
    api: &Api,
    page_token: Option<&str>,
    sync_token: Option<&str>,
    page_size: usize,
) -> anyhow::Result<ConnectionsPage> {
    let url = format!("{}/people/me/connections", api.urls.people);
    let mut q: Vec<(&str, String)> =
        vec![("personFields", PERSON_FIELDS.to_string()), ("pageSize", page_size.to_string()), ("requestSyncToken", "true".to_string())];
    if let Some(t) = page_token {
        q.push(("pageToken", t.to_string()));
    }
    // no sortOrder: it is ignored when a sync token is requested, and every call that uses the
    // sync token must repeat the first call's parameters exactly
    if let Some(t) = sync_token {
        q.push(("syncToken", t.to_string()));
    }
    api.get(&url, &q).await
}

pub async fn get_person(api: &Api, resource_name: &str) -> anyhow::Result<Person> {
    let url = format!("{}/{}", api.urls.people, resource_path(resource_name));
    api.get(&url, &[("personFields", PERSON_FIELDS.to_string())]).await
}

pub async fn create_contact(api: &Api, person: &Person) -> anyhow::Result<Person> {
    let url = format!("{}/people:createContact", api.urls.people);
    api.post(&url, &[("personFields", PERSON_FIELDS.to_string())], serde_json::to_value(person)?).await
}

/// `person.etag` must be the current etag, else Google answers 400 FAILED_PRECONDITION.
/// Replaces every field in `UPDATE_FIELDS`; prefer `update_contact_fields` with only what changed.
pub async fn update_contact(api: &Api, resource_name: &str, person: &Person) -> anyhow::Result<Person> {
    let all: Vec<&str> = UPDATE_FIELDS.split(',').collect();
    update_contact_fields(api, resource_name, person, &all).await
}

/// updateContact for the given `updatePersonFields` only: Google replaces each listed field as a
/// whole and leaves the others alone.
pub async fn update_contact_fields(api: &Api, resource_name: &str, person: &Person, fields: &[&str]) -> anyhow::Result<Person> {
    let url = format!("{}/{}:updateContact", api.urls.people, resource_path(resource_name));
    api.patch(&url, &[("updatePersonFields", fields.join(",")), ("personFields", PERSON_FIELDS.to_string())], serde_json::to_value(person)?)
        .await
}

pub async fn delete_contact(api: &Api, resource_name: &str) -> anyhow::Result<()> {
    let url = format!("{}/{}:deleteContact", api.urls.people, resource_path(resource_name));
    api.delete(&url, &[]).await
}

/// people.updateContactPhoto — `bytes` is the raw image.
pub async fn update_contact_photo(api: &Api, resource_name: &str, bytes: &[u8]) -> anyhow::Result<Person> {
    use base64::Engine;
    let url = format!("{}/{}:updateContactPhoto", api.urls.people, resource_path(resource_name));
    let body = json!({
        "photoBytes": base64::engine::general_purpose::STANDARD.encode(bytes),
        "personFields": PERSON_FIELDS,
    });
    let v: Value = api.patch(&url, &[], body).await?;
    Ok(serde_json::from_value(v.get("person").cloned().unwrap_or(Value::Null)).unwrap_or_default())
}

pub async fn list_contact_groups(api: &Api, page_token: Option<&str>) -> anyhow::Result<ContactGroupsPage> {
    let url = format!("{}/contactGroups", api.urls.people);
    let mut q: Vec<(&str, String)> =
        vec![("pageSize", "1000".to_string()), ("groupFields", "name,groupType,memberCount,metadata".to_string())];
    if let Some(t) = page_token {
        q.push(("pageToken", t.to_string()));
    }
    api.get(&url, &q).await
}

/// otherContacts.list — one page (needs the contacts.other.readonly scope).
pub async fn list_other_contacts(
    api: &Api,
    page_token: Option<&str>,
    sync_token: Option<&str>,
    page_size: usize,
) -> anyhow::Result<OtherContactsPage> {
    let url = format!("{}/otherContacts", api.urls.people);
    let mut q: Vec<(&str, String)> =
        vec![("readMask", OTHER_READ_MASK.to_string()), ("pageSize", page_size.to_string()), ("requestSyncToken", "true".to_string())];
    if let Some(t) = page_token {
        q.push(("pageToken", t.to_string()));
    }
    if let Some(t) = sync_token {
        q.push(("syncToken", t.to_string()));
    }
    api.get(&url, &q).await
}

/// "people/c123" → "people/c123" with the id segment percent-encoded.
fn resource_path(resource_name: &str) -> String {
    match resource_name.split_once('/') {
        Some((kind, id)) => format!("{kind}/{}", super::seg(id)),
        None => super::seg(resource_name),
    }
}
