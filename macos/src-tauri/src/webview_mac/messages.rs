//! Messages from Serica's page scripts (the `serica` handler, reachable only
//! from Serica's own content world). Origins are taken from WebKit's frame
//! info, never from what the message says.

use super::*;

fn list(v: &serde_json::Value, k: &str) -> Vec<String> {
    v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).take(5000).collect()).unwrap_or_default()
}

/// Handles one message; the value is the reply the page script gets.
pub fn handle(message: &WKScriptMessage) -> Option<serde_json::Value> {
    let wv = unsafe { message.webView() }?;
    let ctx = ctx_of(&wv)?;
    let raw = unsafe { message.body() };
    let body = to_json(Some(&raw));
    if body.get("__serica").is_none() {
        return None;
    }
    let frame = unsafe { message.frameInfo() };
    let main = unsafe { frame.isMainFrame() };
    let origin = unsafe { frame.securityOrigin() };
    let (proto, host) = unsafe { (origin.protocol().to_string(), origin.host().to_string()) };
    match body["t"].as_str().unwrap_or("") {
        "settled" => {
            // Only the page on screen matters (the UI captures it).
            if main && ctx.kind == Kind::Tab && ctx.app.store.lock().active_tab_id.as_deref() == Some(ctx.tab_id.as_str()) {
                ui_emit(&ctx.app.handle, "serica:page-settled", ctx.tab_id.as_str());
            }
            None
        }
        "cosmetic" => {
            if !main {
                return None;
            }
            let (generichide, top, exceptions) = {
                let st = ctx.st.borrow();
                (st.generichide, st.top_url.clone(), st.exceptions.clone())
            };
            if generichide || !ctx.app.shields.active_for(&top) {
                return None;
            }
            let selectors = ctx.app.shields.class_id_selectors(&list(&body, "classes"), &list(&body, "ids"), &exceptions);
            if !selectors.is_empty() {
                let css = hide_css(selectors);
                let js = format!(
                    "(()=>{{const s=document.createElement('style');s.textContent={};(document.head||document.documentElement).appendChild(s);}})();",
                    js_string(&css)
                );
                super::api::eval_world(&wv, &js);
            }
            None
        }
        t @ ("ext-install" | "ext-query") => {
            if !main || proto != "https" || host != "chromewebstore.google.com" {
                return None;
            }
            let ext_id = body["id"].as_str().filter(|s| crate::extensions::is_extension_id(s))?;
            if t == "ext-query" {
                if ctx.app.extensions.is_installed(ext_id) {
                    super::api::eval_world(&wv, &crate::extensions::store_state_js(ext_id, "installed", None));
                }
            } else {
                crate::commands::spawn_store_install(&ctx.app, &ctx.tab_id, ext_id);
            }
            None
        }
        "favicon" => {
            if !main || ctx.kind != Kind::Tab {
                return None;
            }
            let href = body["href"].as_str().filter(|h| (h.starts_with("http://") || h.starts_with("https://") || h.starts_with("data:image/")) && h.len() <= 200_000)?;
            ctx.app.tab_event(&ctx.tab_id, TabEvent::Favicon(href.to_string()));
            None
        }
        "media" => {
            let playing = body["playing"].as_bool().unwrap_or(false);
            let key = unsafe { format!("{proto}://{host}|{}", url_string(frame.request().URL().as_deref())) };
            let (native, any) = {
                let mut st = ctx.st.borrow_mut();
                if main {
                    st.top_playing = playing;
                } else if playing {
                    st.playing_frames.insert(key);
                } else {
                    st.playing_frames.remove(&key);
                }
                (st.native_audio, st.top_playing || !st.playing_frames.is_empty())
            };
            if !native {
                ctx.app.tab_event(&ctx.tab_id, TabEvent::Audio(any));
            }
            None
        }
        "media-frame" => {
            if !main {
                let url = unsafe { url_string(frame.request().URL().as_deref()) };
                let mut st = ctx.st.borrow_mut();
                let known = st.media_frames.iter().any(|f| unsafe { url_string(f.request().URL().as_deref()) } == url);
                if !known && st.media_frames.len() < 16 {
                    st.media_frames.push(frame.clone());
                }
            }
            None
        }
        "ctx" => {
            let s = |k: &str| body[k].as_str().map(str::to_string).filter(|v| !v.is_empty());
            ctx.st.borrow_mut().last_ctx = Some(CtxInfo {
                link: s("link"),
                image: s("image"),
                selection: s("selection"),
                editable: body["editable"].as_bool().unwrap_or(false),
                page: s("page").unwrap_or_default(),
                point: if main { body["x"].as_f64().zip(body["y"].as_f64()) } else { None },
            });
            None
        }
        "cert-proceed" => {
            if !main {
                return None;
            }
            let nonce = body["nonce"].as_str().unwrap_or("");
            let target = {
                let mut st = ctx.st.borrow_mut();
                match st.cert_nonce.take() {
                    Some((n, h)) if !n.is_empty() && n == nonce => Some((h, st.top_url.clone())),
                    other => {
                        st.cert_nonce = other;
                        None
                    }
                }
            };
            if let Some((h, url)) = target {
                tracing::warn!("continuing to {h} despite its certificate (user's choice, this session only)");
                CERT_EXCEPTIONS.with(|c| c.borrow_mut().insert(h));
                super::nav::load_url(&wv, &url);
            }
            None
        }
        t if t.starts_with("pw-") => {
            if ctx.kind != Kind::Tab {
                return None;
            }
            let top = ctx.st.borrow().top_url.clone();
            crate::passwords::message(&ctx.app, &ctx.tab_id, &ctx.profile, &proto, &host, unsafe { origin.port() } as u16, &top, &body)
        }
        _ => None,
    }
}
