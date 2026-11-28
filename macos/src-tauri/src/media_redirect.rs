//! Media `redirect=` filters on WebKit (Spotify's audio ads, and the like).
//!
//! uBlock Origin answers some ad media requests with one second of silence
//! instead of blocking them (`$media,redirect=noop-1s.mp4`): the player then
//! plays "the ad", which ends at once, and moves on, where a blocked request
//! would stall it. WebKit's content blockers have no redirect action, so
//! these filters are dropped from the compiled lists. Here they become a
//! small page-world script for the sites they name: when the page points an
//! `<audio>` / `<video>` / `<source>` (or `new Audio()`) at a URL a filter
//! matches, it gets a silent clip instead, before anything is requested.
//! Exceptions (`@@…$media`) for the same site win, as in uBlock.

use std::collections::HashMap;

/// One `$media` filter scoped to sites.
#[derive(Debug, Clone, PartialEq)]
struct Rule {
    /// JavaScript regex source for the request URL.
    re: String,
    exception: bool,
}

/// Filters by the site (`domain=` / `from=` entry) they apply on.
#[derive(Debug, Default)]
pub struct MediaRules {
    by_site: HashMap<String, Vec<Rule>>,
}

impl MediaRules {
    pub fn parse<'a>(texts: impl IntoIterator<Item = &'a str>) -> Self {
        let mut by_site: HashMap<String, Vec<Rule>> = HashMap::new();
        for text in texts {
            for line in text.lines() {
                if let Some((sites, rule)) = parse_line(line.trim()) {
                    for s in sites {
                        let v = by_site.entry(s).or_default();
                        if !v.contains(&rule) {
                            v.push(rule.clone());
                        }
                    }
                }
            }
        }
        // A site with only exceptions needs no script.
        by_site.retain(|_, v| v.iter().any(|r| !r.exception));
        MediaRules { by_site }
    }

    pub fn is_empty(&self) -> bool {
        self.by_site.is_empty()
    }

    /// The rules for `host` and its parent domains.
    fn for_host(&self, host: &str) -> Vec<&Rule> {
        let mut out = Vec::new();
        let mut h = host.trim_start_matches("www.");
        loop {
            if let Some(v) = self.by_site.get(h) {
                out.extend(v.iter());
            }
            match h.split_once('.') {
                Some((_, rest)) if rest.contains('.') => h = rest,
                _ => break,
            }
        }
        out
    }

    /// The page-world script for `host`, or `None` when no filter names it.
    pub fn script_for(&self, host: &str) -> Option<String> {
        let rules = self.for_host(host);
        if !rules.iter().any(|r| !r.exception) {
            return None;
        }
        let list: Vec<String> = rules.iter().map(|r| format!("[{},{}]", js_regex(&r.re), r.exception)).collect();
        Some(SCRIPT.replace("__RULES__", &list.join(",")))
    }
}

/// A filter line → (sites, rule), for `$media` filters that redirect (or
/// exceptions for media) and name the sites they apply on.
fn parse_line(line: &str) -> Option<(Vec<String>, Rule)> {
    if line.is_empty() || line.starts_with('!') || line.starts_with('[') || line.contains("##") || line.contains("#@#") || line.contains("#?#") || line.contains("#$#") {
        return None;
    }
    let (exception, body) = match line.strip_prefix("@@") {
        Some(b) => (true, b),
        None => (false, line),
    };
    // Options follow the last `$` (a regex filter `/…/` keeps its own `$`).
    let (pattern, opts) = if body.starts_with('/') && body[1..].contains("/$") {
        let end = body.rfind("/$")?;
        (&body[..=end], &body[end + 2..])
    } else {
        let i = body.rfind('$')?;
        (&body[..i], &body[i + 1..])
    };
    let mut media = false;
    let mut redirect = false;
    let mut sites = Vec::new();
    for o in opts.split(',') {
        let o = o.trim();
        if o == "media" {
            media = true;
        } else if let Some(v) = o.strip_prefix("redirect=") {
            // Only the silent clips uBlock uses for players.
            redirect = v.starts_with("noop-") && (v.contains(".mp4") || v.contains(".mp3") || v.contains(".wav"));
        } else if let Some(v) = o.strip_prefix("domain=").or_else(|| o.strip_prefix("from=")) {
            sites.extend(v.split('|').filter(|d| !d.starts_with('~') && !d.contains('*') && !d.is_empty()).map(|d| d.trim_start_matches("www.").to_ascii_lowercase()));
        } else if o.starts_with("redirect-rule") || o == "~media" {
            // redirect-rule only applies to requests something else blocks.
            return None;
        }
    }
    if !media || sites.is_empty() || !(redirect || exception) {
        return None;
    }
    Some((sites, Rule { re: pattern_to_regex(pattern)?, exception }))
}

/// An ABP/uBO network pattern as a regex source.
fn pattern_to_regex(p: &str) -> Option<String> {
    if p.len() > 2 && p.starts_with('/') && p.ends_with('/') {
        return Some(p[1..p.len() - 1].to_string());
    }
    if p.is_empty() || p == "*" {
        return Some(String::new());
    }
    let mut re = String::new();
    let mut rest = p;
    if let Some(r) = rest.strip_prefix("||") {
        re.push_str(r"^[a-z][a-z0-9+.-]*:\/\/(?:[^\/?#]*\.)?");
        rest = r;
    } else if let Some(r) = rest.strip_prefix('|') {
        re.push('^');
        rest = r;
    }
    let (rest, end) = match rest.strip_suffix('|') {
        Some(r) => (r, true),
        None => (rest, false),
    };
    for c in rest.chars() {
        match c {
            '*' => re.push_str(".*"),
            '^' => re.push_str(r"(?:[^\w.%-]|$)"),
            c if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '%' => re.push(c),
            '/' => re.push_str(r"\/"),
            c => {
                re.push('\\');
                re.push(c);
            }
        }
    }
    if end {
        re.push('$');
    }
    Some(re)
}

/// A regex source as a JS regex literal (case-insensitive, as filters are).
fn js_regex(src: &str) -> String {
    if src.is_empty() {
        return "/(?:)/i".into();
    }
    let mut s = String::with_capacity(src.len() + 4);
    s.push('/');
    for c in src.chars() {
        if c == '\n' || c == '\r' {
            continue;
        }
        s.push(c);
    }
    s.push_str("/i");
    s
}

const SCRIPT: &str = r#"
(() => {
  if (window.__sericaMediaRedirect) return;
  Object.defineProperty(window, '__sericaMediaRedirect', { value: true });
  let rules;
  try { rules = [__RULES__]; } catch (e) { return; }
  // One second of silence (8 kHz, 8-bit mono WAV), as uBlock's noop clips.
  const silent = (() => {
    const n = 8000, b = new Uint8Array(44 + n), v = new DataView(b.buffer);
    const w = (o, s) => { for (let i = 0; i < s.length; i++) b[o + i] = s.charCodeAt(i); };
    w(0, 'RIFF'); v.setUint32(4, 36 + n, true); w(8, 'WAVEfmt '); v.setUint32(16, 16, true);
    v.setUint16(20, 1, true); v.setUint16(22, 1, true); v.setUint32(24, 8000, true); v.setUint32(28, 8000, true);
    v.setUint16(32, 1, true); v.setUint16(34, 8, true); w(36, 'data'); v.setUint32(40, n, true); b.fill(128, 44);
    let s = ''; for (let i = 0; i < b.length; i += 4096) s += String.fromCharCode.apply(null, b.subarray(i, i + 4096));
    return 'data:audio/wav;base64,' + btoa(s);
  })();
  const swap = (u) => {
    if (typeof u !== 'string' && !(u instanceof URL)) return u;
    let href;
    try { href = new URL(String(u), location.href).href; } catch (e) { return u; }
    if (!/^https?:/.test(href)) return u;
    let hit = false;
    for (const [re, exception] of rules) {
      if (re.test(href)) { if (exception) return u; hit = true; }
    }
    return hit ? silent : u;
  };
  const hook = (proto, prop) => {
    const d = proto && Object.getOwnPropertyDescriptor(proto, prop);
    if (!d || !d.set) return;
    Object.defineProperty(proto, prop, { ...d, set(v) { return d.set.call(this, swap(v)); } });
  };
  hook(HTMLMediaElement.prototype, 'src');
  hook(HTMLSourceElement.prototype, 'src');
  const setAttr = Element.prototype.setAttribute;
  Element.prototype.setAttribute = function (name, value) {
    if ((this instanceof HTMLMediaElement || this instanceof HTMLSourceElement) && String(name).toLowerCase() === 'src') value = swap(value);
    return setAttr.call(this, name, value);
  };
  const NativeAudio = window.Audio;
  if (NativeAudio) {
    const Audio = function Audio(src) { return arguments.length ? new NativeAudio(swap(src)) : new NativeAudio(); };
    Audio.prototype = NativeAudio.prototype;
    window.Audio = Audio;
  }
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"
! comment
||scdn.co/mp3-ad/$media,redirect=noop-1s.mp4:10,domain=open.spotify.com
||spotify.com/ad-logic/$media,redirect=noop-1s.mp4:10,domain=open.spotify.com
*$media,3p,redirect-rule=noop-1s.mp4:10,domain=open.spotify.com
/traffic.megaphone.fm/*.mp3^$media,domain=open.spotify.com,urlskip=/x/ +https
@@/traffic.megaphone.fm/*.mp3^$media,domain=open.spotify.com
||akamaized.net/audio/$media,redirect=noop-1s.mp4:10,from=open.spotify.com
||example.com/ads.js$script,redirect=noop.js,domain=example.org
open.spotify.com###leaderboard-ad-element
"#;

    #[test]
    fn spotify_filters_become_a_script() {
        let m = MediaRules::parse([LIST]);
        let rules = m.for_host("open.spotify.com");
        assert_eq!(rules.iter().filter(|r| !r.exception).count(), 3, "{rules:?}");
        assert_eq!(rules.iter().filter(|r| r.exception).count(), 1);
        let s = m.script_for("open.spotify.com").unwrap();
        assert!(s.contains("scdn\\.co\\/mp3-ad\\/"), "{s}");
        // Script redirects and cosmetic filters are not media redirects.
        assert!(m.script_for("example.org").is_none());
        assert!(m.script_for("www.example.net").is_none());
    }

    #[test]
    fn parent_domains_apply() {
        let m = MediaRules::parse(["||ads.test/a.mp3$media,redirect=noop-1s.mp4,domain=music.test"]);
        assert!(m.script_for("play.music.test").is_some());
        assert!(m.script_for("music.test").is_some());
        assert!(m.script_for("test").is_none());
    }

    #[test]
    fn patterns() {
        assert_eq!(pattern_to_regex("||a.com/x^").unwrap(), r"^[a-z][a-z0-9+.-]*:\/\/(?:[^\/?#]*\.)?a\.com\/x(?:[^\w.%-]|$)");
        assert_eq!(pattern_to_regex("/x/*.mp3|").unwrap(), r"\/x\/.*\.mp3$");
        assert_eq!(pattern_to_regex("/^https:\\/\\/a/").unwrap(), r"^https:\/\/a");
    }
}
