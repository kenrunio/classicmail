//! Rule data model and the pure matching / planning logic (no I/O — fully unit-tested).
//!
//! JSON shapes (camelCase, tagged by `type`), e.g.
//! `{"type":"from","people":[{"name":"Sarah Cohen","email":"sarah@x.com"}]}`,
//! `{"type":"moveToFolder","folderId":12,"folderPath":"Clients/Orion"}`.

use serde::{Deserialize, Serialize};

/// A person in a rule ("from <people or distribution list>"). An empty `email` means "match by
/// name / address text"; an `email` starting with `@` matches a whole domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
}

impl Person {
    pub fn email(e: &str) -> Self {
        Person { name: String::new(), email: e.into() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Importance {
    Low,
    Normal,
    High,
}

impl Importance {
    /// `messages.importance`: 0 low, 1 normal, 2 high.
    pub fn as_i64(self) -> i64 {
        match self {
            Importance::Low => 0,
            Importance::Normal => 1,
            Importance::High => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Condition {
    /// from <people or distribution list>
    From { people: Vec<Person> },
    /// sent to <people or distribution list> (To or Cc)
    SentTo { people: Vec<Person> },
    /// with <specific words> in the subject (any of)
    SubjectContains { words: Vec<String> },
    /// with <specific words> in the body
    BodyContains { words: Vec<String> },
    /// with <specific words> in the subject or body
    SubjectOrBodyContains { words: Vec<String> },
    /// with <specific words> in the message header
    HeaderContains { words: Vec<String> },
    /// which has an attachment
    HasAttachment,
    /// marked as <importance>
    Importance { level: Importance },
    /// sent only to me
    SentOnlyToMe,
    /// where my name is in the To box
    MyNameInTo,
    /// where my name is in the To or Cc box
    MyNameInToOrCc,
    /// where my name is not in the To box
    MyNameNotInTo,
    /// through the <specified> account
    ThroughAccount { account_id: i64 },
    /// from senders at <specific domains>
    FromDomain { domains: Vec<String> },
}

/// Follow-up flag values understood by `mail.setFlag`.
pub const FLAG_VALUES: &[&str] = &["today", "tomorrow", "thisWeek", "nextWeek", "noDate", "complete", "none"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Action {
    /// move it to the <specified> folder
    MoveToFolder {
        folder_id: i64,
        #[serde(default)]
        folder_path: String,
    },
    /// move a copy to the <specified> folder
    CopyToFolder {
        folder_id: i64,
        #[serde(default)]
        folder_path: String,
    },
    /// delete it
    Delete,
    /// mark it as read
    MarkRead,
    /// flag message for <follow up at this time>
    Flag { flag: String },
    /// mark it as <importance> (local only — Gmail has no importance field)
    SetImportance { level: Importance },
    /// forward it to <people or distribution list>
    ForwardTo { people: Vec<Person> },
    /// display a Desktop Alert
    DisplayDesktopAlert,
    /// play a <sound>
    PlaySound {
        #[serde(default)]
        sound: String,
    },
    /// stop processing more rules
    StopProcessing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub position: i64,
    /// None = every mailbox
    #[serde(default)]
    pub account_id: Option<i64>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    #[serde(default)]
    pub exceptions: Vec<Condition>,
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn yes() -> bool {
    true
}

/// What the matcher knows about one message. `me` = the mailbox owner's addresses (lowercase).
#[derive(Debug, Clone, Default)]
pub struct MsgFacts {
    pub id: i64,
    pub account_id: i64,
    pub from_name: String,
    pub from_email: String,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub subject: String,
    pub has_attachments: bool,
    pub importance: i64,
    /// plain text body ("" when not loaded / unavailable)
    pub body: String,
    /// raw headers as "Name: value" lines
    pub headers: String,
    pub me: Vec<String>,
}

// ---- matching -----------------------------------------------------------------------------------

fn lc(s: &str) -> String {
    s.to_lowercase()
}

/// Lower case with every run of whitespace as one space, so a phrase still matches where the
/// body text was wrapped onto the next line.
fn fold(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn contains_any(hay: &str, words: &[String]) -> bool {
    let words: Vec<String> = words.iter().map(|w| fold(w)).filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return false;
    }
    let hay = fold(hay);
    words.iter().any(|w| hay.contains(w.as_str()))
}

/// Does an address (name + email) match one rule person?
fn person_matches(p: &Person, name: &str, email: &str) -> bool {
    let email = lc(email.trim());
    let pe = lc(p.email.trim());
    if !pe.is_empty() {
        if let Some(domain) = pe.strip_prefix('@') {
            return !domain.is_empty() && email.ends_with(&format!("@{domain}"));
        }
        if pe.contains('@') {
            return email == pe;
        }
        // a bare word typed in the address box: substring of the address or name
        return email.contains(&pe) || lc(name).contains(&pe);
    }
    let pn = lc(p.name.trim());
    !pn.is_empty() && (lc(name).contains(&pn) || email.contains(&pn))
}

fn any_person(people: &[Person], name: &str, email: &str) -> bool {
    people.iter().any(|p| person_matches(p, name, email))
}

fn is_me(m: &MsgFacts, email: &str) -> bool {
    let e = lc(email.trim());
    !e.is_empty() && m.me.contains(&e)
}

impl Condition {
    /// Needs the body or the full headers (`message_bodies`) to be evaluated.
    pub fn needs_body(&self) -> bool {
        matches!(self, Condition::BodyContains { .. } | Condition::SubjectOrBodyContains { .. } | Condition::HeaderContains { .. })
    }

    pub fn matches(&self, m: &MsgFacts) -> bool {
        match self {
            Condition::From { people } => any_person(people, &m.from_name, &m.from_email),
            Condition::SentTo { people } => m.to.iter().chain(m.cc.iter()).any(|a| any_person(people, &a.name, &a.email)),
            Condition::SubjectContains { words } => contains_any(&m.subject, words),
            Condition::BodyContains { words } => contains_any(&m.body, words),
            Condition::SubjectOrBodyContains { words } => contains_any(&m.subject, words) || contains_any(&m.body, words),
            Condition::HeaderContains { words } => contains_any(&m.headers, words),
            Condition::HasAttachment => m.has_attachments,
            Condition::Importance { level } => m.importance == level.as_i64(),
            Condition::SentOnlyToMe => m.to.len() == 1 && is_me(m, &m.to[0].email) && m.cc.iter().all(|a| is_me(m, &a.email)),
            Condition::MyNameInTo => m.to.iter().any(|a| is_me(m, &a.email)),
            Condition::MyNameInToOrCc => m.to.iter().chain(m.cc.iter()).any(|a| is_me(m, &a.email)),
            Condition::MyNameNotInTo => !m.to.iter().any(|a| is_me(m, &a.email)),
            Condition::ThroughAccount { account_id } => m.account_id == *account_id,
            Condition::FromDomain { domains } => {
                let email = lc(m.from_email.trim());
                let Some((_, dom)) = email.rsplit_once('@') else { return false };
                domains
                    .iter()
                    .map(|d| lc(d.trim().trim_start_matches('@')))
                    .any(|d| !d.is_empty() && (dom == d || dom.ends_with(&format!(".{d}"))))
            }
        }
    }
}

impl Rule {
    pub fn stops(&self) -> bool {
        self.actions.iter().any(|a| matches!(a, Action::StopProcessing))
    }

    pub fn needs_body(&self) -> bool {
        self.conditions.iter().chain(self.exceptions.iter()).any(Condition::needs_body)
    }

    pub fn applies_to_account(&self, account_id: i64) -> bool {
        self.account_id.is_none_or(|a| a == account_id)
    }

    /// All conditions hold (none = every message) and no exception does.
    pub fn matches(&self, m: &MsgFacts) -> bool {
        self.applies_to_account(m.account_id)
            && self.conditions.iter().all(|c| c.matches(m))
            && !self.exceptions.iter().any(|c| c.matches(m))
    }

    /// Problems that stop the rule from being saved (shown to the user).
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Please specify a name for this rule.".into());
        }
        if !self.actions.iter().any(|a| !matches!(a, Action::StopProcessing)) {
            return Err("You must select at least one action for this rule.".into());
        }
        for c in self.conditions.iter().chain(self.exceptions.iter()) {
            let empty = match c {
                Condition::From { people } | Condition::SentTo { people } => {
                    people.iter().all(|p| p.email.trim().is_empty() && p.name.trim().is_empty())
                }
                Condition::SubjectContains { words }
                | Condition::BodyContains { words }
                | Condition::SubjectOrBodyContains { words }
                | Condition::HeaderContains { words } => words.iter().all(|w| w.trim().is_empty()),
                Condition::FromDomain { domains } => domains.iter().all(|w| w.trim().is_empty()),
                _ => false,
            };
            if empty {
                return Err("You must specify a value for each underlined value in the rule description.".into());
            }
        }
        for a in &self.actions {
            match a {
                Action::MoveToFolder { folder_id, .. } | Action::CopyToFolder { folder_id, .. } if *folder_id <= 0 => {
                    return Err("Choose a folder for the move/copy action.".into());
                }
                Action::Flag { flag } if !FLAG_VALUES.contains(&flag.as_str()) => {
                    return Err(format!("Unknown follow-up flag \"{flag}\"."));
                }
                Action::ForwardTo { people } if (people.is_empty() || people.iter().any(|p| !valid_email(&p.email))) => {
                    return Err("Forward to: enter valid e-mail addresses.".into());
                }
                _ => {}
            }
        }
        Ok(())
    }
}

pub fn valid_email(e: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^[A-Za-z0-9._%+'\-]+@[A-Za-z0-9\-]+(\.[A-Za-z0-9\-]+)*\.[A-Za-z]{2,}$").unwrap())
        .is_match(e.trim())
}

// ---- planning -----------------------------------------------------------------------------------

/// The outcome of running an ordered rule list over one message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// ids of the rules that matched, in order
    pub matched: Vec<i64>,
    /// names of the matched rules (for alerts / logs)
    pub matched_names: Vec<String>,
    /// de-duplicated actions in execution order
    pub actions: Vec<Action>,
}

/// Evaluate `rules` (already ordered by position) against one message, honouring
/// "stop processing more rules". Disabled rules are skipped.
pub fn plan(rules: &[Rule], m: &MsgFacts) -> Plan {
    let mut out = Plan::default();
    let mut raw = Vec::new();
    for r in rules {
        if !r.enabled || !r.matches(m) {
            continue;
        }
        out.matched.push(r.id);
        out.matched_names.push(r.name.clone());
        raw.extend(r.actions.iter().cloned());
        if r.stops() {
            break;
        }
    }
    out.actions = resolve(raw);
    out
}

/// Turn the concatenated actions of all matched rules into a sensible sequence:
/// copies first, then state changes (read, flag, importance), notifications and forwards, and the
/// final disposition last — delete wins over moves; a second move becomes a copy (Outlook keeps
/// the first "move" and moves copies to the other folders).
pub fn resolve(actions: Vec<Action>) -> Vec<Action> {
    let mut copies: Vec<Action> = Vec::new();
    let mut first_move: Option<Action> = None;
    let mut delete = false;
    let mut mark_read = false;
    let mut flag: Option<Action> = None;
    let mut importance: Option<Action> = None;
    let mut forwards: Vec<Action> = Vec::new();
    let mut alert = false;
    let mut sound: Option<Action> = None;

    let push_copy = |copies: &mut Vec<Action>, id: i64, path: String| {
        if !copies.iter().any(|c| matches!(c, Action::CopyToFolder { folder_id, .. } if *folder_id == id)) {
            copies.push(Action::CopyToFolder { folder_id: id, folder_path: path });
        }
    };

    for a in actions {
        match a {
            Action::MoveToFolder { folder_id, folder_path } => match &first_move {
                None => first_move = Some(Action::MoveToFolder { folder_id, folder_path }),
                Some(Action::MoveToFolder { folder_id: f, .. }) if *f == folder_id => {}
                Some(_) => push_copy(&mut copies, folder_id, folder_path),
            },
            Action::CopyToFolder { folder_id, folder_path } => push_copy(&mut copies, folder_id, folder_path),
            Action::Delete => delete = true,
            Action::MarkRead => mark_read = true,
            a @ Action::Flag { .. } => flag = Some(a),
            a @ Action::SetImportance { .. } => importance = Some(a),
            a @ Action::ForwardTo { .. } if !forwards.contains(&a) => forwards.push(a),
            Action::ForwardTo { .. } => {}
            Action::DisplayDesktopAlert => alert = true,
            a @ Action::PlaySound { .. } => sound = Some(a),
            Action::StopProcessing => {}
        }
    }
    // a copy into the folder we move to is pointless
    if let Some(Action::MoveToFolder { folder_id: mv, .. }) = &first_move {
        copies.retain(|c| !matches!(c, Action::CopyToFolder { folder_id, .. } if folder_id == mv));
    }

    let mut out = copies;
    if mark_read {
        out.push(Action::MarkRead);
    }
    out.extend(flag);
    out.extend(importance);
    out.extend(forwards);
    if alert {
        out.push(Action::DisplayDesktopAlert);
    }
    out.extend(sound);
    if delete {
        out.push(Action::Delete);
    } else if let Some(mv) = first_move {
        out.push(mv);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, email: &str) -> Person {
        Person { name: name.into(), email: email.into() }
    }

    fn msg() -> MsgFacts {
        MsgFacts {
            id: 1,
            account_id: 7,
            from_name: "Sarah Cohen".into(),
            from_email: "Sarah.Cohen@HarborCap.example".into(),
            to: vec![p("Kenny Green", "kenny@duvz.demo")],
            cc: vec![p("Maya Goldberg", "maya.goldberg@duvz.demo")],
            subject: "Orion Q3 earnings call — דוח רבעוני".into(),
            has_attachments: true,
            importance: 2,
            body: "Hi Kenny, please find the deck attached. Regards".into(),
            headers: "X-Mailer: Outlook 16\nList-Id: <ir.harborcap.example>".into(),
            me: vec!["kenny@duvz.demo".into()],
        }
    }

    fn rule(id: i64, conds: Vec<Condition>, actions: Vec<Action>) -> Rule {
        Rule {
            id,
            name: format!("r{id}"),
            enabled: true,
            position: id,
            account_id: None,
            conditions: conds,
            exceptions: vec![],
            actions,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn mv(id: i64) -> Action {
        Action::MoveToFolder { folder_id: id, folder_path: format!("F{id}") }
    }
    fn cp(id: i64) -> Action {
        Action::CopyToFolder { folder_id: id, folder_path: format!("F{id}") }
    }

    #[test]
    fn from_matches_email_name_and_domain() {
        let m = msg();
        let c = |people| Condition::From { people };
        assert!(c(vec![Person::email("sarah.cohen@harborcap.example")]).matches(&m));
        assert!(c(vec![Person::email("@harborcap.example")]).matches(&m));
        assert!(!c(vec![Person::email("@cap.example")]).matches(&m));
        assert!(c(vec![p("sarah", "")]).matches(&m));
        assert!(c(vec![Person::email("harborcap")]).matches(&m)); // bare word = substring
        assert!(!c(vec![Person::email("david.levi@meridian.example")]).matches(&m));
        assert!(!c(vec![]).matches(&m));
        assert!(!c(vec![p("", "")]).matches(&m));
    }

    #[test]
    fn sent_to_checks_to_and_cc() {
        let m = msg();
        assert!(Condition::SentTo { people: vec![Person::email("maya.goldberg@duvz.demo")] }.matches(&m));
        assert!(Condition::SentTo { people: vec![p("Kenny", "")] }.matches(&m));
        assert!(!Condition::SentTo { people: vec![Person::email("info@duvz.demo")] }.matches(&m));
    }

    #[test]
    fn text_conditions() {
        let m = msg();
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(Condition::SubjectContains { words: w(&["EARNINGS"]) }.matches(&m));
        assert!(Condition::SubjectContains { words: w(&["nothing", "רבעוני"]) }.matches(&m));
        assert!(!Condition::SubjectContains { words: w(&["deck"]) }.matches(&m));
        assert!(!Condition::SubjectContains { words: w(&["", "  "]) }.matches(&m));
        assert!(Condition::BodyContains { words: w(&["deck"]) }.matches(&m));
        assert!(!Condition::BodyContains { words: w(&["orion"]) }.matches(&m));
        assert!(Condition::SubjectOrBodyContains { words: w(&["orion"]) }.matches(&m));
        assert!(Condition::SubjectOrBodyContains { words: w(&["deck"]) }.matches(&m));
        assert!(!Condition::SubjectOrBodyContains { words: w(&["zzz"]) }.matches(&m));
        assert!(Condition::HeaderContains { words: w(&["list-id: <ir."]) }.matches(&m));
        assert!(!Condition::HeaderContains { words: w(&["x-spam"]) }.matches(&m));
        // a phrase wrapped onto the next line still matches
        let mut wrapped = m.clone();
        wrapped.body = "please find the\n  deck attached".into();
        assert!(Condition::BodyContains { words: w(&["The Deck"]) }.matches(&wrapped));
        let mut empty = m.clone();
        empty.body.clear();
        assert!(!Condition::BodyContains { words: w(&["deck"]) }.matches(&empty));
    }

    #[test]
    fn attachment_and_importance() {
        let mut m = msg();
        assert!(Condition::HasAttachment.matches(&m));
        assert!(Condition::Importance { level: Importance::High }.matches(&m));
        assert!(!Condition::Importance { level: Importance::Low }.matches(&m));
        m.has_attachments = false;
        m.importance = 1;
        assert!(!Condition::HasAttachment.matches(&m));
        assert!(Condition::Importance { level: Importance::Normal }.matches(&m));
    }

    #[test]
    fn my_name_conditions() {
        let mut m = msg();
        // To: me, Cc: Maya → not "only to me"
        assert!(!Condition::SentOnlyToMe.matches(&m));
        assert!(Condition::MyNameInTo.matches(&m));
        assert!(Condition::MyNameInToOrCc.matches(&m));
        assert!(!Condition::MyNameNotInTo.matches(&m));
        m.cc.clear();
        assert!(Condition::SentOnlyToMe.matches(&m));
        m.to[0].email = "KENNY@duvz.demo".into();
        assert!(Condition::SentOnlyToMe.matches(&m));
        // me only in Cc
        m.to = vec![p("", "all@harborcap.example")];
        m.cc = vec![p("", "kenny@duvz.demo")];
        assert!(!Condition::SentOnlyToMe.matches(&m));
        assert!(!Condition::MyNameInTo.matches(&m));
        assert!(Condition::MyNameInToOrCc.matches(&m));
        assert!(Condition::MyNameNotInTo.matches(&m));
        // two in To
        m.to = vec![p("", "kenny@duvz.demo"), p("", "x@y.example")];
        m.cc.clear();
        assert!(!Condition::SentOnlyToMe.matches(&m));
    }

    #[test]
    fn account_and_domain() {
        let m = msg();
        assert!(Condition::ThroughAccount { account_id: 7 }.matches(&m));
        assert!(!Condition::ThroughAccount { account_id: 8 }.matches(&m));
        let d = |s: &str| Condition::FromDomain { domains: vec![s.into()] };
        assert!(d("harborcap.example").matches(&m));
        assert!(d("@HarborCap.example").matches(&m));
        assert!(d("example").matches(&m)); // parent domain
        assert!(!d("cap.example").matches(&m));
        assert!(!d("").matches(&m));
    }

    #[test]
    fn rule_needs_all_conditions_and_no_exception() {
        let m = msg();
        let mut r = rule(1, vec![Condition::HasAttachment, Condition::MyNameInTo], vec![Action::MarkRead]);
        assert!(r.matches(&m));
        r.conditions.push(Condition::Importance { level: Importance::Low });
        assert!(!r.matches(&m));
        r.conditions.pop();
        r.exceptions.push(Condition::SubjectContains { words: vec!["earnings".into()] });
        assert!(!r.matches(&m));
        // no conditions = every message
        let all = rule(2, vec![], vec![Action::MarkRead]);
        assert!(all.matches(&m));
        // account scoping
        let mut scoped = all.clone();
        scoped.account_id = Some(99);
        assert!(!scoped.matches(&m));
        scoped.account_id = Some(7);
        assert!(scoped.matches(&m));
    }

    #[test]
    fn plan_order_and_stop_processing() {
        let m = msg();
        let rules = vec![
            rule(1, vec![Condition::HasAttachment], vec![cp(10)]),
            rule(2, vec![Condition::Importance { level: Importance::Low }], vec![Action::Delete]),
            rule(3, vec![Condition::MyNameInTo], vec![mv(20), Action::StopProcessing]),
            rule(4, vec![], vec![Action::Delete]),
        ];
        let p = plan(&rules, &m);
        assert_eq!(p.matched, vec![1, 3]);
        assert_eq!(p.actions, vec![cp(10), mv(20)]);

        // disabled rule 3 → processing continues to rule 4 (delete wins over nothing)
        let mut r2 = rules.clone();
        r2[2].enabled = false;
        let p = plan(&r2, &m);
        assert_eq!(p.matched, vec![1, 4]);
        assert_eq!(p.actions, vec![cp(10), Action::Delete]);

        // stop processing only applies when the rule matched
        let mut r3 = rules.clone();
        r3[2].conditions = vec![Condition::SentOnlyToMe];
        let p = plan(&r3, &m);
        assert_eq!(p.matched, vec![1, 4]);
    }

    #[test]
    fn resolve_actions() {
        let flag = |f: &str| Action::Flag { flag: f.into() };
        let fwd = Action::ForwardTo { people: vec![Person::email("a@b.example")] };
        let out = resolve(vec![
            mv(1),
            Action::MarkRead,
            flag("today"),
            mv(2),
            cp(2),
            cp(3),
            Action::MarkRead,
            flag("tomorrow"),
            fwd.clone(),
            fwd.clone(),
            Action::DisplayDesktopAlert,
            Action::StopProcessing,
            mv(1),
        ]);
        assert_eq!(out, vec![cp(2), cp(3), Action::MarkRead, flag("tomorrow"), fwd, Action::DisplayDesktopAlert, mv(1)]);
        // delete beats move; copies kept
        assert_eq!(resolve(vec![mv(1), cp(5), Action::Delete]), vec![cp(5), Action::Delete]);
        // copy into the move target is dropped
        assert_eq!(resolve(vec![cp(1), mv(1)]), vec![mv(1)]);
        assert!(resolve(vec![Action::StopProcessing]).is_empty());
    }

    #[test]
    fn json_shapes() {
        let r: Rule = serde_json::from_value(serde_json::json!({
            "name": "Harbor", "accountId": 3,
            "conditions": [{"type": "from", "people": [{"name": "Sarah", "email": "s@h.example"}]},
                           {"type": "subjectContains", "words": ["Q3"]},
                           {"type": "throughAccount", "accountId": 3},
                           {"type": "importance", "level": "high"},
                           {"type": "hasAttachment"}],
            "actions": [{"type": "moveToFolder", "folderId": 4, "folderPath": "Clients"},
                        {"type": "flag", "flag": "noDate"}, {"type": "stopProcessing"}]
        }))
        .unwrap();
        assert!(r.enabled);
        assert!(r.stops());
        assert_eq!(r.conditions[2], Condition::ThroughAccount { account_id: 3 });
        let back = serde_json::to_value(&r.actions[0]).unwrap();
        assert_eq!(back, serde_json::json!({"type": "moveToFolder", "folderId": 4, "folderPath": "Clients"}));
        assert!(r.validate().is_ok());
    }

    #[test]
    fn validation() {
        let mut r = rule(1, vec![], vec![]);
        assert!(r.validate().unwrap_err().contains("action"));
        r.actions = vec![Action::StopProcessing];
        assert!(r.validate().is_err());
        r.actions = vec![Action::MoveToFolder { folder_id: 0, folder_path: String::new() }];
        assert!(r.validate().unwrap_err().contains("folder"));
        r.actions = vec![Action::ForwardTo { people: vec![Person::email("not an address")] }];
        assert!(r.validate().is_err());
        r.actions = vec![Action::ForwardTo { people: vec![Person::email("o'brien@x.example")] }];
        assert!(r.validate().is_ok());
        r.conditions = vec![Condition::SubjectContains { words: vec![" ".into()] }];
        assert!(r.validate().unwrap_err().contains("underlined"));
        r.conditions.clear();
        r.name = " ".into();
        assert!(r.validate().unwrap_err().contains("name"));
    }
}
