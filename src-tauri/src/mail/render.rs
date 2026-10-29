//! Reading-pane HTML: sanitise (ammonia, generous e-mail allowlist incl. `<style>` and tables; no
//! scripts, handlers, forms, frames, objects), rewrite `cid:` to the `cmattach` protocol and block
//! remote pictures unless allowed.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use regex::Regex;

/// Base URL of the attachment protocol as seen from the WebView. On Windows (WebView2) Tauri serves
/// custom protocols as `http://<scheme>.localhost/`; elsewhere as `<scheme>://localhost/`.
pub fn attach_base() -> &'static str {
    if cfg!(windows) { "http://cmattach.localhost" } else { "cmattach://localhost" }
}

pub fn inline_url(att_id: i64) -> String {
    format!("{}/inline/{att_id}", attach_base())
}

/// Shown instead of a blocked picture: a light box with a small red ✕ (like Outlook).
const BLOCKED_IMG: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='100%25' height='100%25'%3E%3Crect x='0.5' y='0.5' width='100%25' height='100%25' fill='%23f7f7f7' stroke='%23a0a0a0'/%3E%3Cpath d='M5 5l7 7M12 5l-7 7' stroke='%23d22' stroke-width='2'/%3E%3C/svg%3E";

#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub html: String,
    /// remote content was found and blocked
    pub blocked: bool,
    /// remote content exists (so "Download Pictures" makes sense even when allowed)
    pub has_remote: bool,
}

fn css_url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)url\(\s*(['"]?)\s*([^'")]*)\s*(['"]?)\s*\)"#).unwrap())
}

fn css_import_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)@import[^;]*;?"#).unwrap())
}

fn body_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)<(/?)body\b"#).unwrap())
}

fn is_remote(url: &str) -> bool {
    let l = url.trim().to_ascii_lowercase();
    l.starts_with("http:") || l.starts_with("https:") || l.starts_with("//")
}

/// Where `cid:` pictures point: lower-cased Content-ID → local attachment id (served by the
/// `cmattach` protocol), or → a ready URL (archived messages use `data:` URLs).
pub trait CidUrls: Clone + Send + Sync + 'static {
    fn url_for(&self, key: &str) -> Option<String>;
}

impl CidUrls for HashMap<String, i64> {
    fn url_for(&self, key: &str) -> Option<String> {
        self.get(key).map(|id| inline_url(*id))
    }
}

impl CidUrls for HashMap<String, String> {
    fn url_for(&self, key: &str) -> Option<String> {
        self.get(key).cloned()
    }
}

fn cid_target<C: CidUrls>(url: &str, cids: &C) -> Option<String> {
    let l = url.trim();
    let cid = l.strip_prefix("cid:").or_else(|| l.strip_prefix("CID:")).or_else(|| l.strip_prefix("Cid:"))?;
    let cid = urlencoding::decode(cid).map(|c| c.into_owned()).unwrap_or_else(|_| cid.to_string());
    let key = cid.trim_start_matches('<').trim_end_matches('>').to_ascii_lowercase();
    cids.url_for(&key)
}

/// Quoted strings that are remote URLs (`image-set("https://…" 1x)`, `src("…")`): CSS can load
/// those without `url()`.
fn css_remote_string_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)(["'])\s*(?:https?:|//)[^"']*["']"#).unwrap())
}

/// Decode the CSS escapes that could hide a function name, keyword or URL (`u\72l(`, `@\69mport`,
/// `\68ttp\3a //`): only escapes that stand for ASCII letters / digits or `: / . - _`. Anything else
/// (quotes, `<`, braces...) stays escaped, so decoding can't end a string or a `<style>` element.
fn css_unescape_word_chars(css: &str) -> Cow<'_, str> {
    if !css.contains('\\') {
        return Cow::Borrowed(css);
    }
    let keep = |c: char| c.is_ascii_alphanumeric() || ":/.-_".contains(c);
    let chars: Vec<char> = css.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' || i + 1 >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut hex = String::new();
        while j < chars.len() && hex.len() < 6 && chars[j].is_ascii_hexdigit() {
            hex.push(chars[j]);
            j += 1;
        }
        if hex.is_empty() {
            // `\x` stands for x
            let c = chars[i + 1];
            if keep(c) {
                out.push(c);
            } else {
                out.push('\\');
                out.push(c);
            }
            i += 2;
            continue;
        }
        // a hex escape swallows one whitespace after it
        let mut end = j;
        if end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
        match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
            Some(c) if keep(c) => out.push(c),
            _ => out.extend(&chars[i..end]),
        }
        i = end;
    }
    Cow::Owned(out)
}

/// Rewrite `url(...)` in CSS: cid → cmattach, remote → removed when blocking; `@import` always
/// removed. (The reading pane's CSP blocks remote loads as well; this keeps print / reply clean and
/// drives the "pictures blocked" info bar.)
fn rewrite_css<C: CidUrls>(css: &str, cids: &C, allow_remote: bool, blocked: &AtomicBool, has_remote: &AtomicBool) -> String {
    let css = css_unescape_word_chars(css);
    let css = css_import_re().replace_all(&css, "");
    let css = css_remote_string_re().replace_all(&css, |c: &regex::Captures| {
        has_remote.store(true, Ordering::Relaxed);
        if allow_remote {
            return c[0].to_string();
        }
        blocked.store(true, Ordering::Relaxed);
        "\"\"".to_string()
    });
    css_url_re()
        .replace_all(&css, |c: &regex::Captures| {
            let url = c.get(2).map(|m| m.as_str()).unwrap_or("");
            if let Some(t) = cid_target(url, cids) {
                return format!("url('{t}')");
            }
            if is_remote(url) {
                has_remote.store(true, Ordering::Relaxed);
                if allow_remote {
                    return format!("url('{}')", url.replace('\'', "%27"));
                }
                blocked.store(true, Ordering::Relaxed);
                return "none".to_string();
            }
            if url.to_ascii_lowercase().starts_with("data:image/") {
                return format!("url('{}')", url.replace('\'', "%27"));
            }
            "none".to_string()
        })
        .into_owned()
}

const TAGS: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "area",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
    "big",
    "blockquote",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "font",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "map",
    "mark",
    "nav",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "tt",
    "u",
    "ul",
    "var",
    "wbr",
    "main",
    "label",
    "o:p",
];

const ATTRS: &[&str] = &[
    "style",
    "class",
    "id",
    "dir",
    "lang",
    "title",
    "align",
    "valign",
    "bgcolor",
    "width",
    "height",
    "border",
    "cellpadding",
    "cellspacing",
    "color",
    "face",
    "size",
    "background",
    "nowrap",
    "colspan",
    "rowspan",
    "span",
    "alt",
    "src",
    "href",
    "name",
    "start",
    "type",
    "cite",
    "datetime",
    "hspace",
    "vspace",
    "frame",
    "rules",
    "summary",
    "abbr",
    "scope",
    "headers",
    "clear",
    "noshade",
    "compact",
    "shape",
    "coords",
    "usemap",
    "reversed",
    "value",
    "role",
    "aria-hidden",
    "aria-label",
];

/// Sanitise e-mail HTML for display. `cids` maps lower-cased Content-IDs → local attachment ids
/// (or URLs, see `CidUrls`).
pub fn render_html<C: CidUrls>(html: &str, cids: &C, allow_remote: bool) -> Rendered {
    if super::mime::html_too_deep(html) {
        // hostile nesting (see MAX_HTML_DEPTH): show the text only
        return Rendered { html: super::mime::text_to_html(&super::mime::strip_tags(html)), ..Default::default() };
    }
    let blocked = Arc::new(AtomicBool::new(false));
    let has_remote = Arc::new(AtomicBool::new(false));
    let (blocked2, has_remote2, cids2) = (blocked.clone(), has_remote.clone(), cids.clone());
    // keep <body> styling: turn it into a div
    let pre = body_tag_re().replace_all(html, "<${1}div data-cm-body=\"1\"").into_owned();
    let mut b = ammonia::Builder::default();
    b.tags(TAGS.iter().copied().collect::<HashSet<_>>())
        .clean_content_tags(
            ["script", "title", "noscript", "template", "iframe", "object", "embed", "textarea", "select", "button", "svg", "math"]
                .into_iter()
                .collect::<HashSet<_>>(),
        )
        .generic_attributes(ATTRS.iter().copied().collect::<HashSet<_>>())
        .add_tag_attributes("div", ["data-cm-body"])
        .url_schemes(["http", "https", "mailto", "cid", "data", "tel"].into_iter().collect::<HashSet<_>>())
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true)
        .attribute_filter(move |element, attribute, value| -> Option<Cow<'_, str>> {
            let (blocked, has_remote, cids) = (&blocked2, &has_remote2, &cids2);
            match (element, attribute) {
                ("img", "src") => {
                    if let Some(t) = cid_target(value, cids) {
                        return Some(Cow::Owned(t));
                    }
                    if is_remote(value) {
                        has_remote.store(true, Ordering::Relaxed);
                        if allow_remote {
                            return Some(Cow::Borrowed(value));
                        }
                        blocked.store(true, Ordering::Relaxed);
                        return Some(Cow::Borrowed(BLOCKED_IMG));
                    }
                    if value.trim_start().to_ascii_lowercase().starts_with("data:image/") {
                        return Some(Cow::Borrowed(value));
                    }
                    Some(Cow::Borrowed(BLOCKED_IMG))
                }
                (_, "src") => None,
                (_, "background") => {
                    if let Some(t) = cid_target(value, cids) {
                        return Some(Cow::Owned(t));
                    }
                    if is_remote(value) {
                        has_remote.store(true, Ordering::Relaxed);
                        if allow_remote {
                            return Some(Cow::Borrowed(value));
                        }
                        blocked.store(true, Ordering::Relaxed);
                    }
                    None
                }
                (_, "style") => Some(Cow::Owned(rewrite_css(value, cids, allow_remote, blocked, has_remote))),
                // `name` on img/form/embed/object becomes a property of `document` (DOM clobbering):
                // `<img name="addEventListener">` would break the parent's link interception
                (e, "name") if e != "a" => None,
                ("a" | "area", "href") => {
                    let l = value.trim().to_ascii_lowercase();
                    if l.starts_with("http:") || l.starts_with("https:") || l.starts_with("mailto:") || l.starts_with("tel:") {
                        Some(Cow::Borrowed(value))
                    } else {
                        None
                    }
                }
                _ => Some(Cow::Borrowed(value)),
            }
        });
    // ammonia doesn't look inside <style>; fix up its CSS afterwards
    let cleaned = b.clean(&pre).to_string();
    let out = rewrite_style_blocks(&cleaned, cids, allow_remote, &blocked, &has_remote);
    Rendered { html: out, blocked: blocked.load(Ordering::Relaxed), has_remote: has_remote.load(Ordering::Relaxed) }
}

fn rewrite_style_blocks<C: CidUrls>(html: &str, cids: &C, allow_remote: bool, blocked: &AtomicBool, has_remote: &AtomicBool) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    while let Some(s) = lower[pos..].find("<style") {
        let s = pos + s;
        let Some(gt) = lower[s..].find('>') else { break };
        let body_start = s + gt + 1;
        let Some(e) = lower[body_start..].find("</style") else { break };
        let body_end = body_start + e;
        out.push_str(&html[pos..body_start]);
        // `expression(` / `behavior:` are IE-only, but strip anyway; and no position:fixed overlays
        let css = rewrite_css(&html[body_start..body_end], cids, allow_remote, blocked, has_remote);
        out.push_str(&css.replace("expression(", "x(").replace("behavior:", "x:"));
        pos = body_end;
    }
    out.push_str(&html[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cids() -> HashMap<String, i64> {
        HashMap::from([("logo@x".to_string(), 7)])
    }

    #[test]
    fn strips_dangerous_markup() {
        let r = render_html(
            "<html><head><title>T</title><script>alert(1)</script></head><body bgcolor=\"#eee\" onload=\"x()\"><p onclick=\"evil()\" style=\"color:red\">Hi</p>\
             <form action=\"http://x\"><input name=a></form><iframe src=\"http://evil\"></iframe><a href=\"javascript:alert(1)\">j</a>\
             <a href=\"https://ok.example/\">ok</a><object data=x></object></body></html>",
            &cids(),
            false,
        );
        let h = r.html.to_lowercase();
        assert!(
            !h.contains("script")
                && !h.contains("onload")
                && !h.contains("onclick")
                && !h.contains("<form")
                && !h.contains("<iframe")
                && !h.contains("<object")
                && !h.contains("javascript:")
        );
        assert!(!h.contains(">t<"), "title text removed");
        assert!(h.contains("style=\"color:red\""));
        assert!(h.contains("href=\"https://ok.example/\""));
        assert!(h.contains("bgcolor=\"#eee\""), "body attributes kept on the div: {h}");
    }

    #[test]
    fn rewrites_cid_and_blocks_remote_images() {
        let src = "<style>.h{background:url(https://t.example/bg.png)} @import url(http://x/y.css);</style>\
                   <img src=\"cid:logo@x\"><img src=\"https://t.example/pixel.gif\" width=\"1\" height=\"1\">\
                   <td background=\"http://t.example/bg.jpg\" style=\"background-image:url('https://t.example/a.png')\">x</td>";
        let r = render_html(src, &cids(), false);
        assert!(r.blocked && r.has_remote);
        assert!(r.html.contains(&inline_url(7)), "{}", r.html);
        assert!(!r.html.contains("t.example"), "{}", r.html);
        assert!(!r.html.contains("@import"));
        let r = render_html(src, &cids(), true);
        assert!(!r.blocked && r.has_remote);
        assert!(r.html.contains("https://t.example/pixel.gif"));
        assert!(r.html.contains("https://t.example/bg.png"));
    }

    /// Hostile mail: nothing that runs, navigates, submits or loads from the network may survive
    /// (with remote content blocked).
    #[test]
    fn hostile_markup_is_neutralised() {
        let src = r#"<html><head><base href="https://evil.example/"><meta http-equiv="refresh" content="0;url=https://evil.example/">
<link rel="stylesheet" href="https://evil.example/a.css"><style>
.a{background:u\72l(https://evil.example/esc1.png)}
.b{background:url(\68ttp\3a //evil.example/esc2.png)}
@\69mport url(https://evil.example/esc3.css);
@import "https://evil.example/plain.css";
.c{background-image:image-set("https://evil.example/set.png" 1x)}
@font-face{font-family:x;src:url(https://evil.example/f.woff)}
</style></head><body background="https://evil.example/bg.png">
<a href="javascript:alert(1)">j</a><a href="vbscript:msgbox(1)">v</a><a href="data:text/html,<script>alert(1)</script>">d</a>
<a href="https://ok.example/" target="_top">ok</a><map name="m"><area href="javascript:alert(2)" shape="rect" coords="0,0,1,1"></map>
<img name="addEventListener" src="cid:logo@x"><img src="https://evil.example/pixel.gif" srcset="https://evil.example/2x.gif 2x">
<form action="https://evil.example/"><input name="pw" type="password"><button formaction="https://evil.example/">Go</button></form>
<svg><script>alert(3)</script><image href="https://evil.example/svg.png"/><a xlink:href="javascript:alert(4)"><text>t</text></a></svg>
<math><mtext><table><mglyph><style><img src=x onerror=alert(5)></style></mglyph></table></mtext></math>
<video src="https://evil.example/v.mp4" poster="https://evil.example/p.png"></video><audio src="https://evil.example/a.mp3"></audio>
<iframe srcdoc="<script>alert(6)</script>"></iframe><embed src="https://evil.example/e.swf"><object data="https://evil.example/o"></object>
<div style="background-image:image-set('https://evil.example/inline.png' 1x);width:expression(alert(7))">x</div>
<table background="https://evil.example/t.png"><tr><td style="background:URL( 'https://evil.example/td.png' )">c</td></tr></table>
<p onmouseover="alert(8)" style="behavior:url(x.htc)">hover</p><noscript><img src=https://evil.example/ns.png></noscript>
</body></html>"#;
        let r = render_html(src, &cids(), false);
        let h = r.html.to_lowercase();
        assert!(r.blocked && r.has_remote);
        for bad in [
            "evil.example",
            "<base",
            "<meta",
            "<link",
            "<form",
            "<input",
            "<button",
            "<svg",
            "<math",
            "<video",
            "<audio",
            "<iframe",
            "<embed",
            "<object",
            "<script",
            "javascript:",
            "vbscript:",
            "data:text",
            "onerror",
            "onmouseover",
            "srcset",
            "target=",
            "@import",
            "name=\"addeventlistener\"",
        ] {
            assert!(!h.contains(bad), "{bad} survived: {h}");
        }
        assert!(h.contains("href=\"https://ok.example/\""));
        assert!(h.contains(&inline_url(7)), "cid image kept: {h}");
        // still all removed with remote content allowed except the plain remote references
        let r = render_html(src, &cids(), true);
        let h = r.html.to_lowercase();
        for bad in ["<script", "javascript:", "onerror", "<form", "<base", "<meta", "@import"] {
            assert!(!h.contains(bad), "{bad} survived (allowed): {h}");
        }
    }

    #[test]
    fn css_escapes_are_decoded_only_for_word_characters() {
        assert_eq!(css_unescape_word_chars(r"u\72l(\68ttp\3a //x)"), "url(http://x)");
        assert_eq!(css_unescape_word_chars(r"@\69 mport"), "@import");
        // quotes, angle brackets and backslashes stay escaped (can't end a string or <style>)
        assert_eq!(css_unescape_word_chars(r#"content:"\22\3c/style\3e\\""#), r#"content:"\22\3c/style\3e\\""#);
        assert_eq!(css_unescape_word_chars("plain"), "plain");
    }

    #[test]
    fn deep_nesting_falls_back_to_text_quickly() {
        let deep = format!("<p>top</p>{}secret text{}", "<div><b>".repeat(30_000), "</b></div>".repeat(30_000));
        let t = std::time::Instant::now();
        let r = render_html(&deep, &cids(), false);
        assert!(r.html.contains("secret text") && !r.html.contains("<b>"), "{}", &r.html[..200.min(r.html.len())]);
        let text = crate::mail::mime::html_to_text(&deep);
        assert!(text.contains("secret text"));
        assert!(t.elapsed().as_secs() < 2, "took {:?}", t.elapsed());
        // ordinary documents (unclosed <p>/<li>/<td>, stray end tags) are not "deep"
        let normal = format!("<table>{}</table>{}", "<tr><td><p>x".repeat(3000), "<ul><li>a<li>b</ul></span></div>".repeat(3000));
        assert!(!crate::mail::mime::html_too_deep(&normal));
        // stray end tags close nothing, so they can't hide real nesting
        assert!(crate::mail::mime::html_too_deep(&"<div></p></span>".repeat(2000)));
        // the tag stripper stays linear with unclosed <style>
        let t = std::time::Instant::now();
        let _ = crate::mail::mime::strip_tags(&"<style>x".repeat(200_000));
        assert!(t.elapsed().as_secs() < 2, "strip_tags took {:?}", t.elapsed());
    }

    #[test]
    fn keeps_tables_fonts_and_hebrew() {
        let r = render_html(
            "<table cellpadding=\"4\" border=\"1\"><tr><td align=\"right\" dir=\"rtl\"><font face=\"Arial\" color=\"#333\">שלום</font></td></tr></table>",
            &cids(),
            false,
        );
        assert!(
            r.html.contains("cellpadding=\"4\"")
                && r.html.contains("<font face=\"Arial\"")
                && r.html.contains("שלום")
                && r.html.contains("dir=\"rtl\"")
        );
        assert!(!r.blocked);
    }
}
