//! Fake People API (see fake_google/mod.rs for conventions): contacts (`people/me/connections`,
//! `people:createContact`, `people/{id}:updateContact|deleteContact|updateContactPhoto`), contact
//! groups and "other contacts", with sync tokens (incl. deletions + expiry), paging and etags.
//!
//! People are stored as Google-shaped JSON so whatever the client writes is echoed back faithfully.

use std::collections::{BTreeMap, HashSet};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::Method,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    Shared, User, paginate,
    seed::{self, DAY_MS, PEOPLE, Rng},
};

const SYNC_TOKEN_TTL_MS: i64 = 7 * DAY_MS;

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    pub persons: Vec<FakePerson>,
    pub groups: Vec<FakeGroup>,
    pub other: Vec<FakePerson>,
    /// change sequence, bumped on every write (sync tokens remember it)
    pub seq: u64,
    /// bumping this invalidates every sync token issued so far (tests / demo)
    pub token_epoch: u64,
    /// id counter for new contacts
    pub next_id: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FakePerson {
    pub id: String,
    pub etag: String,
    pub seq: u64,
    pub updated_ms: i64,
    pub deleted: bool,
    /// Person fields (names, emailAddresses, ...) without resourceName/etag/metadata.
    pub data: Value,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FakeGroup {
    pub id: String,
    pub name: String,
    pub system: bool,
    pub etag: String,
    pub updated_ms: i64,
}

impl Store {
    fn bump(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn new_id(&mut self) -> String {
        self.next_id += 1;
        // Google ids look like "c1234567890123456789"
        format!("c{}", 7_000_000_000_000_000_000u64 + self.next_id * 7919)
    }

    /// `params`: fingerprint of the listing's other parameters — Google requires the calls that
    /// use a sync token to repeat them exactly ("all other request parameters must match").
    fn sync_token(&self, params: &str) -> String {
        format!("ST{}.{}.{}.{}", self.token_epoch, self.seq, seed::now_ms(), fingerprint(params))
    }

    /// Parse a token we issued: Ok(seq) if valid, Err(response) if expired / mismatched.
    fn token_seq(&self, token: &str, params: &str) -> Result<u64, Response> {
        let parsed = (|| {
            let rest = token.strip_prefix("ST")?;
            let mut it = rest.split('.');
            let epoch: u64 = it.next()?.parse().ok()?;
            let seq: u64 = it.next()?.parse().ok()?;
            let issued: i64 = it.next()?.parse().ok()?;
            let fp = it.next().unwrap_or("").to_string();
            Some((epoch, seq, issued, fp))
        })();
        let Some((epoch, seq, issued, fp)) = parsed else { return Err(expired()) };
        if epoch != self.token_epoch || seed::now_ms() - issued > SYNC_TOKEN_TTL_MS {
            return Err(expired());
        }
        if fp != fingerprint(params) {
            return Err(rpc_error(
                400,
                "INVALID_ARGUMENT",
                "Request parameters must match the request that produced the sync token.",
                None,
            ));
        }
        Ok(seq)
    }

    fn person_mut(&mut self, id: &str) -> Option<&mut FakePerson> {
        self.persons.iter_mut().find(|p| p.id == id && !p.deleted)
    }

    fn member_count(&self, group_rn: &str) -> usize {
        self.persons.iter().filter(|p| !p.deleted && memberships_of(&p.data).iter().any(|g| g == group_rn)).count()
    }
}

fn fingerprint(params: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    params.hash(&mut h);
    format!("{:x}", h.finish() & 0xffff_ffff)
}

/// People API errors are google.rpc.Status: `status` + `details[ErrorInfo]`, no `errors` array.
fn rpc_error(code: u16, status: &str, message: &str, reason: Option<&str>) -> Response {
    let mut err = json!({ "code": code, "message": message, "status": status });
    if let Some(r) = reason {
        err["details"] = json!([{ "@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": r, "domain": "people.googleapis.com" }]);
    }
    let sc = axum::http::StatusCode::from_u16(code).unwrap_or(axum::http::StatusCode::BAD_REQUEST);
    (sc, Json(json!({ "error": err }))).into_response()
}

fn bad_arg(message: &str) -> Response {
    rpc_error(400, "INVALID_ARGUMENT", message, None)
}

fn etag(n: u64) -> String {
    format!("%EgUBAgMFLhoEAQIFByIM{:08x}", n.wrapping_mul(2654435761) & 0xffff_ffff)
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

// ---- JSON helpers ---------------------------------------------------------------------------

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn join_nonempty(parts: &[&str], sep: &str) -> String {
    parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(sep)
}

fn memberships_of(data: &Value) -> Vec<String> {
    data.get("memberships")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|m| m["contactGroupMembership"]["contactGroupResourceName"].as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn ensure_my_contacts(data: &mut Value) {
    if memberships_of(data).iter().any(|g| g == "contactGroups/myContacts") {
        return;
    }
    let obj = data.as_object_mut().expect("person object");
    let arr = obj.entry("memberships").or_insert_with(|| json!([]));
    if let Some(a) = arr.as_array_mut() {
        a.push(json!({ "contactGroupMembership": { "contactGroupResourceName": "contactGroups/myContacts" } }));
    }
}

/// Fill in the server-computed parts (display names, formatted addresses, group ids, metadata).
fn normalize(data: &mut Value) {
    let Some(obj) = data.as_object_mut() else { return };
    // output-only fields a client may echo back
    for k in ["resourceName", "etag", "metadata", "coverPhotos", "ageRanges"] {
        obj.remove(k);
    }
    if let Some(names) = obj.get_mut("names").and_then(Value::as_array_mut) {
        for (i, n) in names.iter_mut().enumerate() {
            let (p, g, m, f, sfx) =
                (s(n, "honorificPrefix"), s(n, "givenName"), s(n, "middleName"), s(n, "familyName"), s(n, "honorificSuffix"));
            let mut display = join_nonempty(&[&p, &g, &m, &f, &sfx], " ");
            if display.is_empty() {
                display = s(n, "unstructuredName");
            }
            let first_part = join_nonempty(&[&p, &g, &m], " ");
            let last_first = if f.is_empty() {
                display.clone()
            } else if first_part.is_empty() {
                join_nonempty(&[&f, &sfx], ", ")
            } else {
                format!("{}, {}", join_nonempty(&[&f, &sfx], " "), first_part)
            };
            n["displayName"] = json!(display);
            n["displayNameLastFirst"] = json!(last_first);
            n["unstructuredName"] = json!(display);
            n["metadata"] = json!({ "primary": i == 0, "source": { "type": "CONTACT" } });
        }
    }
    if let Some(addrs) = obj.get_mut("addresses").and_then(Value::as_array_mut) {
        for a in addrs.iter_mut() {
            let line2 = join_nonempty(&[&join_nonempty(&[&s(a, "city"), &s(a, "region")], ", "), &s(a, "postalCode")], " ");
            a["formattedValue"] = json!(join_nonempty(&[&s(a, "streetAddress"), &s(a, "extendedAddress"), &line2, &s(a, "country")], "\n"));
            if let Some(t) = a.get("type").and_then(Value::as_str) {
                a["formattedType"] = json!(capitalize(t));
            }
        }
    }
    for k in ["phoneNumbers", "emailAddresses", "urls", "events"] {
        if let Some(arr) = obj.get_mut(k).and_then(Value::as_array_mut) {
            for (i, it) in arr.iter_mut().enumerate() {
                if let Some(t) = it.get("type").and_then(Value::as_str) {
                    it["formattedType"] = json!(capitalize(t));
                }
                if k == "phoneNumbers" {
                    let digits: String = s(it, "value").chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
                    it["canonicalForm"] = json!(digits);
                }
                if it.get("metadata").is_none() {
                    it["metadata"] = json!({ "primary": i == 0 });
                }
            }
        }
    }
    if let Some(ms) = obj.get_mut("memberships").and_then(Value::as_array_mut) {
        let mut seen = HashSet::new();
        ms.retain(|m| {
            m["contactGroupMembership"]["contactGroupResourceName"].as_str().map(|r| seen.insert(r.to_string())).unwrap_or(false)
        });
        for m in ms.iter_mut() {
            let rn = m["contactGroupMembership"]["contactGroupResourceName"].as_str().unwrap_or("").to_string();
            m["contactGroupMembership"]["contactGroupId"] = json!(rn.trim_start_matches("contactGroups/"));
        }
    }
}

fn capitalize(t: &str) -> String {
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Google's JSON for one person, restricted to `fields` (None = everything).
fn render(p: &FakePerson, fields: Option<&HashSet<String>>) -> Value {
    let source = json!({ "type": "CONTACT", "id": p.id.trim_start_matches('c'), "etag": p.etag, "updateTime": rfc3339(p.updated_ms) });
    if p.deleted {
        return json!({
            "resourceName": format!("people/{}", p.id),
            "etag": p.etag,
            "metadata": { "sources": [source], "deleted": true, "objectType": "PERSON" },
        });
    }
    let mut out = Map::new();
    out.insert("resourceName".into(), json!(format!("people/{}", p.id)));
    out.insert("etag".into(), json!(p.etag));
    if let Some(obj) = p.data.as_object() {
        for (k, v) in obj {
            if fields.map(|f| f.contains(k)).unwrap_or(true) {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    if fields.map(|f| f.contains("metadata")).unwrap_or(true) {
        out.insert("metadata".into(), json!({ "sources": [source], "objectType": "PERSON" }));
    }
    Value::Object(out)
}

fn parse_fields(raw: &Option<String>) -> Option<HashSet<String>> {
    raw.as_ref().map(|r| r.split(',').map(|f| f.trim().trim_start_matches("person.").to_string()).filter(|f| !f.is_empty()).collect())
}

// ---- routes -------------------------------------------------------------------------------

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/v1/people/me/connections", get(list_connections))
        .route("/v1/people:createContact", post(create_contact))
        .route("/v1/people/{rest}", get(person_op).patch(person_op).delete(person_op))
        .route("/v1/contactGroups", get(list_groups))
        .route("/v1/otherContacts", get(list_other))
        // not a Google endpoint: lets tests / demos expire every outstanding sync token
        .route("/v1/fake:expireSyncTokens", post(expire_sync_tokens))
}

async fn expire_sync_tokens(State(s): State<Shared>, User(email): User) -> Json<Value> {
    s.with(|st| st.user(&email).people.token_epoch += 1);
    Json(json!({}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    person_fields: Option<String>,
    read_mask: Option<String>,
    page_size: Option<usize>,
    page_token: Option<String>,
    sync_token: Option<String>,
    request_sync_token: Option<bool>,
    sort_order: Option<String>,
    sources: Option<String>,
}

/// Real Google: 400 FAILED_PRECONDITION with ErrorInfo reason EXPIRED_SYNC_TOKEN (not 410).
fn expired() -> Response {
    rpc_error(
        400,
        "FAILED_PRECONDITION",
        "Sync token is expired. Clear local cache and retry call without the sync token.",
        Some("EXPIRED_SYNC_TOKEN"),
    )
}

/// Shared by connections.list and otherContacts.list.
fn list_people(items: &[FakePerson], store: &Store, q: &ListQuery, fields: Option<&HashSet<String>>, key: &str) -> Response {
    if q.page_size.is_some_and(|n| n > 1000) {
        return bad_arg("pageSize must be between 1 and 1000.");
    }
    let size = q.page_size.unwrap_or(100).clamp(1, 1000);
    let params = format!(
        "{}|{}|{}",
        q.person_fields.as_deref().or(q.read_mask.as_deref()).unwrap_or(""),
        q.sort_order.as_deref().unwrap_or(""),
        q.sources.as_deref().unwrap_or("")
    );
    let (mut selected, incremental): (Vec<&FakePerson>, bool) = match &q.sync_token {
        Some(t) => match store.token_seq(t, &params) {
            Ok(since) => {
                let mut v: Vec<&FakePerson> = items.iter().filter(|p| p.seq > since).collect();
                v.sort_by_key(|p| p.seq);
                (v, true)
            }
            Err(e) => return e,
        },
        None => (items.iter().filter(|p| !p.deleted).collect(), false),
    };
    if !incremental {
        match q.sort_order.as_deref() {
            Some("LAST_MODIFIED_DESCENDING") => selected.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms)),
            Some("FIRST_NAME_ASCENDING") => selected.sort_by_key(|p| s(&p.data["names"][0], "givenName").to_lowercase()),
            Some("LAST_NAME_ASCENDING") => selected.sort_by_key(|p| s(&p.data["names"][0], "familyName").to_lowercase()),
            _ => selected.sort_by_key(|p| p.updated_ms),
        }
    }
    let total = selected.len();
    let (page, next) = paginate(&selected, &q.page_token, size);
    let mut out = Map::new();
    let rendered = page
        .iter()
        .map(|p| {
            let mut v = render(p, fields);
            if key == "otherContacts" {
                v["resourceName"] = json!(format!("otherContacts/{}", p.id));
            }
            v
        })
        .collect();
    out.insert(key.into(), Value::Array(rendered));
    if let Some(n) = next {
        out.insert("nextPageToken".into(), json!(n));
    } else if q.request_sync_token.unwrap_or(false) || q.sync_token.is_some() {
        out.insert("nextSyncToken".into(), json!(store.sync_token(&params)));
    }
    if key == "connections" {
        out.insert("totalPeople".into(), json!(total));
        out.insert("totalItems".into(), json!(total));
    } else {
        out.insert("totalSize".into(), json!(total));
    }
    Json(Value::Object(out)).into_response()
}

async fn list_connections(State(s): State<Shared>, User(email): User, Query(q): Query<ListQuery>) -> Response {
    let Some(fields) = parse_fields(&q.person_fields) else {
        return bad_arg("personFields mask is required. Please specify one or more valid paths.");
    };
    s.read(|st| {
        let store = &st.user(&email).people;
        list_people(&store.persons, store, &q, Some(&fields), "connections")
    })
}

async fn list_other(State(s): State<Shared>, User(email): User, Query(q): Query<ListQuery>) -> Response {
    let Some(fields) = parse_fields(&q.read_mask) else {
        return bad_arg("readMask is required. Please specify one or more valid paths.");
    };
    let allowed: HashSet<&str> = ["names", "emailAddresses", "phoneNumbers", "photos", "metadata"].into_iter().collect();
    if let Some(bad) = fields.iter().find(|f| !allowed.contains(f.as_str())) {
        return bad_arg(&format!("Invalid readMask path \"{bad}\"."));
    }
    s.read(|st| {
        let store = &st.user(&email).people;
        list_people(&store.other, store, &q, Some(&fields), "otherContacts")
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersonQuery {
    person_fields: Option<String>,
    update_person_fields: Option<String>,
}

async fn create_contact(State(s): State<Shared>, User(email): User, Query(q): Query<PersonQuery>, Json(mut body): Json<Value>) -> Response {
    if !body.is_object() {
        return bad_arg("Invalid person.");
    }
    let fields = parse_fields(&q.person_fields);
    if let Some(o) = body.as_object_mut() {
        o.remove("photos");
    }
    ensure_my_contacts(&mut body);
    normalize(&mut body);
    s.with(|st| {
        let n = st.next();
        let people = &mut st.user(&email).people;
        let id = people.new_id();
        let seq = people.bump();
        let p = FakePerson { id, etag: etag(n), seq, updated_ms: seed::now_ms(), deleted: false, data: body };
        let out = render(&p, fields.as_ref());
        people.persons.push(p);
        Json(out).into_response()
    })
}

/// GET people/{id}, PATCH people/{id}:updateContact | :updateContactPhoto, DELETE people/{id}:deleteContact
async fn person_op(
    State(s): State<Shared>,
    User(email): User,
    method: Method,
    Path(rest): Path<String>,
    Query(q): Query<PersonQuery>,
    body: Bytes,
) -> Response {
    let (id, action) = match rest.split_once(':') {
        Some((id, a)) => (id.to_string(), a.to_string()),
        None => (rest.clone(), String::new()),
    };
    match (method.as_str(), action.as_str()) {
        ("GET", "") => {
            let fields = parse_fields(&q.person_fields);
            if fields.is_none() {
                return bad_arg("personFields mask is required. Please specify one or more valid paths.");
            }
            s.read(|st| match st.user(&email).people.persons.iter().find(|p| p.id == id && !p.deleted) {
                Some(p) => Json(render(p, fields.as_ref())).into_response(),
                None => rpc_error(404, "NOT_FOUND", "Requested entity was not found.", None),
            })
        }
        ("PATCH", "updateContact") => {
            let Ok(body) = serde_json::from_slice::<Value>(&body) else { return bad_arg("Invalid JSON payload.") };
            let Some(update) = parse_fields(&q.update_person_fields) else {
                return bad_arg("updatePersonFields mask is required. Please specify one or more valid paths.");
            };
            let fields = parse_fields(&q.person_fields);
            let given_etag = body.get("etag").and_then(Value::as_str).map(str::to_string);
            s.with(|st| {
                let n = st.next();
                let people = &mut st.user(&email).people;
                let seq = people.seq + 1;
                let Some(p) = people.person_mut(&id) else { return rpc_error(404, "NOT_FOUND", "Requested entity was not found.", None) };
                match &given_etag {
                    None => return rpc_error(400, "FAILED_PRECONDITION", "Request must set person.etag or person.metadata.sources.etag so that the person is not changed since it was read.", None),
                    Some(e) if *e != p.etag => {
                        return rpc_error(400, "FAILED_PRECONDITION", "Request person.etag is different than the current person.etag. Clear local cache and get the latest person.", None);
                    }
                    _ => {}
                }
                let obj = p.data.as_object_mut().expect("person object");
                for f in &update {
                    match body.get(f) {
                        Some(v) if !v.is_null() => {
                            obj.insert(f.clone(), v.clone());
                        }
                        _ => {
                            obj.remove(f);
                        }
                    }
                }
                ensure_my_contacts(&mut p.data);
                normalize(&mut p.data);
                p.seq = seq;
                p.etag = etag(n);
                p.updated_ms = seed::now_ms();
                let out = render(p, fields.as_ref());
                people.seq = seq;
                Json(out).into_response()
            })
        }
        ("PATCH", "updateContactPhoto") => {
            use base64::Engine;
            let Ok(body) = serde_json::from_slice::<Value>(&body) else { return bad_arg("Invalid JSON payload.") };
            let Some(bytes) =
                body.get("photoBytes").and_then(Value::as_str).and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
            else {
                return bad_arg("photoBytes must be set.");
            };
            let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
                "image/png"
            } else if bytes.starts_with(&[0xff, 0xd8]) {
                "image/jpeg"
            } else if bytes.starts_with(b"GIF") {
                "image/gif"
            } else if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") {
                "image/svg+xml"
            } else {
                return bad_arg("Invalid photo bytes.");
            };
            let url = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
            let fields = body.get("personFields").and_then(Value::as_str).map(|f| f.to_string());
            let fields = parse_fields(&fields);
            s.with(|st| {
                let n = st.next();
                let people = &mut st.user(&email).people;
                let seq = people.seq + 1;
                let Some(p) = people.person_mut(&id) else { return rpc_error(404, "NOT_FOUND", "Requested entity was not found.", None) };
                p.data["photos"] = json!([{ "metadata": { "primary": true }, "url": url }]);
                p.seq = seq;
                p.etag = etag(n);
                p.updated_ms = seed::now_ms();
                let out = render(p, fields.as_ref());
                people.seq = seq;
                Json(json!({ "person": out })).into_response()
            })
        }
        ("DELETE", "deleteContact") => s.with(|st| {
            let people = &mut st.user(&email).people;
            let seq = people.seq + 1;
            let Some(p) = people.person_mut(&id) else { return rpc_error(404, "NOT_FOUND", "Requested entity was not found.", None) };
            p.deleted = true;
            p.seq = seq;
            p.updated_ms = seed::now_ms();
            people.seq = seq;
            Json(json!({})).into_response()
        }),
        _ => bad_arg(&format!("Unsupported operation {method} {rest}")),
    }
}

async fn list_groups(State(s): State<Shared>, User(email): User, Query(q): Query<ListQuery>) -> Response {
    s.read(|st| {
        let store = &st.user(&email).people;
        let groups: Vec<Value> = store
            .groups
            .iter()
            .map(|g| {
                let rn = format!("contactGroups/{}", g.id);
                json!({
                    "resourceName": rn,
                    "etag": g.etag,
                    "metadata": { "updateTime": rfc3339(g.updated_ms) },
                    "groupType": if g.system { "SYSTEM_CONTACT_GROUP" } else { "USER_CONTACT_GROUP" },
                    "name": g.name,
                    "formattedName": formatted_group_name(g),
                    "memberCount": store.member_count(&rn),
                })
            })
            .collect();
        let size = q.page_size.unwrap_or(30).clamp(1, 1000);
        let total = groups.len();
        let (page, next) = paginate(&groups, &q.page_token, size);
        let mut out = json!({ "contactGroups": page, "totalItems": total });
        match next {
            Some(n) => out["nextPageToken"] = json!(n),
            None => out["nextSyncToken"] = json!(store.sync_token("groups")),
        }
        Json(out).into_response()
    })
}

fn formatted_group_name(g: &FakeGroup) -> String {
    match g.id.as_str() {
        "myContacts" => "My Contacts".into(),
        "starred" => "Starred".into(),
        "friends" => "Friends".into(),
        "family" => "Family".into(),
        "coworkers" => "Coworkers".into(),
        _ => g.name.clone(),
    }
}

// ---- seed -----------------------------------------------------------------------------------

/// Extra demo people beyond `seed::PEOPLE` (investors, analysts, client executives, brokers...).
struct Extra {
    prefix: &'static str,
    first: &'static str,
    last: &'static str,
    company: &'static str,
    domain: &'static str,
    title: &'static str,
    dept: &'static str,
    phone: &'static str,
    street: &'static str,
    city: &'static str,
    region: &'static str,
    postal: &'static str,
    country: &'static str,
    /// explicit e-mail (Hebrew names); "" = first.last@domain
    email: &'static str,
    group: &'static str,
}

macro_rules! extra {
    ($prefix:expr, $first:expr, $last:expr, $company:expr, $domain:expr, $title:expr, $dept:expr, $phone:expr, $street:expr, $city:expr, $region:expr, $postal:expr, $country:expr, $email:expr, $group:expr) => {
        Extra {
            prefix: $prefix,
            first: $first,
            last: $last,
            company: $company,
            domain: $domain,
            title: $title,
            dept: $dept,
            phone: $phone,
            street: $street,
            city: $city,
            region: $region,
            postal: $postal,
            country: $country,
            email: $email,
            group: $group,
        }
    };
}

const EXTRA: &[Extra] = &[
    // buy-side investors
    extra!(
        "Mr.",
        "Jonathan",
        "Reeves",
        "Granite Peak Capital",
        "granitepeak.example",
        "Portfolio Manager",
        "Global Equities",
        "+1 212 555 0193",
        "425 Park Avenue, 18th Floor",
        "New York",
        "NY",
        "10022",
        "United States of America",
        "",
        "Investors"
    ),
    extra!(
        "Ms.",
        "Priya",
        "Raman",
        "Lakeshore Asset Management",
        "lakeshoream.example",
        "Senior Analyst, Healthcare",
        "Research",
        "+1 312 555 0127",
        "71 S Wacker Drive",
        "Chicago",
        "IL",
        "60606",
        "United States of America",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Mark",
        "Thompson",
        "Fidelis Global Investors",
        "fidelisgi.example",
        "Managing Partner",
        "",
        "+1 617 555 0144",
        "200 Clarendon Street",
        "Boston",
        "MA",
        "02116",
        "United States of America",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Hannah",
        "Schmidt",
        "Alster Invest",
        "alsterinvest.example",
        "Fondsmanagerin",
        "Aktien Europa",
        "+49 40 5550 3312",
        "Ballindamm 17",
        "Hamburg",
        "",
        "20095",
        "Germany",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Luca",
        "Bianchi",
        "Ponte Capital",
        "pontecap.example",
        "Equity Analyst",
        "",
        "+39 02 5550 7781",
        "Via Montenapoleone 8",
        "Milano",
        "MI",
        "20121",
        "Italy",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Kenji",
        "Watanabe",
        "Sakura Asset Partners",
        "sakura-ap.example",
        "Head of Global Equities",
        "",
        "+81 3 5550 2210",
        "1-6-1 Marunouchi, Chiyoda-ku",
        "Tokyo",
        "",
        "100-0005",
        "Japan",
        "",
        "Investors"
    ),
    extra!(
        "Ms.",
        "Olivia",
        "Grant",
        "Thames Ridge Investors",
        "thamesridge.example",
        "Investment Director",
        "",
        "+44 20 5550 4419",
        "30 St Mary Axe",
        "London",
        "",
        "EC3A 8BF",
        "United Kingdom",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Benjamin",
        "Stern",
        "Stern Family Office",
        "sternfo.example",
        "Principal",
        "",
        "+1 212 555 0170",
        "745 Fifth Avenue",
        "New York",
        "NY",
        "10151",
        "United States of America",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Gal",
        "Avraham",
        "Ophir Capital Markets",
        "ophircm.example",
        "Head of Research",
        "Research",
        "+972 3 555 0145",
        "Rothschild Blvd 22",
        "Tel Aviv",
        "",
        "6688218",
        "Israel",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Michal",
        "Bar-On",
        "Kinneret Pension Fund",
        "kinneretpf.example",
        "Equity Investment Manager",
        "Investments",
        "+972 3 555 0161",
        "Menachem Begin Rd 132",
        "Tel Aviv",
        "",
        "6701101",
        "Israel",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Eitan",
        "Shalev",
        "Arbel Partners",
        "arbelpartners.example",
        "Partner",
        "",
        "+972 9 555 0132",
        "Abba Eban Blvd 10",
        "Herzliya",
        "",
        "4672530",
        "Israel",
        "",
        "Investors"
    ),
    extra!(
        "Mme",
        "Natalie",
        "Dubois",
        "Lumière Capital",
        "lumierecap.example",
        "Gérante de portefeuille",
        "",
        "+33 1 5550 6620",
        "12 Rue de la Paix",
        "Paris",
        "",
        "75002",
        "France",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Samuel",
        "Okafor",
        "Delta Frontier Fund",
        "deltafrontier.example",
        "Analyst",
        "Emerging Markets",
        "+44 20 5550 7734",
        "1 Canada Square",
        "London",
        "",
        "E14 5AB",
        "United Kingdom",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Grace",
        "Liu",
        "Pacific Crest Investments",
        "paccrest.example",
        "VP, Technology Investments",
        "",
        "+1 415 555 0188",
        "555 California Street",
        "San Francisco",
        "CA",
        "94104",
        "United States of America",
        "",
        "Investors"
    ),
    extra!(
        "",
        "William",
        "Foster",
        "Foster & Lane Capital",
        "fosterlane.example",
        "Chief Investment Officer",
        "",
        "+1 203 555 0119",
        "2 Greenwich Office Park",
        "Greenwich",
        "CT",
        "06831",
        "United States of America",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Isabel",
        "Martínez",
        "Iberia Growth Fund",
        "iberiagrowth.example",
        "Portfolio Manager",
        "",
        "+34 91 555 0177",
        "Paseo de la Castellana 95",
        "Madrid",
        "",
        "28046",
        "Spain",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Erik",
        "Johansson",
        "Nordic Value Partners",
        "nordicvalue.example",
        "Senior Portfolio Manager",
        "",
        "+46 8 555 0123",
        "Stureplan 4C",
        "Stockholm",
        "",
        "114 35",
        "Sweden",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Ruth",
        "Abramson",
        "Abramson Asset Management",
        "abramsonam.example",
        "Managing Director",
        "",
        "+1 416 555 0135",
        "100 King Street West",
        "Toronto",
        "ON",
        "M5X 1A9",
        "Canada",
        "",
        "Investors"
    ),
    extra!(
        "",
        "Ahmed",
        "Haddad",
        "Gulf Horizon Capital",
        "gulfhorizon.example",
        "Investment Manager",
        "",
        "+971 4 555 0142",
        "Gate Village 3, DIFC",
        "Dubai",
        "",
        "",
        "United Arab Emirates",
        "",
        "Investors"
    ),
    // sell-side analysts
    extra!(
        "Dr.",
        "Alan",
        "Weiss",
        "Kingsbridge Securities",
        "kingsbridgesec.example",
        "Senior Equity Research Analyst, MedTech",
        "Equity Research",
        "+1 212 555 0156",
        "1 Liberty Plaza",
        "New York",
        "NY",
        "10006",
        "United States of America",
        "",
        ""
    ),
    extra!(
        "",
        "Shira",
        "Navon",
        "Yarden Securities",
        "yardensec.example",
        "Research Analyst",
        "Research",
        "+972 3 555 0178",
        "Ahad Ha'Am St 9",
        "Tel Aviv",
        "",
        "6525101",
        "Israel",
        "",
        ""
    ),
    extra!(
        "",
        "Patrick",
        "O'Neill",
        "Oak Hill Research",
        "oakhillres.example",
        "Analyst, Semiconductors",
        "",
        "+1 617 555 0109",
        "100 Federal Street",
        "Boston",
        "MA",
        "02110",
        "United States of America",
        "",
        ""
    ),
    extra!(
        "",
        "Ingrid",
        "Larsen",
        "Fjord Equity Research",
        "fjordresearch.example",
        "Head of Research",
        "",
        "+47 22 55 50 12",
        "Dronning Mauds gate 15",
        "Oslo",
        "",
        "0250",
        "Norway",
        "",
        ""
    ),
    extra!(
        "",
        "Arjun",
        "Mehta",
        "Crestline Partners",
        "crestline.example",
        "Associate Analyst",
        "Equity Research",
        "+1 646 555 0166",
        "11 Madison Avenue",
        "New York",
        "NY",
        "10010",
        "United States of America",
        "",
        ""
    ),
    extra!(
        "",
        "Claire",
        "Beaumont",
        "Beaumont Research",
        "beaumontres.example",
        "Independent Analyst",
        "",
        "+44 20 5550 2290",
        "8 Bishopsgate",
        "London",
        "",
        "EC2N 4BQ",
        "United Kingdom",
        "",
        ""
    ),
    extra!(
        "",
        "Yossi",
        "Dahan",
        "Birchwood Research",
        "birchwood-research.example",
        "Analyst, Israeli Tech",
        "",
        "+972 3 555 0187",
        "Ha'Arba'a St 28",
        "Tel Aviv",
        "",
        "6473925",
        "Israel",
        "",
        ""
    ),
    // client executives
    extra!(
        "",
        "Amit",
        "Golan",
        "Orion Technologies",
        "oriontech.example",
        "Chief Executive Officer",
        "Management",
        "+972 9 555 0101",
        "Hamenofim St 2",
        "Herzliya",
        "",
        "4672553",
        "Israel",
        "",
        "Clients"
    ),
    extra!(
        "",
        "Dana",
        "Klein",
        "Sunfield Energy",
        "sunfield.example",
        "Chief Financial Officer",
        "Finance",
        "+972 8 555 0120",
        "Desert Gate Park, Building 3",
        "Be'er Sheva",
        "",
        "8489325",
        "Israel",
        "",
        "Clients"
    ),
    extra!(
        "",
        "Ronen",
        "Avital",
        "Sunfield Energy",
        "sunfield.example",
        "Chief Executive Officer",
        "Management",
        "+972 8 555 0121",
        "Desert Gate Park, Building 3",
        "Be'er Sheva",
        "",
        "8489325",
        "Israel",
        "",
        "Clients"
    ),
    extra!(
        "Adv.",
        "Efrat",
        "Sasson",
        "Corvina Medical",
        "corvina.example",
        "General Counsel",
        "Legal",
        "+972 4 555 0168",
        "Hamada St 7",
        "Haifa",
        "",
        "2069206",
        "Israel",
        "",
        "Clients"
    ),
    extra!(
        "",
        "Ido",
        "Barak",
        "Silverline Semiconductors",
        "silverlinesemi.example",
        "Chief Financial Officer",
        "Finance",
        "+972 4 555 0150",
        "Matam Park, Building 22",
        "Haifa",
        "",
        "3508409",
        "Israel",
        "",
        "Clients"
    ),
    extra!(
        "",
        "Liat",
        "Hadad",
        "Silverline Semiconductors",
        "silverlinesemi.example",
        "VP Investor Relations",
        "Investor Relations",
        "+972 4 555 0151",
        "Matam Park, Building 22",
        "Haifa",
        "",
        "3508409",
        "Israel",
        "",
        "Clients"
    ),
    extra!(
        "",
        "Guy",
        "Navon",
        "Aquifer Water Tech",
        "aquiferwt.example",
        "Chief Executive Officer",
        "",
        "+972 9 555 0175",
        "Ha'Taasia St 12",
        "Kfar Saba",
        "",
        "4442112",
        "Israel",
        "",
        "Clients"
    ),
    extra!(
        "",
        "Merav",
        "Tal",
        "Aquifer Water Tech",
        "aquiferwt.example",
        "Corporate Controller",
        "Finance",
        "+972 9 555 0176",
        "Ha'Taasia St 12",
        "Kfar Saba",
        "",
        "4442112",
        "Israel",
        "",
        "Clients"
    ),
    extra!(
        "",
        "Assaf",
        "Zur",
        "CyberDome Security",
        "cyberdome.example",
        "Chairman of the Board",
        "",
        "+972 3 555 0133",
        "Yigal Alon St 98",
        "Tel Aviv",
        "",
        "6789141",
        "Israel",
        "",
        "Clients"
    ),
    extra!(
        "",
        "Keren",
        "Elbaz",
        "CyberDome Security",
        "cyberdome.example",
        "Chief Financial Officer",
        "Finance",
        "+972 3 555 0134",
        "Yigal Alon St 98",
        "Tel Aviv",
        "",
        "6789141",
        "Israel",
        "",
        "Clients"
    ),
    extra!(
        "",
        "Uri",
        "Peled",
        "Golan Foods",
        "golanfoods.example",
        "Chief Executive Officer",
        "",
        "+972 4 555 0190",
        "Industrial Zone",
        "Katzrin",
        "",
        "1290000",
        "Israel",
        "",
        "Clients"
    ),
    // brokers & bankers
    extra!(
        "",
        "Steven",
        "Marsh",
        "Liberty Street Securities",
        "libertyst.example",
        "Director, Corporate Access",
        "Corporate Access",
        "+1 212 555 0182",
        "60 Wall Street",
        "New York",
        "NY",
        "10005",
        "United States of America",
        "",
        "Brokers"
    ),
    extra!(
        "",
        "Fiona",
        "Campbell",
        "Kingsbridge Securities",
        "kingsbridgesec.example",
        "Head of Corporate Access, EMEA",
        "Corporate Access",
        "+44 20 5550 6611",
        "25 Bank Street",
        "London",
        "",
        "E14 5JP",
        "United Kingdom",
        "",
        "Brokers"
    ),
    extra!(
        "",
        "Matthew",
        "Klein",
        "Summit Securities",
        "summitsec.example",
        "Managing Director, ECM",
        "Equity Capital Markets",
        "+1 646 555 0131",
        "277 Park Avenue",
        "New York",
        "NY",
        "10172",
        "United States of America",
        "",
        "Brokers"
    ),
    extra!(
        "",
        "Nir",
        "Segal",
        "Tavor Brokerage",
        "tavorbrokers.example",
        "Senior Broker",
        "Institutional Desk",
        "+972 3 555 0158",
        "Ha'Barzel St 30",
        "Tel Aviv",
        "",
        "6971048",
        "Israel",
        "",
        "Brokers"
    ),
    extra!(
        "",
        "Hila",
        "Shamir",
        "Tavor Brokerage",
        "tavorbrokers.example",
        "Institutional Sales",
        "Institutional Desk",
        "+972 3 555 0159",
        "Ha'Barzel St 30",
        "Tel Aviv",
        "",
        "6971048",
        "Israel",
        "",
        "Brokers"
    ),
    extra!(
        "",
        "Daniel",
        "Ortiz",
        "Bay Capital Markets",
        "baycm.example",
        "Sales Trader",
        "",
        "+1 415 555 0162",
        "101 Montgomery Street",
        "San Francisco",
        "CA",
        "94104",
        "United States of America",
        "",
        "Brokers"
    ),
    extra!(
        "",
        "Adam",
        "Brooks",
        "Liberty Street Securities",
        "libertyst.example",
        "Equity Sales",
        "",
        "+1 212 555 0183",
        "60 Wall Street",
        "New York",
        "NY",
        "10005",
        "United States of America",
        "",
        "Brokers"
    ),
    // service providers and others
    extra!(
        "",
        "Laura",
        "Bennett",
        "Atlas Conferences",
        "atlasconf.example",
        "Conference Director",
        "",
        "+1 305 555 0153",
        "1395 Brickell Avenue",
        "Miami",
        "FL",
        "33131",
        "United States of America",
        "",
        ""
    ),
    extra!(
        "",
        "Ruti",
        "Gabay",
        "Business Daily IL",
        "bizdaily.example",
        "Capital Markets Reporter",
        "Newsroom",
        "+972 3 555 0138",
        "Ha'Shfela St 4",
        "Tel Aviv",
        "",
        "6603804",
        "Israel",
        "",
        ""
    ),
    extra!(
        "Adv.",
        "Michael",
        "Rosenthal",
        "Rosenthal & Co. Law Offices",
        "rosenthal-law.example",
        "Senior Partner",
        "Securities",
        "+972 3 555 0141",
        "Azrieli Center, Round Tower",
        "Tel Aviv",
        "",
        "6701101",
        "Israel",
        "",
        ""
    ),
    extra!(
        "",
        "Tali",
        "Weinberg",
        "Proxy Advisory Israel",
        "proxyadvisory.example",
        "Governance Analyst",
        "",
        "+972 3 555 0149",
        "Ha'Rakevet St 58",
        "Tel Aviv",
        "",
        "6777016",
        "Israel",
        "",
        ""
    ),
    extra!(
        "",
        "Nicole",
        "Harper",
        "Harper Translation Services",
        "harpertrans.example",
        "Project Manager",
        "",
        "+44 20 5550 3381",
        "14 Old Street",
        "London",
        "",
        "EC1V 9BH",
        "United Kingdom",
        "",
        ""
    ),
    extra!(
        "",
        "Ofer",
        "Lavi",
        "Lavi & Partners CPA",
        "lavicpa.example",
        "Audit Partner",
        "Audit",
        "+972 3 555 0172",
        "Derech Menachem Begin 144",
        "Tel Aviv",
        "",
        "6492102",
        "Israel",
        "",
        ""
    ),
    extra!(
        "",
        "Jennifer",
        "Walsh",
        "Market Listing Partners",
        "mlp.example",
        "Relationship Manager, Israel",
        "",
        "+1 212 555 0114",
        "165 Broadway",
        "New York",
        "NY",
        "10006",
        "United States of America",
        "",
        ""
    ),
    extra!(
        "",
        "Paul",
        "Schneider",
        "IR Webcast Pro",
        "irwebcast.example",
        "Account Manager",
        "",
        "+49 69 5550 4471",
        "Neue Mainzer Str. 52",
        "Frankfurt am Main",
        "",
        "60311",
        "Germany",
        "",
        ""
    ),
    extra!(
        "",
        "Yuval",
        "Ben-Ami",
        "Juniper Ventures",
        "junipervc.example",
        "Associate",
        "",
        "+972 54 555 0198",
        "Habarzel St 32",
        "Tel Aviv",
        "",
        "6971054",
        "Israel",
        "",
        ""
    ),
    extra!(
        "",
        "Tom",
        "Harris",
        "Harris Event Photography",
        "harrisphoto.example",
        "Photographer",
        "",
        "+972 52 555 0128",
        "",
        "Ramat Gan",
        "",
        "",
        "Israel",
        "",
        ""
    ),
    // Hebrew names
    extra!(
        "",
        "משה",
        "כהן",
        "אלפא השקעות",
        "alpha-inv.example",
        "מנהל השקעות",
        "השקעות",
        "+972 3 555 0111",
        "שדרות רוטשילד 45",
        "תל אביב",
        "",
        "6578403",
        "ישראל",
        "moshe.cohen@alpha-inv.example",
        "Investors"
    ),
    extra!(
        "",
        "רונית",
        "לוי",
        "בית השקעות גליל",
        "galil-ih.example",
        "אנליסטית בכירה",
        "מחקר",
        "+972 4 555 0115",
        "שדרות הנשיא 120",
        "חיפה",
        "",
        "3463301",
        "ישראל",
        "ronit.levi@galil-ih.example",
        ""
    ),
    extra!(
        "",
        "אבי",
        "ברק",
        "קרן ירדן",
        "yarden-fund.example",
        "שותף מנהל",
        "",
        "+972 2 555 0119",
        "רחוב יפו 97",
        "ירושלים",
        "",
        "9434206",
        "ישראל",
        "avi@yarden-fund.example",
        "Investors"
    ),
    extra!(
        "",
        "שירה",
        "אלון",
        "אורט טכנולוגיות",
        "orettech.example",
        "סמנכ\"לית כספים",
        "כספים",
        "+972 9 555 0126",
        "רחוב המדע 8",
        "הרצליה",
        "",
        "4672509",
        "ישראל",
        "shira.alon@orettech.example",
        "Clients"
    ),
    extra!(
        "",
        "יוסי",
        "מזרחי",
        "ברוקרים בע\"מ",
        "brokers-il.example",
        "ברוקר",
        "מסחר",
        "+972 3 555 0139",
        "רחוב הארבעה 21",
        "תל אביב",
        "",
        "6473921",
        "ישראל",
        "yossi@brokers-il.example",
        "Brokers"
    ),
    extra!(
        "עו\"ד",
        "מיכל",
        "דוד",
        "דוד ושות'",
        "david-law.example",
        "שותפה",
        "",
        "+972 3 555 0147",
        "דרך מנחם בגין 7",
        "רמת גן",
        "",
        "5268102",
        "ישראל",
        "michal@david-law.example",
        ""
    ),
];

fn group_for_company(company: &str) -> &'static str {
    match company {
        "Harbor Capital"
        | "Meridian Partners"
        | "Northgate Advisors"
        | "Tamar Investments"
        | "Bluewater Fund"
        | "Rhein Asset Management"
        | "Juniper Ventures"
        | "Vistula Capital"
        | "Seine Gestion"
        | "Jerusalem Equity" => "Investors",
        "Orion Technologies" | "Corvina Medical" | "Sunfield Energy" => "Clients",
        "Summit Securities" => "Brokers",
        _ => "",
    }
}

/// City / country for the shared demo people, from their phone prefix.
fn place_for_phone(phone: &str, rng: &mut Rng) -> (&'static str, &'static str, &'static str, &'static str, &'static str) {
    // (street, city, region, postal, country)
    if phone.starts_with("+972 3") {
        *rng.pick(&[
            ("Rothschild Blvd 3", "Tel Aviv", "", "6688101", "Israel"),
            ("Menachem Begin Rd 125", "Tel Aviv", "", "6701201", "Israel"),
        ])
    } else if phone.starts_with("+972 9") {
        ("Hamenofim St 10", "Herzliya", "", "4672561", "Israel")
    } else if phone.starts_with("+972 4") {
        ("Ha'Atzmaut Rd 40", "Haifa", "", "3303320", "Israel")
    } else if phone.starts_with("+972 8") {
        ("Desert Gate Park, Building 1", "Be'er Sheva", "", "8489301", "Israel")
    } else if phone.starts_with("+972 2") {
        ("Jaffa Rd 33", "Jerusalem", "", "9414201", "Israel")
    } else if phone.starts_with("+972") {
        ("Ahad Ha'Am St 1", "Tel Aviv", "", "6525101", "Israel")
    } else if phone.starts_with("+1 212") || phone.starts_with("+1 646") {
        ("350 Park Avenue", "New York", "NY", "10022", "United States of America")
    } else if phone.starts_with("+1 617") {
        ("1 Lincoln Street", "Boston", "MA", "02111", "United States of America")
    } else if phone.starts_with("+1 312") {
        ("10 S Dearborn Street", "Chicago", "IL", "60603", "United States of America")
    } else if phone.starts_with("+1 305") {
        ("801 Brickell Avenue", "Miami", "FL", "33131", "United States of America")
    } else if phone.starts_with("+44") {
        ("25 Old Broad Street", "London", "", "EC2N 1HN", "United Kingdom")
    } else if phone.starts_with("+49") {
        ("Taunusanlage 12", "Frankfurt am Main", "", "60325", "Germany")
    } else if phone.starts_with("+48") {
        ("ul. Emilii Plater 53", "Warszawa", "", "00-113", "Poland")
    } else if phone.starts_with("+33") {
        ("8 Place Vendôme", "Paris", "", "75001", "France")
    } else {
        ("", "", "", "", "")
    }
}

const NOTES: &[&str] = &[
    "Met at the TASE Tel Aviv Investor Conference. Interested in Israeli medtech and cleantech names.",
    "Prefers calls in the morning New York time. Assistant handles scheduling.",
    "Long-only, typically 2–4% positions. Asked for the updated investor presentation after Q2 results.",
    "Covers Israeli tech on the sell side. Send earnings release under embargo 30 minutes before publication.",
    "Attended the London roadshow in March. Follow up on the capital markets day invite.",
    "Hebrew speaker. Happy to meet in Tel Aviv when visiting.",
    "Key contact for the annual report translation project.",
    "Do not send research reports – compliance restriction.",
    "Met at the Atlas Healthcare Conference in Miami. Wants 1x1 with management next time.",
    "נפגשנו בכנס המשקיעים בתל אביב. מעוניין בפגישה עם ההנהלה.",
];

const MOBILE_PREFIX: &[&str] = &["+972 50 555 0", "+972 52 555 0", "+972 54 555 0", "+1 917 555 0", "+44 7700 900"];

/// An initials avatar as a data: URI (stands in for a photo on lh3.googleusercontent.com).
fn avatar(first: &str, last: &str, rng: &mut Rng) -> String {
    use base64::Engine;
    let colors = ["#4A7EBB", "#C0504D", "#9BBB59", "#8064A2", "#F79646", "#4BACC6", "#7F6084", "#2C4D75"];
    let bg = rng.pick(&colors);
    let initials: String = [first, last].iter().filter_map(|w| w.chars().next()).collect();
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='96' height='96'><rect width='96' height='96' fill='{bg}'/>\
         <circle cx='48' cy='38' r='18' fill='#ffffff' opacity='0.25'/><path d='M14 96c4-22 18-32 34-32s30 10 34 32z' fill='#ffffff' opacity='0.25'/>\
         <text x='48' y='60' font-family='Segoe UI, Arial' font-size='34' font-weight='600' fill='#ffffff' text-anchor='middle'>{initials}</text></svg>"
    );
    format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg))
}

#[allow(clippy::too_many_arguments)]
fn build_person(
    rng: &mut Rng,
    prefix: &str,
    first: &str,
    last: &str,
    company: &str,
    title: &str,
    dept: &str,
    email: &str,
    phone: &str,
    place: (&str, &str, &str, &str, &str),
    group: &str,
    hebrew: bool,
) -> Value {
    let mut p = Map::new();
    let mut name = json!({ "givenName": first, "familyName": last });
    if !prefix.is_empty() {
        name["honorificPrefix"] = json!(prefix);
    }
    if !hebrew && rng.chance(8) {
        name["middleName"] = json!(format!("{}.", (b'A' + rng.below(26) as u8) as char));
    }
    p.insert("names".into(), json!([name]));
    let nick = match first {
        "Jonathan" => "Jon",
        "William" => "Bill",
        "Benjamin" => "Ben",
        "Daniel" => "Dani",
        "Michael" => "Mike",
        "Robert" => "Bob",
        "Thomas" => "Tom",
        "Samuel" => "Sam",
        "Matthew" => "Matt",
        "Patrick" => "Pat",
        "Steven" => "Steve",
        "Christopher" | "Chris" => "Chris",
        "Yossi" => "Yos",
        "Avi" => "Avi",
        _ => "",
    };
    if !nick.is_empty() && nick != first && rng.chance(60) {
        p.insert("nicknames".into(), json!([{ "value": nick }]));
    }
    let mut emails = vec![json!({ "value": email, "type": "work" })];
    let ascii = |s: &str| s.to_lowercase().chars().filter(|c| c.is_ascii_alphabetic()).collect::<String>();
    if !hebrew && rng.chance(25) {
        let personal = format!("{}.{}{}@mail.example", ascii(first), ascii(last), rng.below(90) + 10);
        emails.push(json!({ "value": personal, "type": "home" }));
    }
    p.insert("emailAddresses".into(), Value::Array(emails));
    let mut phones = vec![json!({ "value": phone, "type": "work" })];
    if rng.chance(60) {
        phones.push(json!({ "value": format!("{}{:03}", rng.pick(MOBILE_PREFIX), rng.below(1000)), "type": "mobile" }));
    }
    if rng.chance(18) {
        phones.push(json!({ "value": format!("{}{}", &phone[..phone.len().saturating_sub(2)], rng.below(90) + 10), "type": "workFax" }));
    }
    if rng.chance(15) {
        phones.push(json!({ "value": format!("{}{:04}", &phone[..phone.len().saturating_sub(4)], rng.below(10000)), "type": "home" }));
    }
    p.insert("phoneNumbers".into(), Value::Array(phones));
    let (street, city, region, postal, country) = place;
    let mut addrs = Vec::new();
    if !city.is_empty() && rng.chance(85) {
        addrs.push(
            json!({ "type": "work", "streetAddress": street, "city": city, "region": region, "postalCode": postal, "country": country }),
        );
    }
    if !city.is_empty() && rng.chance(12) {
        addrs.push(json!({ "type": "home", "streetAddress": format!("{} {}", rng.below(80) + 1, if hebrew { "רחוב הזית" } else { "Oak Lane" }), "city": city, "region": region, "postalCode": postal, "country": country }));
    }
    if !addrs.is_empty() {
        p.insert("addresses".into(), Value::Array(addrs));
    }
    if !company.is_empty() {
        let mut org = json!({ "name": company, "title": title, "type": "work" });
        if !dept.is_empty() {
            org["department"] = json!(dept);
        }
        p.insert("organizations".into(), json!([org]));
    }
    let domain = email.split('@').nth(1).unwrap_or("");
    if rng.chance(40) && !domain.is_empty() {
        p.insert("urls".into(), json!([{ "value": format!("https://www.{domain}"), "type": "work" }]));
    }
    if rng.chance(30) {
        p.insert(
            "birthdays".into(),
            json!([{ "date": { "year": 1960 + rng.below(35) as i32, "month": rng.below(12) + 1, "day": rng.below(28) + 1 } }]),
        );
    }
    if rng.chance(6) {
        p.insert("events".into(), json!([{ "type": "anniversary", "date": { "year": 1995 + rng.below(25) as i32, "month": rng.below(12) + 1, "day": rng.below(28) + 1 } }]));
    }
    if rng.chance(30) {
        let note = if hebrew { NOTES[NOTES.len() - 1] } else { NOTES[rng.below(NOTES.len() as u64 - 1) as usize] };
        p.insert("biographies".into(), json!([{ "value": note, "contentType": "TEXT_PLAIN" }]));
    }
    if rng.chance(55) {
        p.insert("photos".into(), json!([{ "metadata": { "primary": true }, "url": avatar(first, last, rng) }]));
    } else {
        p.insert(
            "photos".into(),
            json!([{ "metadata": { "primary": true }, "url": "https://lh3.googleusercontent.com/cm/default-user=s100", "default": true }]),
        );
    }
    let mut memberships = vec![json!({ "contactGroupMembership": { "contactGroupResourceName": "contactGroups/myContacts" } })];
    if !group.is_empty() {
        memberships.push(json!({ "contactGroupMembership": { "contactGroupResourceName": format!("contactGroups/{}", group_id(group)) } }));
    }
    if rng.chance(10) {
        memberships.push(json!({ "contactGroupMembership": { "contactGroupResourceName": "contactGroups/starred" } }));
    }
    p.insert("memberships".into(), Value::Array(memberships));
    let mut v = Value::Object(p);
    normalize(&mut v);
    v
}

fn group_id(name: &str) -> &'static str {
    match name {
        "Investors" => "3c1a5e0b8d2f4a11",
        "Clients" => "5f9b2c7d1e3a4b22",
        "Brokers" => "7a4d8e2f6b1c3d33",
        _ => "0",
    }
}

const OTHER_FIRST: &[&str] =
    &["Megan", "Josh", "Tal", "Noam", "Ella", "Ryan", "Sivan", "Laura", "Omer", "Kate", "Lior", "Ben", "Hadas", "Chris", "Anat", "Eli"];
const OTHER_LAST: &[&str] =
    &["Price", "Miller", "Azulay", "Friedman", "Stone", "Harel", "Young", "Vaknin", "Ross", "Biton", "Hart", "Golan"];
const OTHER_DOMAINS: &[&str] = &[
    "harborcap.example",
    "summitsec.example",
    "atlasconf.example",
    "kingsbridgesec.example",
    "oriontech.example",
    "corvina.example",
    "bizdaily.example",
    "tavorbrokers.example",
    "irwebcast.example",
    "mail.example",
];
const OTHER_NAMELESS: &[&str] = &[
    "registration@atlasconf.example",
    "ir@oriontech.example",
    "events@kingsbridgesec.example",
    "assistant.office@harborcap.example",
    "newsdesk@bizdaily.example",
    "support@irwebcast.example",
];

pub fn seed(email: &str, _name: &str) -> Store {
    let mut rng = Rng::for_key(&format!("people:{email}"));
    let now = seed::now_ms();
    let mut st = Store::default();
    let groups = [
        ("myContacts", "myContacts", true),
        ("starred", "starred", true),
        ("friends", "friends", true),
        ("family", "family", true),
        ("coworkers", "coworkers", true),
        (group_id("Investors"), "Investors", false),
        (group_id("Clients"), "Clients", false),
        (group_id("Brokers"), "Brokers", false),
    ];
    for (i, (id, name, system)) in groups.iter().enumerate() {
        st.groups.push(FakeGroup {
            id: id.to_string(),
            name: name.to_string(),
            system: *system,
            etag: etag(i as u64 + 1),
            updated_ms: now - 400 * DAY_MS,
        });
    }
    let add = |st: &mut Store, data: Value, rng: &mut Rng| {
        let id = st.new_id();
        let seq = st.bump();
        let updated = now - rng.below(700) as i64 * DAY_MS - rng.below(DAY_MS as u64) as i64;
        st.persons.push(FakePerson { id, etag: etag(seq * 31 + 7), seq, updated_ms: updated, deleted: false, data });
    };
    let mine = email.split('@').next().unwrap_or("");
    for person in PEOPLE {
        if person.domain.starts_with("duvz") && mine == person.first.to_lowercase() {
            continue;
        }
        let place = place_for_phone(person.phone, &mut rng);
        let data = build_person(
            &mut rng,
            "",
            person.first,
            person.last,
            person.company,
            person.title,
            "",
            &person.email(),
            person.phone,
            place,
            group_for_company(person.company),
            false,
        );
        add(&mut st, data, &mut rng);
    }
    for x in EXTRA {
        let hebrew = !x.email.is_empty();
        let email = if hebrew {
            x.email.to_string()
        } else {
            let clean = |s: &str| s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>();
            format!("{}.{}@{}", clean(x.first), clean(x.last), x.domain)
        };
        let data = build_person(
            &mut rng,
            x.prefix,
            x.first,
            x.last,
            x.company,
            x.title,
            x.dept,
            &email,
            x.phone,
            (x.street, x.city, x.region, x.postal, x.country),
            x.group,
            hebrew,
        );
        add(&mut st, data, &mut rng);
    }
    // a couple of company-only entries, like a switchboard or a venue
    for (company, phone, mail) in [
        ("Atlas Conferences – Registration Desk", "+1 305 555 0100", "registration@atlasconf.example"),
        ("David InterContinental Tel Aviv – Events", "+972 3 555 0400", "events@intercon-tlv.example"),
    ] {
        let mut data = json!({
            "organizations": [{ "name": company, "type": "work" }],
            "emailAddresses": [{ "value": mail, "type": "work" }],
            "phoneNumbers": [{ "value": phone, "type": "main" }],
            "memberships": [{ "contactGroupMembership": { "contactGroupResourceName": "contactGroups/myContacts" } }],
        });
        normalize(&mut data);
        add(&mut st, data, &mut rng);
    }
    // ~40 "other contacts": people Kenny e-mailed who were never saved as contacts
    let mut seen = HashSet::new();
    let mut others: BTreeMap<String, Value> = BTreeMap::new();
    while others.len() < 34 {
        let first = *rng.pick(OTHER_FIRST);
        let last = *rng.pick(OTHER_LAST);
        let domain = *rng.pick(OTHER_DOMAINS);
        let addr = format!("{}.{}@{}", first.to_lowercase(), last.to_lowercase(), domain);
        if !seen.insert(addr.clone()) {
            continue;
        }
        let mut v =
            json!({ "names": [{ "givenName": first, "familyName": last }], "emailAddresses": [{ "value": addr, "type": "other" }] });
        normalize(&mut v);
        others.insert(addr, v);
    }
    for addr in OTHER_NAMELESS {
        others.insert(addr.to_string(), json!({ "emailAddresses": [{ "value": addr }] }));
    }
    for (_, data) in others {
        let n = st.next_id + 1;
        st.next_id = n;
        let seq = st.bump();
        st.other.push(FakePerson {
            id: format!("c{}", 9_100_000_000_000_000_000u64 + n * 104729),
            etag: etag(seq),
            seq,
            updated_ms: now - rng.below(300) as i64 * DAY_MS,
            deleted: false,
            data,
        });
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_deterministic_and_plausible() {
        let a = seed("kenny@duvz.demo", "Kenny Green");
        let b = seed("kenny@duvz.demo", "Kenny Green");
        assert_eq!(a.persons.len(), b.persons.len());
        assert!(a.persons.len() >= 75, "{} contacts", a.persons.len());
        assert!(a.other.len() >= 38);
        assert_eq!(serde_json::to_string(&a.persons[5].data).unwrap(), serde_json::to_string(&b.persons[5].data).unwrap());
        // Hebrew names, groups and photos are present
        assert!(a.persons.iter().any(|p| s(&p.data["names"][0], "displayName") == "משה כהן"));
        assert!(a.persons.iter().any(|p| memberships_of(&p.data).contains(&"contactGroups/7a4d8e2f6b1c3d33".to_string())));
        assert!(a.persons.iter().any(|p| p.data["photos"][0]["default"].as_bool() != Some(true)));
    }

    #[test]
    fn normalize_computes_names() {
        let mut v = json!({ "names": [{ "honorificPrefix": "Dr.", "givenName": "Alan", "familyName": "Weiss" }] });
        normalize(&mut v);
        assert_eq!(v["names"][0]["displayName"], "Dr. Alan Weiss");
        assert_eq!(v["names"][0]["displayNameLastFirst"], "Weiss, Dr. Alan");
    }
}
