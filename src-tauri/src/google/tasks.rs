//! Typed wrappers for the Google Tasks REST API (v1).
//!
//! <https://developers.google.com/tasks/reference/rest>. Dates: `updated` / `completed` are RFC 3339
//! timestamps; `due` is RFC 3339 too but Google only keeps the date part (time is always 00:00Z).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Api, seg};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskList {
    pub id: String,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub updated: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// "needsAction" | "completed"
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub completed: Option<String>,
    #[serde(default)]
    pub deleted: Option<bool>,
    #[serde(default)]
    pub hidden: Option<bool>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub position: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
}

fn lists_url(api: &Api) -> String {
    format!("{}/users/@me/lists", api.urls.tasks)
}

fn tasks_url(api: &Api, list: &str) -> String {
    format!("{}/lists/{}/tasks", api.urls.tasks, seg(list))
}

// ---- task lists -------------------------------------------------------------------------------

pub async fn list_tasklists(api: &Api) -> anyhow::Result<Vec<TaskList>> {
    api.get_all_items(&lists_url(api), &[("maxResults", "100".to_string())]).await
}

pub async fn insert_tasklist(api: &Api, title: &str) -> anyhow::Result<TaskList> {
    api.post(&lists_url(api), &[], serde_json::json!({ "title": title })).await
}

pub async fn patch_tasklist(api: &Api, id: &str, title: &str) -> anyhow::Result<TaskList> {
    api.patch(&format!("{}/{}", lists_url(api), seg(id)), &[], serde_json::json!({ "title": title })).await
}

pub async fn delete_tasklist(api: &Api, id: &str) -> anyhow::Result<()> {
    api.delete(&format!("{}/{}", lists_url(api), seg(id)), &[]).await
}

// ---- tasks ------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ListOpts {
    pub show_completed: bool,
    /// must be true as well to see tasks completed in Google's own apps (they hide them at once)
    pub show_hidden: bool,
    pub show_deleted: bool,
    /// RFC 3339 lower bound on `updated` (incremental sync)
    pub updated_min: Option<String>,
}

/// Every task of a list (all pages).
pub async fn list_tasks(api: &Api, list: &str, opts: &ListOpts) -> anyhow::Result<Vec<Task>> {
    let mut q = vec![
        ("maxResults", "100".to_string()),
        ("showCompleted", opts.show_completed.to_string()),
        ("showHidden", opts.show_hidden.to_string()),
        ("showDeleted", opts.show_deleted.to_string()),
        // tasks assigned to the user from Docs / Chat spaces appear in their lists in Google
        // Tasks; the API leaves them out unless asked
        ("showAssigned", "true".to_string()),
    ];
    if let Some(u) = &opts.updated_min {
        q.push(("updatedMin", u.clone()));
    }
    api.get_all_items(&tasks_url(api, list), &q).await
}

pub async fn get_task(api: &Api, list: &str, id: &str) -> anyhow::Result<Task> {
    api.get(&format!("{}/{}", tasks_url(api, list), seg(id)), &[]).await
}

/// Insert; `parent` / `previous` place it (top of the parent's children when `previous` is None).
pub async fn insert_task(api: &Api, list: &str, body: Value, parent: Option<&str>, previous: Option<&str>) -> anyhow::Result<Task> {
    let mut q = Vec::new();
    if let Some(p) = parent {
        q.push(("parent", p.to_string()));
    }
    if let Some(p) = previous {
        q.push(("previous", p.to_string()));
    }
    api.post(&tasks_url(api, list), &q, body).await
}

pub async fn patch_task(api: &Api, list: &str, id: &str, body: Value) -> anyhow::Result<Task> {
    api.patch(&format!("{}/{}", tasks_url(api, list), seg(id)), &[], body).await
}

pub async fn delete_task(api: &Api, list: &str, id: &str) -> anyhow::Result<()> {
    api.delete(&format!("{}/{}", tasks_url(api, list), seg(id)), &[]).await
}

/// Move within a list (or to `destination` list): new parent and previous sibling.
pub async fn move_task(
    api: &Api,
    list: &str,
    id: &str,
    parent: Option<&str>,
    previous: Option<&str>,
    destination: Option<&str>,
) -> anyhow::Result<Task> {
    let mut q = Vec::new();
    if let Some(p) = parent {
        q.push(("parent", p.to_string()));
    }
    if let Some(p) = previous {
        q.push(("previous", p.to_string()));
    }
    if let Some(d) = destination {
        q.push(("destinationTasklist", d.to_string()));
    }
    api.call(reqwest::Method::POST, &format!("{}/{}/move", tasks_url(api, list), seg(id)), &q, super::client::Body::None).await
}

// ---- date helpers -----------------------------------------------------------------------------

pub use super::rfc3339_ms as parse_ts;

/// epoch ms → RFC 3339 with milliseconds, UTC ("2026-10-11T08:00:00.000Z").
pub fn fmt_ts(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms).unwrap_or_default().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Google `due` ("2026-10-11T00:00:00.000Z") → "2026-10-11".
pub fn due_to_date(s: &str) -> Option<String> {
    let d = s.get(0..10)?;
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok().map(|_| d.to_string())
}

/// "2026-10-11" → Google `due`.
pub fn date_to_due(d: &str) -> String {
    format!("{d}T00:00:00.000Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        assert_eq!(due_to_date("2026-10-11T00:00:00.000Z").as_deref(), Some("2026-10-11"));
        assert_eq!(due_to_date("garbage"), None);
        assert_eq!(date_to_due("2026-01-02"), "2026-01-02T00:00:00.000Z");
        let ms = parse_ts("2026-10-11T08:15:30.123Z").unwrap();
        assert_eq!(fmt_ts(ms), "2026-10-11T08:15:30.123Z");
    }
}
