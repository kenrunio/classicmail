//! Typed wrappers for the Gmail REST API (users/me/...). Shapes follow
//! https://developers.google.com/gmail/api/reference/rest — only fields we use.

use base64::{Engine, engine::general_purpose::URL_SAFE, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Api, client::Body, seg};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Profile {
    pub email_address: String,
    pub messages_total: i64,
    pub threads_total: i64,
    pub history_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Label {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String, // "system" | "user"
    pub message_list_visibility: Option<String>,
    pub label_list_visibility: Option<String>,
    pub messages_total: Option<i64>,
    pub messages_unread: Option<i64>,
    pub color: Option<LabelColor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LabelColor {
    pub text_color: String,
    pub background_color: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessageRef {
    pub id: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListMessages {
    pub messages: Vec<MessageRef>,
    pub next_page_token: Option<String>,
    pub result_size_estimate: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PartBody {
    pub attachment_id: Option<String>,
    pub size: i64,
    /// base64url
    pub data: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessagePart {
    pub part_id: String,
    pub mime_type: String,
    pub filename: String,
    pub headers: Vec<Header>,
    pub body: PartBody,
    pub parts: Vec<MessagePart>,
}

impl MessagePart {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)).map(|h| h.value.as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub label_ids: Vec<String>,
    pub snippet: String,
    pub history_id: String,
    /// epoch ms as a string
    pub internal_date: String,
    pub size_estimate: i64,
    pub payload: Option<MessagePart>,
    /// base64url RFC 822 (format=raw)
    pub raw: Option<String>,
}

impl Message {
    pub fn internal_date_ms(&self) -> i64 {
        self.internal_date.parse().unwrap_or(0)
    }
    pub fn history_id_u64(&self) -> u64 {
        self.history_id.parse().unwrap_or(0)
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.payload.as_ref().and_then(|p| p.header(name))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HistoryMessage {
    pub message: MessageRef,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HistoryLabel {
    pub message: MessageRef,
    pub label_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct History {
    pub id: String,
    pub messages_added: Vec<HistoryMessage>,
    pub messages_deleted: Vec<HistoryMessage>,
    pub labels_added: Vec<HistoryLabel>,
    pub labels_removed: Vec<HistoryLabel>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListHistory {
    pub history: Vec<History>,
    pub next_page_token: Option<String>,
    pub history_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Draft {
    pub id: String,
    pub message: Message,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListDrafts {
    pub drafts: Vec<Draft>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SendAs {
    pub send_as_email: String,
    pub display_name: String,
    pub reply_to_address: String,
    pub signature: String,
    pub is_primary: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ListLabels {
    labels: Vec<Label>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ListSendAs {
    send_as: Vec<SendAs>,
}

/// A Gmail filter (settings.filters): messages matching `criteria` get `action` when they arrive.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub criteria: FilterCriteria,
    pub action: FilterAction,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterCriteria {
    /// Gmail search syntax ("Has the words")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// "Doesn't have"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negated_query: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterAction {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub add_label_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub remove_label_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ListFilters {
    filter: Vec<Filter>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AttachmentBody {
    data: String,
}

/// Decode Gmail's base64url (with or without padding).
pub fn b64url_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    let t = s.trim_end_matches('=');
    Ok(URL_SAFE_NO_PAD.decode(t).or_else(|_| URL_SAFE.decode(s))?)
}

pub fn b64url_encode(b: &[u8]) -> String {
    URL_SAFE.encode(b)
}

/// Above this, messages go through the upload endpoint (`uploadType=multipart`: JSON metadata +
/// message/rfc822) instead of base64 in a JSON body; the JSON path is meant for small messages.
const UPLOAD_THRESHOLD: usize = 2 * 1024 * 1024;
/// Gmail refuses uploads above 35 MB (and messages above 25 MB of attachments).
pub const MAX_UPLOAD: usize = 35 * 1024 * 1024;

/// `multipart/related` body for Google's multipart upload: metadata JSON first, then the message.
/// Note: with `uploadType=media` there is no way to pass metadata such as `threadId`, so a reply
/// sent that way would start a new conversation.
pub fn multipart_related(metadata: &Value, raw: &[u8]) -> (String, Vec<u8>) {
    let boundary = format!("classicmail_{:016x}{:016x}", rand::random::<u64>(), rand::random::<u64>());
    let mut body = Vec::with_capacity(raw.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes());
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Type: message/rfc822\r\n\r\n").as_bytes());
    body.extend_from_slice(raw);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/related; boundary={boundary}"), body)
}

impl Api {
    fn gm(&self, path: &str) -> String {
        format!("{}/users/me/{}", self.urls.gmail, path)
    }

    pub async fn gmail_profile(&self) -> anyhow::Result<Profile> {
        self.get(&self.gm("profile"), &[]).await
    }

    pub async fn gmail_labels(&self) -> anyhow::Result<Vec<Label>> {
        Ok(self.get::<ListLabels>(&self.gm("labels"), &[]).await?.labels)
    }

    /// One label with counts (messagesTotal / messagesUnread).
    pub async fn gmail_label(&self, id: &str) -> anyhow::Result<Label> {
        self.get(&self.gm(&format!("labels/{}", seg(id))), &[]).await
    }

    pub async fn gmail_create_label(&self, name: &str) -> anyhow::Result<Label> {
        self.post(&self.gm("labels"), &[], json!({ "name": name, "labelListVisibility": "labelShow", "messageListVisibility": "show" }))
            .await
    }

    pub async fn gmail_rename_label(&self, id: &str, name: &str) -> anyhow::Result<Label> {
        self.patch(&self.gm(&format!("labels/{}", seg(id))), &[], json!({ "name": name })).await
    }

    pub async fn gmail_delete_label(&self, id: &str) -> anyhow::Result<()> {
        self.delete(&self.gm(&format!("labels/{}", seg(id))), &[]).await
    }

    /// messages.list. `q` uses Gmail search syntax.
    pub async fn gmail_list(
        &self,
        q: Option<&str>,
        label_ids: &[&str],
        page_token: Option<&str>,
        max: u32,
        include_spam_trash: bool,
    ) -> anyhow::Result<ListMessages> {
        let mut query: Vec<(&str, String)> = vec![("maxResults", max.to_string())];
        if let Some(q) = q {
            query.push(("q", q.to_string()));
        }
        for l in label_ids {
            query.push(("labelIds", l.to_string()));
        }
        if let Some(t) = page_token {
            query.push(("pageToken", t.to_string()));
        }
        if include_spam_trash {
            query.push(("includeSpamTrash", "true".into()));
        }
        self.get(&self.gm("messages"), &query).await
    }

    /// messages.get. `format`: "metadata" | "full" | "raw" | "minimal".
    pub async fn gmail_get(&self, id: &str, format: &str, metadata_headers: &[&str]) -> anyhow::Result<Message> {
        let mut query: Vec<(&str, String)> = vec![("format", format.to_string())];
        for h in metadata_headers {
            query.push(("metadataHeaders", h.to_string()));
        }
        self.get(&self.gm(&format!("messages/{}", seg(id))), &query).await
    }

    pub async fn gmail_modify(&self, id: &str, add: &[String], remove: &[String]) -> anyhow::Result<Message> {
        self.post(&self.gm(&format!("messages/{}/modify", seg(id))), &[], json!({ "addLabelIds": add, "removeLabelIds": remove })).await
    }

    /// Up to 1000 ids per call.
    pub async fn gmail_batch_modify(&self, ids: &[String], add: &[String], remove: &[String]) -> anyhow::Result<()> {
        for chunk in ids.chunks(1000) {
            let _: Value = self
                .post(&self.gm("messages/batchModify"), &[], json!({ "ids": chunk, "addLabelIds": add, "removeLabelIds": remove }))
                .await?;
        }
        Ok(())
    }

    pub async fn gmail_trash(&self, id: &str) -> anyhow::Result<Message> {
        self.post(&self.gm(&format!("messages/{}/trash", seg(id))), &[], json!({})).await
    }

    pub async fn gmail_untrash(&self, id: &str) -> anyhow::Result<Message> {
        self.post(&self.gm(&format!("messages/{}/untrash", seg(id))), &[], json!({})).await
    }

    /// Permanent delete (needs the full mail.google.com scope).
    pub async fn gmail_delete(&self, id: &str) -> anyhow::Result<()> {
        self.delete(&self.gm(&format!("messages/{}", seg(id))), &[]).await
    }

    pub async fn gmail_history(&self, start_history_id: &str, page_token: Option<&str>) -> anyhow::Result<ListHistory> {
        let mut query: Vec<(&str, String)> = vec![("startHistoryId", start_history_id.to_string()), ("maxResults", "500".into())];
        if let Some(t) = page_token {
            query.push(("pageToken", t.to_string()));
        }
        self.get(&self.gm("history"), &query).await
    }

    /// Attachment bytes (decoded).
    pub async fn gmail_attachment(&self, message_id: &str, attachment_id: &str) -> anyhow::Result<Vec<u8>> {
        let a: AttachmentBody =
            self.get(&self.gm(&format!("messages/{}/attachments/{}", seg(message_id), seg(attachment_id))), &[]).await?;
        b64url_decode(&a.data)
    }

    /// Upload `raw` with JSON `metadata` (`uploadType=multipart`) to `/upload/.../users/me/{path}`,
    /// with `query` (besides uploadType).
    async fn gmail_upload<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        metadata: Value,
        raw: &[u8],
    ) -> anyhow::Result<T> {
        if raw.len() > MAX_UPLOAD {
            return Err(crate::user_err!(
                "too_large",
                "The message is too large for Gmail ({:.1} MB; the limit is 35 MB including encoding).",
                raw.len() as f64 / 1_048_576.0
            ));
        }
        let (content_type, bytes) = multipart_related(&metadata, raw);
        let url = format!("{}/users/me/{path}", self.urls.gmail_upload);
        let mut q: Vec<(&str, String)> = vec![("uploadType", "multipart".into())];
        q.extend(query.iter().cloned());
        self.call(method, &url, &q, Body::Raw { content_type, bytes }).await
    }

    /// messages.import: put a complete RFC 822 message into this mailbox with `label_ids`, as if it
    /// had been delivered (moving / copying mail from another account). Its date stays the one in
    /// its Date header, Gmail never files it as spam, and invitations in it aren't added to the
    /// calendar again.
    pub async fn gmail_import(&self, raw: &[u8], label_ids: &[String]) -> anyhow::Result<Message> {
        let query: Vec<(&str, String)> =
            vec![("neverMarkSpam", "true".into()), ("internalDateSource", "dateHeader".into()), ("processForCalendar", "false".into())];
        let meta = json!({ "labelIds": label_ids });
        if raw.len() > UPLOAD_THRESHOLD {
            return self.gmail_upload(Method::POST, "messages/import", &query, meta, raw).await;
        }
        let mut body = meta;
        body["raw"] = json!(b64url_encode(raw));
        self.post(&self.gm("messages/import"), &query, body).await
    }

    /// Send a complete RFC 822 message. `thread_id` keeps a reply in its conversation — Gmail only
    /// honours it when the Subject matches and In-Reply-To / References point into the thread.
    pub async fn gmail_send(&self, raw: &[u8], thread_id: Option<&str>) -> anyhow::Result<Message> {
        let mut meta = json!({});
        if let Some(t) = thread_id {
            meta["threadId"] = json!(t);
        }
        if raw.len() > UPLOAD_THRESHOLD {
            return self.gmail_upload(Method::POST, "messages/send", &[], meta, raw).await;
        }
        meta["raw"] = json!(b64url_encode(raw));
        self.post(&self.gm("messages/send"), &[], meta).await
    }

    pub async fn gmail_create_draft(&self, raw: &[u8], thread_id: Option<&str>) -> anyhow::Result<Draft> {
        let mut msg = json!({});
        if let Some(t) = thread_id {
            msg["threadId"] = json!(t);
        }
        if raw.len() > UPLOAD_THRESHOLD {
            return self.gmail_upload(Method::POST, "drafts", &[], json!({ "message": msg }), raw).await;
        }
        msg["raw"] = json!(b64url_encode(raw));
        self.post(&self.gm("drafts"), &[], json!({ "message": msg })).await
    }

    pub async fn gmail_update_draft(&self, draft_id: &str, raw: &[u8], thread_id: Option<&str>) -> anyhow::Result<Draft> {
        let mut msg = json!({});
        if let Some(t) = thread_id {
            msg["threadId"] = json!(t);
        }
        let path = format!("drafts/{}", seg(draft_id));
        if raw.len() > UPLOAD_THRESHOLD {
            return self.gmail_upload(Method::PUT, &path, &[], json!({ "id": draft_id, "message": msg }), raw).await;
        }
        msg["raw"] = json!(b64url_encode(raw));
        self.put(&self.gm(&path), &[], json!({ "id": draft_id, "message": msg })).await
    }

    pub async fn gmail_get_draft(&self, draft_id: &str, format: &str) -> anyhow::Result<Draft> {
        self.get(&self.gm(&format!("drafts/{}", seg(draft_id))), &[("format", format.to_string())]).await
    }

    pub async fn gmail_list_drafts(&self, page_token: Option<&str>) -> anyhow::Result<ListDrafts> {
        let mut query: Vec<(&str, String)> = vec![("maxResults", "500".into())];
        if let Some(t) = page_token {
            query.push(("pageToken", t.to_string()));
        }
        self.get(&self.gm("drafts"), &query).await
    }

    pub async fn gmail_delete_draft(&self, draft_id: &str) -> anyhow::Result<()> {
        self.delete(&self.gm(&format!("drafts/{}", seg(draft_id))), &[]).await
    }

    pub async fn gmail_send_draft(&self, draft_id: &str) -> anyhow::Result<Message> {
        self.post(&self.gm("drafts/send"), &[], json!({ "id": draft_id })).await
    }

    pub async fn gmail_send_as(&self) -> anyhow::Result<Vec<SendAs>> {
        Ok(self.get::<ListSendAs>(&self.gm("settings/sendAs"), &[]).await?.send_as)
    }

    /// settings.filters.list.
    pub async fn gmail_filters(&self) -> anyhow::Result<Vec<Filter>> {
        Ok(self.get::<ListFilters>(&self.gm("settings/filters"), &[]).await?.filter)
    }

    /// settings.filters.create (scope gmail.settings.basic). Returns the new filter's id.
    pub async fn gmail_create_filter(&self, f: &Filter) -> anyhow::Result<String> {
        let created: Filter = self.post(&self.gm("settings/filters"), &[], serde_json::to_value(f)?).await?;
        created.id.ok_or_else(|| anyhow::anyhow!("Gmail returned a filter without an id"))
    }

    /// settings.filters.delete (scope gmail.settings.basic).
    pub async fn gmail_delete_filter(&self, id: &str) -> anyhow::Result<()> {
        self.delete(&self.gm(&format!("settings/filters/{}", seg(id))), &[]).await
    }
}
