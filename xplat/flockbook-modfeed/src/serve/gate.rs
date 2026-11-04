//! Who may use `serve`: every request passes `check` before anything else. Pure, so tests cover every rule.
//!
//! The helper listens on 127.0.0.1 only and is reached through Tailscale Serve, which adds the caller's identity
//! (`Tailscale-User-Login`) and removes any copy the caller sent. So:
//! - No identity header: refused (a request that did not come through Serve, or a tagged node).
//! - Another account than the configured one: refused.
//! - A Funnel request (`Tailscale-Funnel-Request`; Serve sends no identity with it): refused.
//! - The socket and the API (they read the agents and send keys): the page's `Origin` must be this host, so a page
//!   from another site open in the owner's browser cannot use the owner's identity (CSRF, socket hijacking).
//! - The API is `POST` with a JSON body only (a cross-site form cannot send one without a preflight we never allow).
//! - `Sec-Fetch-Site`, when the browser sends it, must be `same-origin`; without it (older browsers) the Origin rule
//!   alone decides.
//!
//! Limit: any program on this machine, of any user (loopback is shared), can connect to 127.0.0.1 and send any
//! header. For this user's own programs that is the trust they already have (they can run hc); other users of the
//! machine are not kept out (SERVE.md).

/// What a request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A file of the page (GET, HEAD).
    Page,
    /// The WebSocket at `/ws` (reads; keys go through `Api`).
    Socket,
    /// `POST /api`: approve, answer, send.
    Api,
}

/// Why a request was refused (the 403 body says it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deny {
    Funnel,
    NoIdentity,
    WrongUser,
    NoOrigin,
    WrongOrigin,
    CrossSite,
    Method,
    NotJson,
}

impl Deny {
    pub fn message(self) -> &'static str {
        match self {
            Deny::Funnel => "Refused: this came through Tailscale Funnel. Use Tailscale Serve (tailnet only).",
            Deny::NoIdentity => "Refused: no Tailscale identity. Open this page through Tailscale Serve.",
            Deny::WrongUser => "Refused: this Tailscale account may not use this helper.",
            Deny::NoOrigin => "Refused: no Origin header.",
            Deny::WrongOrigin => "Refused: the page is from another origin.",
            Deny::CrossSite => "Refused: a cross-site request.",
            Deny::Method => "Refused: keys go only with POST.",
            Deny::NotJson => "Refused: the body must be application/json.",
        }
    }
}

/// The rules (see the module doc). `user`: the one Tailscale login allowed. `origin`: an extra page origin to
/// accept (`https://name.tailnet.ts.net`), for a proxy that rewrites Host; by default the Origin must name the Host.
pub fn check(method: &str, headers: &[(String, String)], kind: Kind, user: &str, origin: Option<&str>) -> Result<(), Deny> {
    let all = |name: &str| -> Vec<&str> { headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()).collect() };
    // Any variant Serve would have removed (`_` for `-`) counts as the header itself.
    let named = |name: &str| headers.iter().any(|(k, _)| k.replace('_', "-").eq_ignore_ascii_case(name));
    if named("tailscale-funnel-request") {
        return Err(Deny::Funnel);
    }
    let logins = all("tailscale-user-login");
    match logins.as_slice() {
        [] | [""] => return Err(Deny::NoIdentity),
        [one] if !user.is_empty() && one.eq_ignore_ascii_case(user.trim()) => {}
        _ => return Err(Deny::WrongUser),
    }
    if kind == Kind::Page {
        return Ok(());
    }
    if kind == Kind::Api && method != "POST" {
        return Err(Deny::Method);
    }
    if all("sec-fetch-site").iter().any(|s| !s.eq_ignore_ascii_case("same-origin")) {
        return Err(Deny::CrossSite);
    }
    let origins = all("origin");
    let page = match origins.as_slice() {
        [] => return Err(Deny::NoOrigin),
        [one] => *one,
        _ => return Err(Deny::WrongOrigin),
    };
    // Serve is HTTPS: the page's origin is `https://<authority>`, nothing after it.
    let Some(authority) = page.strip_prefix("https://").filter(|a| !a.is_empty() && !a.contains('/')) else {
        return Err(Deny::WrongOrigin);
    };
    let hosts: Vec<&str> = all("host").into_iter().chain(all("x-forwarded-host")).collect();
    let configured = origin.is_some_and(|o| o.trim_end_matches('/').eq_ignore_ascii_case(page));
    if !configured && !hosts.iter().any(|h| !h.is_empty() && h.eq_ignore_ascii_case(authority)) {
        return Err(Deny::WrongOrigin);
    }
    if kind == Kind::Api {
        let json = all("content-type")
            .first()
            .is_some_and(|t| t.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json"));
        if !json {
            return Err(Deny::NotJson);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "owner@example.com";
    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }
    fn ok_page() -> Vec<(String, String)> {
        h(&[("Host", "mac.tail1.ts.net"), ("Tailscale-User-Login", ME)])
    }
    fn ok_api() -> Vec<(String, String)> {
        h(&[("Host", "mac.tail1.ts.net"), ("Tailscale-User-Login", ME), ("Origin", "https://mac.tail1.ts.net"), ("Content-Type", "application/json")])
    }

    #[test]
    fn identity() {
        assert_eq!(check("GET", &ok_page(), Kind::Page, ME, None), Ok(()));
        assert_eq!(check("GET", &h(&[("Host", "mac.tail1.ts.net")]), Kind::Page, ME, None), Err(Deny::NoIdentity));
        assert_eq!(check("GET", &h(&[("Host", "x"), ("Tailscale-User-Login", "")]), Kind::Page, ME, None), Err(Deny::NoIdentity));
        assert_eq!(check("GET", &h(&[("Host", "x"), ("Tailscale-User-Login", "other@example.com")]), Kind::Page, ME, None), Err(Deny::WrongUser));
        // Header names are case-insensitive; the login compares ASCII case-insensitively (email addresses).
        assert_eq!(check("GET", &h(&[("host", "x"), ("tailscale-user-login", "Owner@Example.com")]), Kind::Page, ME, None), Ok(()));
        // Two identities: refused (only one can be Serve's).
        let two = h(&[("Host", "x"), ("Tailscale-User-Login", ME), ("Tailscale-User-Login", "other@example.com")]);
        assert_eq!(check("GET", &two, Kind::Page, ME, None), Err(Deny::WrongUser));
        // A prefix or a longer name is another account.
        assert_eq!(check("GET", &h(&[("Host", "x"), ("Tailscale-User-Login", "owner@example.com.evil")]), Kind::Page, ME, None), Err(Deny::WrongUser));
        // No configured account never lets anyone in.
        assert_eq!(check("GET", &ok_page(), Kind::Page, "", None), Err(Deny::WrongUser));
    }

    #[test]
    fn funnel() {
        let mut f = ok_page();
        f.push(("Tailscale-Funnel-Request".into(), "?1".into()));
        assert_eq!(check("GET", &f, Kind::Page, ME, None), Err(Deny::Funnel));
        // Funnel sends no identity: still refused as Funnel, whatever value.
        assert_eq!(check("GET", &h(&[("Host", "x"), ("tailscale-funnel-request", "")]), Kind::Page, ME, None), Err(Deny::Funnel));
    }

    #[test]
    fn origin() {
        assert_eq!(check("POST", &ok_api(), Kind::Api, ME, None), Ok(()));
        let mut ws = ok_page();
        ws.push(("Origin".into(), "https://mac.tail1.ts.net".into()));
        assert_eq!(check("GET", &ws, Kind::Socket, ME, None), Ok(()));
        // The socket and the API need an Origin naming this host.
        assert_eq!(check("GET", &ok_page(), Kind::Socket, ME, None), Err(Deny::NoOrigin));
        let no = h(&[("Host", "mac.tail1.ts.net"), ("Tailscale-User-Login", ME), ("Content-Type", "application/json")]);
        assert_eq!(check("POST", &no, Kind::Api, ME, None), Err(Deny::NoOrigin));
        for bad in ["https://evil.example", "http://mac.tail1.ts.net", "https://mac.tail1.ts.net.evil", "null", "https://mac.tail1.ts.net/x", ""] {
            let mut r = no.clone();
            r.push(("Origin".into(), bad.into()));
            assert_eq!(check("POST", &r, Kind::Api, ME, None), Err(Deny::WrongOrigin), "{bad}");
        }
        // Port and case: the authority as the Host header has it.
        let p = h(&[("Host", "mac.tail1.ts.net:8443"), ("Tailscale-User-Login", ME), ("Origin", "https://MAC.tail1.ts.net:8443"), ("Content-Type", "application/json")]);
        assert_eq!(check("POST", &p, Kind::Api, ME, None), Ok(()));
        // Serve's X-Forwarded-Host counts when Host was rewritten; a configured origin too.
        let fwd = h(&[("Host", "127.0.0.1:8787"), ("X-Forwarded-Host", "mac.tail1.ts.net"), ("Tailscale-User-Login", ME), ("Origin", "https://mac.tail1.ts.net"), ("Content-Type", "application/json")]);
        assert_eq!(check("POST", &fwd, Kind::Api, ME, None), Ok(()));
        let cfg = h(&[("Host", "127.0.0.1:8787"), ("Tailscale-User-Login", ME), ("Origin", "https://mac.tail1.ts.net"), ("Content-Type", "application/json")]);
        assert_eq!(check("POST", &cfg, Kind::Api, ME, None), Err(Deny::WrongOrigin));
        assert_eq!(check("POST", &cfg, Kind::Api, ME, Some("https://mac.tail1.ts.net")), Ok(()));
        // Two Origins: refused.
        let mut two = ok_api();
        two.push(("Origin".into(), "https://evil.example".into()));
        assert_eq!(check("POST", &two, Kind::Api, ME, None), Err(Deny::WrongOrigin));
        // The browser says the request is cross-site: refused even with a matching Origin.
        let mut cs = ok_api();
        cs.push(("Sec-Fetch-Site".into(), "cross-site".into()));
        assert_eq!(check("POST", &cs, Kind::Api, ME, None), Err(Deny::CrossSite));
        let mut so = ok_api();
        so.push(("Sec-Fetch-Site".into(), "same-origin".into()));
        assert_eq!(check("POST", &so, Kind::Api, ME, None), Ok(()));
    }

    #[test]
    fn api_is_post_json() {
        assert_eq!(check("GET", &ok_api(), Kind::Api, ME, None), Err(Deny::Method));
        let mut form = ok_api();
        form.retain(|(k, _)| k != "Content-Type");
        assert_eq!(check("POST", &form, Kind::Api, ME, None), Err(Deny::NotJson));
        form.push(("Content-Type".into(), "text/plain".into()));
        assert_eq!(check("POST", &form, Kind::Api, ME, None), Err(Deny::NotJson));
        let mut cs = ok_api();
        cs.retain(|(k, _)| k != "Content-Type");
        cs.push(("Content-Type".into(), "application/json; charset=utf-8".into()));
        assert_eq!(check("POST", &cs, Kind::Api, ME, None), Ok(()));
        // Identity comes first: a Funnel or foreign POST is refused for that reason.
        let mut f = ok_api();
        f.push(("Tailscale-Funnel-Request".into(), "?1".into()));
        assert_eq!(check("POST", &f, Kind::Api, ME, None), Err(Deny::Funnel));
    }
}
