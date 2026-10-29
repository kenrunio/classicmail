//! Integration tests: sync round trips against the in-process fake People API.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{model::Contact, sync};
use crate::{
    accounts,
    ctx::AppCtx,
    google::{Api, people},
    rpc,
};

struct T {
    ctx: Arc<AppCtx>,
    api: Api,
    account_id: i64,
    reg: rpc::Registry,
    _guard: crate::db::tempdir::TempDirGuard,
}

impl T {
    async fn new(email: &str) -> T {
        let (ctx, guard) = AppCtx::for_test().unwrap();
        let acct = accounts::upsert(&ctx.db, email, "Test", "demo").await.unwrap();
        let api = ctx.api(acct.id).await.unwrap();
        T { ctx, api, account_id: acct.id, reg: rpc::build(), _guard: guard }
    }
    async fn call(&self, method: &str, params: Value) -> Value {
        self.reg.call(self.ctx.clone(), method, params).await.unwrap_or_else(|e| panic!("{method}: {e:#}"))
    }
    async fn sync(&self) {
        sync::sync_account(&self.ctx, &self.api).await.unwrap();
    }
    async fn list(&self) -> Vec<Contact> {
        serde_json::from_value(self.call("contacts.list", json!({ "accountId": self.account_id })).await).unwrap()
    }
    async fn server_people(&self) -> Vec<people::Person> {
        let mut out = Vec::new();
        let mut page = None;
        loop {
            let res = people::list_connections(&self.api, page.as_deref(), None, 1000).await.unwrap();
            out.extend(res.connections);
            match res.next_page_token {
                Some(t) => page = Some(t),
                None => return out,
            }
        }
    }
    async fn pending(&self) -> i64 {
        crate::sync::pending::count(&self.ctx.db, self.account_id, "contacts").await.unwrap()
    }
    fn changed_events(&self) -> usize {
        self.ctx.events.captured().iter().filter(|e| e["kind"] == "contacts.changed").count()
    }
}

#[tokio::test]
async fn full_sync_pages_groups_and_sorting() {
    let t = T::new("sync1@duvz.demo").await;
    t.sync().await;
    let list = t.list().await;
    let server = t.server_people().await;
    assert!(list.len() >= 75, "{} contacts", list.len());
    assert_eq!(list.len(), server.len(), "every page was fetched (PAGE_SIZE {})", sync::PAGE_SIZE);
    // sorted by File As (locale-aware: Hebrew after Latin)
    let keys: Vec<String> = list.iter().map(|c| c.file_as.clone()).collect();
    let mut sorted = keys.clone();
    sorted.sort_by(|a, b| super::collate::compare(a, b));
    assert_eq!(keys, sorted);
    assert!(list.iter().any(|c| c.file_as == "Cohen, Sarah" && c.company == "Harbor Capital"));
    assert!(list.iter().any(|c| c.display_name == "משה כהן"));
    assert!(list.iter().any(|c| c.photo_url.as_deref().is_some_and(|u| u.starts_with("data:image/svg"))));
    assert!(list.iter().any(|c| c.photo_url.is_none()), "default photos are not shown");
    // groups with counts, and filtering by group
    let groups = t.call("contacts.groups", json!({})).await;
    let names: Vec<&str> = groups.as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Brokers", "Clients", "Investors"]);
    let investors = groups.as_array().unwrap().iter().find(|g| g["name"] == "Investors").unwrap();
    let members = t.call("contacts.list", json!({ "groupId": investors["id"] })).await;
    assert_eq!(members.as_array().unwrap().len() as i64, investors["count"].as_i64().unwrap());
    assert!(members.as_array().unwrap().len() > 10);
    // letter filter + search
    let c_list = t.call("contacts.list", json!({ "accountId": t.account_id, "letter": "c" })).await;
    assert!(c_list.as_array().unwrap().iter().all(|c| c["fileAs"].as_str().unwrap().to_lowercase().starts_with('c')));
    let found = t.call("contacts.list", json!({ "accountId": t.account_id, "query": "harbor" })).await;
    assert!(found.as_array().unwrap().iter().any(|c| c["displayName"] == "Sarah Cohen"));
    let folders = t.call("contacts.folders", json!({})).await;
    assert_eq!(folders[0]["count"].as_u64().unwrap() as usize, list.len());
    // a second sync with nothing new changes nothing
    let before = t.changed_events();
    t.sync().await;
    assert_eq!(t.changed_events(), before);
}

#[tokio::test]
async fn create_update_delete_round_trip() {
    let t = T::new("sync2@duvz.demo").await;
    t.sync().await;
    let n0 = t.server_people().await.len();

    // create locally: visible at once, pushed on sync
    let created = t
        .call(
            "contacts.create",
            json!({ "accountId": t.account_id, "contact": {
                "givenName": "Dana", "familyName": "Weiss", "company": "Weiss Capital", "jobTitle": "Analyst",
                "emails": [{ "value": "dana@weiss.example", "type": "work" }],
                "phones": [{ "type": "mobile", "value": "+972 50 555 0001" }, { "type": "workFax", "value": "+972 3 555 0002" }],
                "addresses": [{ "type": "work", "street": "1 Herzl St", "city": "Tel Aviv", "country": "Israel" }],
                "mailingAddress": "work", "birthday": "1985-05-06", "notes": "met at conference"
            }}),
        )
        .await;
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["fileAs"], "Weiss, Dana");
    assert_eq!(created["dirty"], true);
    assert!(created["resourceName"].is_null());
    assert_eq!(t.pending().await, 1);
    t.sync().await;
    assert_eq!(t.pending().await, 0);
    let after = t.call("contacts.get", json!({ "id": id })).await;
    let rn = after["resourceName"].as_str().expect("resource name after push").to_string();
    assert_eq!(after["dirty"], false);
    let server = t.server_people().await;
    assert_eq!(server.len(), n0 + 1);
    let remote = people::get_person(&t.api, &rn).await.unwrap();
    let rc = Contact::from_person(&remote);
    assert_eq!(rc.company, "Weiss Capital");
    assert_eq!(rc.phones.len(), 2);
    assert_eq!(rc.birthday.as_deref(), Some("1985-05-06"));
    assert_eq!(rc.mailing_address, "work");

    // update locally → pushed with the etag
    let mut edit: Value = after.clone();
    edit["jobTitle"] = json!("Head of Research");
    edit["emails"] = json!([{ "value": "dana@weiss.example", "type": "work" }, { "value": "dana.w@mail.example", "type": "home" }]);
    let upd = t.call("contacts.update", json!({ "id": id, "contact": edit })).await;
    assert_eq!(upd["jobTitle"], "Head of Research");
    t.sync().await;
    let remote = Contact::from_person(&people::get_person(&t.api, &rn).await.unwrap());
    assert_eq!(remote.job_title, "Head of Research");
    assert_eq!(remote.emails.len(), 2);
    assert_eq!(t.call("contacts.get", json!({ "id": id })).await["dirty"], false);

    // delete locally → deleted on Google
    t.call("contacts.delete", json!({ "ids": [id] })).await;
    assert!(t.reg.call(t.ctx.clone(), "contacts.get", json!({ "id": id })).await.is_err());
    t.sync().await;
    assert_eq!(t.server_people().await.len(), n0);
    assert!(people::get_person(&t.api, &rn).await.is_err());
    assert_eq!(t.list().await.len(), n0);
}

#[tokio::test]
async fn create_then_edit_before_push_and_delete_unpushed() {
    let t = T::new("sync3@duvz.demo").await;
    t.sync().await;
    let n0 = t.server_people().await.len();
    let c = t.call("contacts.create", json!({ "contact": { "givenName": "Temp", "familyName": "Person" } })).await;
    let id = c["id"].as_i64().unwrap();
    let mut e = c.clone();
    e["company"] = json!("Later Co");
    t.call("contacts.update", json!({ "id": id, "contact": e })).await;
    t.sync().await;
    let rn = t.call("contacts.get", json!({ "id": id })).await["resourceName"].as_str().unwrap().to_string();
    assert_eq!(Contact::from_person(&people::get_person(&t.api, &rn).await.unwrap()).company, "Later Co");
    // a contact deleted before it ever reached Google never gets there
    let c2 = t.call("contacts.create", json!({ "contact": { "givenName": "Ghost" } })).await;
    t.call("contacts.delete", json!({ "ids": [c2["id"]] })).await;
    t.sync().await;
    assert_eq!(t.server_people().await.len(), n0 + 1);
    assert_eq!(t.pending().await, 0);
}

#[tokio::test]
async fn incremental_pull_sees_remote_changes_and_deletions() {
    let t = T::new("sync4@duvz.demo").await;
    t.sync().await;
    let n0 = t.list().await.len();
    let token1: Option<String> = {
        let id = t.account_id;
        t.ctx.db.read(move |c| crate::db::util::get_sync_state(c, id, "contacts.syncToken")).await.unwrap()
    };
    assert!(token1.is_some());

    // "another device": edit one, delete one, add one
    let server = t.server_people().await;
    let victim = server.iter().find(|p| p.names.first().and_then(|n| n.given_name.as_deref()) == Some("Sarah")).unwrap();
    let mut changed = victim.clone();
    changed.organizations = vec![people::Organization { name: Some("Harbor Capital Partners".into()), ..Default::default() }];
    let changed = Contact::from_person(&changed).to_person(victim.etag.clone());
    people::update_contact(&t.api, victim.resource_name.as_deref().unwrap(), &changed).await.unwrap();
    let doomed = server.iter().find(|p| p.names.first().and_then(|n| n.given_name.as_deref()) == Some("David")).unwrap();
    people::delete_contact(&t.api, doomed.resource_name.as_deref().unwrap()).await.unwrap();
    let new = Contact { given_name: "Remote".into(), family_name: "Added".into(), ..Default::default() };
    people::create_contact(&t.api, &new.to_person(None)).await.unwrap();

    // incremental: the page with syncToken returns exactly those three
    let page = people::list_connections(&t.api, None, token1.as_deref(), 100).await.unwrap();
    assert_eq!(page.connections.len(), 3);
    assert!(page.connections.iter().any(|p| p.is_deleted()));

    t.sync().await;
    let list = t.list().await;
    assert_eq!(list.len(), n0); // +1 -1
    assert!(list.iter().any(|c| c.company == "Harbor Capital Partners" && c.given_name == "Sarah"));
    assert!(!list.iter().any(|c| c.resource_name == doomed.resource_name));
    assert!(list.iter().any(|c| c.file_as == "Added, Remote"));
}

#[tokio::test]
async fn expired_sync_token_triggers_full_resync() {
    let t = T::new("sync5@duvz.demo").await;
    t.sync().await;
    let n0 = t.list().await.len();
    // a remote delete, then all tokens expire
    let server = t.server_people().await;
    people::delete_contact(&t.api, server[0].resource_name.as_deref().unwrap()).await.unwrap();
    let _: Value = t.api.post(&format!("{}/fake:expireSyncTokens", t.api.urls.people), &[], json!({})).await.unwrap();
    let id = t.account_id;
    let old = t.ctx.db.read(move |c| crate::db::util::get_sync_state(c, id, "contacts.syncToken")).await.unwrap().unwrap();
    let err = people::list_connections(&t.api, None, Some(&old), 10).await.unwrap_err();
    assert!(people::is_expired_sync_token(&err), "{err:#}");
    // real Google: 400 FAILED_PRECONDITION + ErrorInfo EXPIRED_SYNC_TOKEN (not 410)
    assert_eq!(crate::google::client::api_status(&err), Some(400));
    assert_eq!(crate::google::client::api_reason(&err), Some("EXPIRED_SYNC_TOKEN"));
    // a local edit made meanwhile survives the full resync
    let mine = t.list().await.into_iter().find(|c| c.resource_name != server[0].resource_name).unwrap();
    let mut e = serde_json::to_value(&mine).unwrap();
    e["notes"] = json!("edited offline");
    t.call("contacts.update", json!({ "id": mine.id, "contact": e })).await;

    t.sync().await;
    let list = t.list().await;
    assert_eq!(list.len(), n0 - 1, "deletion found by the full resync");
    assert_ne!(t.ctx.db.read(move |c| crate::db::util::get_sync_state(c, id, "contacts.syncToken")).await.unwrap().unwrap(), old);
    assert_eq!(list.iter().find(|c| c.id == mine.id).unwrap().notes, "edited offline");
    let remote = Contact::from_person(&people::get_person(&t.api, mine.resource_name.as_deref().unwrap()).await.unwrap());
    assert_eq!(remote.notes, "edited offline");
}

#[tokio::test]
async fn stale_etag_conflict_local_edit_wins() {
    let t = T::new("sync6@duvz.demo").await;
    t.sync().await;
    let c = t.list().await.into_iter().find(|c| c.given_name == "Yael").unwrap();
    let rn = c.resource_name.clone().unwrap();
    // remote edit bumps the etag
    let remote = people::get_person(&t.api, &rn).await.unwrap();
    let mut rc = Contact::from_person(&remote);
    rc.notes = "remote note".into();
    people::update_contact(&t.api, &rn, &rc.to_person(remote.etag.clone())).await.unwrap();
    // stale update straight to the API is refused
    let stale = people::update_contact(&t.api, &rn, &rc.to_person(remote.etag.clone())).await.unwrap_err();
    assert!(people::is_stale_etag(&stale), "{stale:#}");
    // local edit with the old etag is pushed after refreshing the etag
    let mut e = serde_json::to_value(&c).unwrap();
    e["jobTitle"] = json!("Chief Investment Officer");
    t.call("contacts.update", json!({ "id": c.id, "contact": e })).await;
    t.sync().await;
    assert_eq!(t.pending().await, 0);
    let now = Contact::from_person(&people::get_person(&t.api, &rn).await.unwrap());
    assert_eq!(now.job_title, "Chief Investment Officer");
    let local = t.call("contacts.get", json!({ "id": c.id })).await;
    assert_eq!(local["dirty"], false);
    assert_eq!(local["jobTitle"], "Chief Investment Officer");
}

#[tokio::test]
async fn validation_errors_are_user_errors() {
    let t = T::new("sync7@duvz.demo").await;
    let err = t
        .reg
        .call(t.ctx.clone(), "contacts.create", json!({ "contact": { "givenName": "X", "emails": [{ "value": "not an email" }] } }))
        .await
        .unwrap_err();
    assert_eq!(crate::error::RpcError::from(err).code, "invalid_email");
    let err = t.reg.call(t.ctx.clone(), "contacts.create", json!({ "contact": {} })).await.unwrap_err();
    assert_eq!(crate::error::RpcError::from(err).code, "empty_contact");
    let err = t
        .reg
        .call(t.ctx.clone(), "contacts.create", json!({ "contact": { "givenName": "X", "birthday": "2020-02-31" } }))
        .await
        .unwrap_err();
    assert_eq!(crate::error::RpcError::from(err).code, "invalid_date");
}

#[tokio::test]
async fn autocomplete_and_find_by_email() {
    let t = T::new("sync8@duvz.demo").await;
    t.sync().await;
    // contacts first
    let s: Vec<Value> = serde_json::from_value(t.call("contacts.autocomplete", json!({ "query": "sar" })).await).unwrap();
    assert_eq!(s[0]["email"], "sarah.cohen@harborcap.example");
    assert_eq!(s[0]["source"], "contact");
    // by e-mail prefix, by company
    let s: Vec<Value> = serde_json::from_value(t.call("contacts.autocomplete", json!({ "query": "noa.sh" })).await).unwrap();
    assert_eq!(s[0]["name"], "Noa Shapiro");
    // Hebrew
    let s: Vec<Value> = serde_json::from_value(t.call("contacts.autocomplete", json!({ "query": "כה" })).await).unwrap();
    assert!(s.iter().any(|x| x["name"] == "משה כהן"));
    // other contacts
    let s: Vec<Value> =
        serde_json::from_value(t.call("contacts.autocomplete", json!({ "query": "registration@", "limit": 5 })).await).unwrap();
    assert!(s.iter().any(|x| x["source"] == "other" || x["source"] == "contact"), "{s:?}");
    let other_n: i64 = t.ctx.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM other_contacts", [], |r| r.get(0))?)).await.unwrap();
    assert!(other_n >= 38);
    let other_name: String =
        t.ctx.db.read(|c| Ok(c.query_row("SELECT name FROM other_contacts WHERE name<>'' LIMIT 1", [], |r| r.get(0))?)).await.unwrap();
    let first_word = other_name.split_whitespace().next().unwrap().to_lowercase();
    let s: Vec<Value> = serde_json::from_value(t.call("contacts.autocomplete", json!({ "query": first_word, "limit": 50 })).await).unwrap();
    assert!(s.iter().any(|x| x["source"] == "other"), "{s:?}");
    // no duplicates
    let mut emails: Vec<String> = s.iter().map(|x| x["email"].as_str().unwrap().to_lowercase()).collect();
    let n = emails.len();
    emails.sort();
    emails.dedup();
    assert_eq!(emails.len(), n);

    // recent recipients appear once the mail tables exist (shape probed at runtime)
    let s: Vec<Value> = serde_json::from_value(t.call("contacts.autocomplete", json!({ "query": "zed" })).await).unwrap();
    assert!(s.is_empty());
    let acct = t.account_id;
    t.ctx
        .db
        .write(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO messages (account_id, gmail_id, internal_date, to_json, cc_json, label_ids) VALUES
                   ({acct}, 'm1', 10, '[{{\"name\":\"Zed Alpha\",\"email\":\"zed@alpha.example\"}},{{\"name\":\"Sarah Cohen\",\"email\":\"sarah.cohen@harborcap.example\"}}]', '[]', 'SENT'),
                   ({acct}, 'm2', 20, '[]', '[{{\"name\":\"Zed Beta\",\"email\":\"zed.b@beta.example\"}}]', 'IMPORTANT SENT'),
                   ({acct}, 'm3', 30, '[{{\"name\":\"Zed Received\",\"email\":\"zed.received@x.example\"}}]', '[]', 'INBOX UNREAD');
                 INSERT INTO recipient_cache (email, name, use_count, last_used) VALUES ('zed.cached@c.example', 'Zed Cached', 5, 1);"
            ))?;
            Ok(())
        })
        .await
        .unwrap();
    let s: Vec<Value> = serde_json::from_value(t.call("contacts.autocomplete", json!({ "query": "zed" })).await).unwrap();
    let got: Vec<&str> = s.iter().map(|x| x["email"].as_str().unwrap()).collect();
    assert_eq!(got, vec!["zed.cached@c.example", "zed.b@beta.example", "zed@alpha.example"]);
    assert!(s.iter().all(|x| x["source"] == "recent"));

    // findByEmail
    let card = t.call("contacts.findByEmail", json!({ "email": "SARAH.COHEN@harborcap.example" })).await;
    assert_eq!(card["displayName"], "Sarah Cohen");
    assert_eq!(card["company"], "Harbor Capital");
    assert_eq!(card["phones"][0]["type"], "Business");
    assert!(t.call("contacts.findByEmail", json!({ "email": "nobody@nowhere.example" })).await.is_null());
}

#[tokio::test]
async fn photo_and_move_between_accounts() {
    let t = T::new("sync9@duvz.demo").await;
    let other = accounts::upsert(&t.ctx.db, "sync9b@duvz.demo", "Other", "demo").await.unwrap();
    let other_api = t.ctx.api(other.id).await.unwrap();
    t.sync().await;
    sync::sync_account(&t.ctx, &other_api).await.unwrap();
    let c = t.call("contacts.create", json!({ "accountId": t.account_id, "contact": { "givenName": "Pic", "familyName": "Ture" } })).await;
    let id = c["id"].as_i64().unwrap();
    // 1x1 PNG
    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    t.call("contacts.setPhoto", json!({ "id": id, "dataUrl": png })).await;
    t.sync().await;
    let rn = t.call("contacts.get", json!({ "id": id })).await["resourceName"].as_str().unwrap().to_string();
    let p = people::get_person(&t.api, &rn).await.unwrap();
    assert!(p.photos[0].url.as_deref().unwrap().starts_with("data:image/png"));

    let n_other = t.call("contacts.list", json!({ "accountId": other.id })).await.as_array().unwrap().len();
    let moved = t.call("contacts.move", json!({ "ids": [id], "accountId": other.id })).await;
    let nid = moved[0].as_i64().unwrap();
    assert_ne!(nid, id);
    t.sync().await;
    sync::sync_account(&t.ctx, &other_api).await.unwrap();
    assert!(people::get_person(&t.api, &rn).await.is_err(), "removed from the source account");
    let moved_c = t.call("contacts.get", json!({ "id": nid })).await;
    assert_eq!(moved_c["accountId"], other.id);
    assert!(moved_c["resourceName"].is_string());
    assert_eq!(t.call("contacts.list", json!({ "accountId": other.id })).await.as_array().unwrap().len(), n_other + 1);
}

/// A real contact carries data ClassicMail doesn't model; editing one field must not wipe it.
#[tokio::test]
async fn update_keeps_what_classicmail_does_not_model() {
    use crate::google::people::{ClientData, Event, Organization, Relation};
    let t = T::new("sync9@duvz.demo").await;
    t.sync().await;
    let c = t.list().await.into_iter().find(|c| c.given_name == "Yael").unwrap();
    let rn = c.resource_name.clone().unwrap();
    // as edited in Google Contacts / by another app
    let mut remote = people::get_person(&t.api, &rn).await.unwrap();
    remote.events.push(Event {
        kind: Some("other".into()),
        date: Some(people::Date { year: Some(2019), month: Some(3), day: Some(4) }),
        ..Default::default()
    });
    remote.relations.push(Relation { kind: Some("friend".into()), person: Some("Noa".into()), ..Default::default() });
    remote.client_data.push(ClientData { key: Some("crmId".into()), value: Some("C-17".into()), ..Default::default() });
    remote.organizations.push(Organization { name: Some("Board seat Ltd".into()), title: Some("Director".into()), ..Default::default() });
    people::update_contact(&t.api, &rn, &remote).await.unwrap();
    t.sync().await;
    // edit the phone, the anniversary and the File As locally
    let c = t.list().await.into_iter().find(|x| x.id == c.id).unwrap();
    let mut e = serde_json::to_value(&c).unwrap();
    e["phones"] = json!([{ "type": "mobile", "value": "+972 50 000 0000" }]);
    e["anniversary"] = json!("2001-06-01");
    e["fileAs"] = json!("Yael (Tamar)");
    t.call("contacts.update", json!({ "id": c.id, "contact": e })).await;
    t.sync().await;
    assert_eq!(t.pending().await, 0);
    let after = people::get_person(&t.api, &rn).await.unwrap();
    assert!(after.events.iter().any(|x| x.kind.as_deref() == Some("other")), "other events kept");
    assert!(after.events.iter().any(|x| x.kind.as_deref() == Some("anniversary")), "anniversary added");
    assert!(after.relations.iter().any(|x| x.kind.as_deref() == Some("friend")), "relations untouched");
    assert!(after.client_data.iter().any(|x| x.key.as_deref() == Some("crmId")), "other apps' client data kept");
    assert!(after.client_data.iter().any(|x| x.key.as_deref() == Some("fileAs")));
    assert_eq!(after.organizations.len(), 2, "second organization untouched");
    assert_eq!(after.phone_numbers[0].value.as_deref(), Some("+972 50 000 0000"));
}

#[tokio::test]
async fn sync_token_requires_the_same_parameters() {
    let t = T::new("sync10@duvz.demo").await;
    t.sync().await;
    let id = t.account_id;
    let token = t.ctx.db.read(move |c| crate::db::util::get_sync_state(c, id, "contacts.syncToken")).await.unwrap().unwrap();
    // the incremental call repeats the first call's parameters → fine
    people::list_connections(&t.api, None, Some(&token), sync::PAGE_SIZE).await.unwrap();
    // different personFields with that token → 400 INVALID_ARGUMENT (not an expired token)
    let url = format!("{}/people/me/connections", t.api.urls.people);
    let err = t
        .api
        .get::<Value>(&url, &[("personFields", "names".into()), ("syncToken", token.clone()), ("requestSyncToken", "true".into())])
        .await
        .unwrap_err();
    assert_eq!(crate::google::client::api_status(&err), Some(400));
    assert!(!people::is_expired_sync_token(&err));
    assert!(people::is_mismatched_sync_token(&err));
}

/// A token saved by an older version (other request parameters) must not block contacts sync
/// forever: the sync drops it and starts over.
#[tokio::test]
async fn sync_recovers_from_a_token_for_other_parameters() {
    let t = T::new("sync11@duvz.demo").await;
    let url = format!("{}/people/me/connections", t.api.urls.people);
    let first: Value = t
        .api
        .get(&url, &[("personFields", "names".into()), ("requestSyncToken", "true".into()), ("pageSize", "1000".into())])
        .await
        .unwrap();
    let mut res = first;
    while let Some(p) = res["nextPageToken"].as_str().map(str::to_string) {
        res = t
            .api
            .get(
                &url,
                &[("personFields", "names".into()), ("requestSyncToken", "true".into()), ("pageSize", "1000".into()), ("pageToken", p)],
            )
            .await
            .unwrap();
    }
    let old = res["nextSyncToken"].as_str().expect("sync token").to_string();
    let id = t.account_id;
    t.ctx.db.write(move |c| crate::db::util::set_sync_state(c, id, "contacts.syncToken", Some(&old))).await.unwrap();
    t.sync().await;
    assert!(!t.list().await.is_empty());
    let token = t.ctx.db.read(move |c| crate::db::util::get_sync_state(c, id, "contacts.syncToken")).await.unwrap().unwrap();
    people::list_connections(&t.api, None, Some(&token), sync::PAGE_SIZE).await.unwrap();
}

#[tokio::test]
async fn first_seen_at_survives_syncs_and_dates_come_from_google() {
    let t = T::new("seen1@duvz.demo").await;
    let t0 = crate::db::util::now_ms();
    t.sync().await;
    let list = t.list().await;
    assert!(list.iter().all(|c| c.first_seen_at > 0 && c.first_seen_at <= crate::db::util::now_ms()));
    // first full sync: Date Added is the Google update time (earlier than now), so years-old
    // contacts don't all look added today
    assert!(list.iter().all(|c| c.first_seen_at == c.updated), "Date Added = Google updateTime on first sight");
    assert!(list.iter().filter(|c| c.first_seen_at < t0 - 86_400_000).count() > list.len() / 2);
    let seen: std::collections::HashMap<String, i64> = list.iter().map(|c| (c.resource_name.clone().unwrap(), c.first_seen_at)).collect();

    // a remote edit: Date Modified follows Google's metadata.sources[].updateTime, Date Added stays
    let victim = list.iter().find(|c| c.given_name == "Sarah").unwrap().clone();
    let rn = victim.resource_name.clone().unwrap();
    let remote = people::get_person(&t.api, &rn).await.unwrap();
    let mut rc = Contact::from_person(&remote);
    rc.notes = "met at TASE conference".into();
    let after = people::update_contact(&t.api, &rn, &rc.to_person(remote.etag.clone())).await.unwrap();
    t.sync().await;
    let got = t.list().await.into_iter().find(|c| c.id == victim.id).unwrap();
    assert_eq!(got.notes, "met at TASE conference");
    assert_eq!(got.updated, after.update_time_ms().unwrap());
    assert!(got.updated > victim.updated);
    assert_eq!(got.first_seen_at, victim.first_seen_at);

    // a full resync (expired token) rewrites rows but keeps Date Added
    let _: Value = t.api.post(&format!("{}/fake:expireSyncTokens", t.api.urls.people), &[], json!({})).await.unwrap();
    t.sync().await;
    for c in t.list().await {
        assert_eq!(Some(&c.first_seen_at), seen.get(c.resource_name.as_deref().unwrap()), "{}", c.file_as);
    }

    // a contact created here: added now, and still so after it was pushed and pulled back
    let before = crate::db::util::now_ms();
    let created = t.call("contacts.create", json!({ "contact": { "givenName": "Dana", "familyName": "Levi" } })).await;
    let id = created["id"].as_i64().unwrap();
    let added = created["firstSeenAt"].as_i64().unwrap();
    assert!(added >= before && added <= crate::db::util::now_ms());
    t.sync().await;
    let _: Value = t.api.post(&format!("{}/fake:expireSyncTokens", t.api.urls.people), &[], json!({})).await.unwrap();
    t.sync().await;
    let got = t.call("contacts.get", json!({ "id": id })).await;
    assert!(got["resourceName"].is_string());
    assert_eq!(got["firstSeenAt"].as_i64().unwrap(), added);
    // a local edit changes Date Modified, not Date Added
    let mut e = got.clone();
    e["jobTitle"] = json!("Analyst");
    let edited = t.call("contacts.update", json!({ "id": id, "contact": e })).await;
    assert_eq!(edited["firstSeenAt"].as_i64().unwrap(), added);
    assert!(edited["updated"].as_i64().unwrap() >= added);
    // moving it to another account keeps Date Added
    let other = accounts::upsert(&t.ctx.db, "seen2@duvz.demo", "Other", "demo").await.unwrap();
    let moved = t.call("contacts.move", json!({ "ids": [id], "accountId": other.id })).await;
    let moved = t.call("contacts.get", json!({ "id": moved[0] })).await;
    assert_eq!(moved["firstSeenAt"].as_i64().unwrap(), added);
}

#[tokio::test]
async fn list_sorts_by_each_key_with_hebrew_after_latin() {
    use std::cmp::Ordering::Greater;
    let t = T::new("sort1@duvz.demo").await;
    t.sync().await;
    async fn list(t: &T, sort: &str, desc: bool) -> Vec<Contact> {
        let v = t.call("contacts.list", json!({ "accountId": t.account_id, "sortBy": sort, "descending": desc })).await;
        serde_json::from_value::<Vec<Contact>>(v).unwrap()
    }
    let is_hebrew = |s: &str| s.chars().next().is_some_and(|c| ('\u{05D0}'..='\u{05EA}').contains(&c));
    let last = |c: &Contact| if c.family_name.is_empty() { c.file_as.clone() } else { c.family_name.clone() };
    let first = |c: &Contact| if c.given_name.is_empty() { c.file_as.clone() } else { c.given_name.clone() };
    // last name: Latin names first, then Hebrew, each in alphabetical order
    let lasts: Vec<String> = list(&t, "lastName", false).await.iter().map(last).collect();
    let first_heb = lasts.iter().position(|s| is_hebrew(s)).expect("demo data has Hebrew names");
    assert!(first_heb > 0 && lasts[first_heb..].iter().all(|s| is_hebrew(s)), "{lasts:?}");
    assert!(lasts.windows(2).all(|w| super::collate::compare(&w[0], &w[1]) != Greater), "{lasts:?}");
    // descending: Hebrew first
    assert!(is_hebrew(&last(&list(&t, "lastName", true).await[0])));
    // first name
    let firsts: Vec<String> = list(&t, "firstName", false).await.iter().map(first).collect();
    assert!(firsts.windows(2).all(|w| super::collate::compare(&w[0], &w[1]) != Greater), "{firsts:?}");
    // company: blanks last
    let by_co = list(&t, "company", false).await;
    let first_blank = by_co.iter().position(|c| c.company.is_empty()).unwrap_or(by_co.len());
    assert!(by_co[first_blank..].iter().all(|c| c.company.is_empty()));
    assert!(by_co[..first_blank].windows(2).all(|w| super::collate::compare(&w[0].company, &w[1].company) != Greater));
    // dates
    let added = list(&t, "added", true).await;
    assert!(added.windows(2).all(|w| w[0].first_seen_at >= w[1].first_seen_at));
    let modified = list(&t, "modified", false).await;
    assert!(modified.windows(2).all(|w| w[0].updated <= w[1].updated));
}

#[test]
fn migration_backfills_first_seen_at() {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE accounts (id INTEGER PRIMARY KEY); INSERT INTO accounts(id) VALUES (1);").unwrap();
    c.execute_batch(super::schema::MIGRATIONS[0].1).unwrap();
    c.execute_batch("INSERT INTO contacts(account_id, resource_name, updated) VALUES (1, 'people/a', 1234), (1, 'people/b', 0);").unwrap();
    assert_eq!(super::schema::MIGRATIONS[1].0, 2);
    c.execute_batch(super::schema::MIGRATIONS[1].1).unwrap();
    let a: i64 = c.query_row("SELECT first_seen_at FROM contacts WHERE resource_name='people/a'", [], |r| r.get(0)).unwrap();
    let b: i64 = c.query_row("SELECT first_seen_at FROM contacts WHERE resource_name='people/b'", [], |r| r.get(0)).unwrap();
    assert_eq!(a, 1234);
    // the migration's SQL takes SQLite's own (real) clock, not the app's (a fixed date in tests)
    assert!((b - chrono::Utc::now().timestamp_millis()).abs() < 5_000, "{b}");
}
