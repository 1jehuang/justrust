//! `justrust mcp`: a Model Context Protocol server on stdio for the Jcode
//! account behind hosted builds.
//!
//! ```text
//! claude mcp add justrust -- justrust mcp
//! ```
//!
//! Tools: account_status, login, list_api_keys, create_api_key,
//! revoke_api_key, billing_link. Building is not a tool: agents already have
//! a shell and `justrust check|test` prints better output than a tool call
//! would. Where a build runs is chosen per command with `--local`,
//! `--remote`, or `--auto`, so there is no routing tool either. Anything that
//! costs money (subscribing, plan changes, raising the monthly limit) only
//! returns a link for the human: an agent must not raise its own spending.
//!
//! Transport: newline-delimited JSON-RPC 2.0 on stdin/stdout. Nothing else
//! is written to stdout.

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

use crate::remote_hosted as hosted;

const PROTOCOL_VERSION: &str = "2025-06-18";
const ACCOUNT_URL: &str = "https://jcode.sh/account";
const PRICING_URL: &str = "https://jcode.sh/rust#subscribe";

/// The account operations the tools need. Split out so the protocol is
/// tested without the network.
pub trait Api {
    fn status(&self) -> Result<Value>;
    fn login_link(&self) -> Result<Option<String>>;
    fn list_keys(&self) -> Result<Value>;
    fn create_key(&self) -> Result<Value>;
    fn revoke_key(&self, key_id: &str) -> Result<()>;
}

struct Live;

impl Api for Live {
    fn status(&self) -> Result<Value> {
        let s = hosted::status();
        let signed_in = hosted::signed_in();
        let backend = crate::remote_backend::load();
        let route = match (&backend, signed_in) {
            (Some(crate::remote_backend::Backend::Hosted), true) => {
                "remote (hosted builds)".to_string()
            }
            (Some(crate::remote_backend::Backend::Hosted), false) | (None, _) => {
                "local until signed in (use the login tool)".to_string()
            }
            (Some(b), _) => format!(
                "router: remote on {} only when predicted faster (own machine configured; `justrust remote use hosted` switches to hosted builds)",
                b.kind()
            ),
        };
        let builds = s
            .builds
            .as_ref()
            .or(s.me.as_ref().and_then(|m| m.builds.as_ref()));
        Ok(json!({
            "signed_in": signed_in,
            "email": s.me.as_ref().and_then(|m| m.email.clone()),
            "tier": s.me.as_ref().and_then(|m| m.tier.clone()),
            "builds": builds,
            "builds_summary": builds.map(|b| b.describe()),
            "host_state": s.host_state,
            "default_route": format!("{route}. Per command: --local, --remote, or --auto (router)"),
            "error": s.error,
            "manage": ACCOUNT_URL,
        }))
    }

    fn login_link(&self) -> Result<Option<String>> {
        crate::account::login_link()
    }

    fn list_keys(&self) -> Result<Value> {
        let c = hosted::creds()?;
        hosted::ok(hosted::request(&c, "GET", "keys", None)?)
    }

    fn create_key(&self) -> Result<Value> {
        let c = hosted::creds()?;
        hosted::ok(hosted::request(&c, "POST", "keys", Some(&json!({})))?)
    }

    fn revoke_key(&self, key_id: &str) -> Result<()> {
        let c = hosted::creds()?;
        hosted::ok(hosted::request(
            &c,
            "DELETE",
            &format!("keys/{key_id}"),
            None,
        )?)?;
        Ok(())
    }
}

fn tools() -> Value {
    let none = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    json!([
        {
            "name": "account_status",
            "description": "Jcode account behind justrust hosted builds: signed in or not, builds left this month, plan, hosted machine state, and where builds run by default. Read-only.",
            "inputSchema": none,
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "login",
            "description": "Sign in for hosted Rust builds. Returns a link: give it to the user to open and approve. Remote builds then need a card on file: the free first month ($20/mo after) starts with the `upgrade` link. Does not wait; any later justrust command finishes the sign-in once approved.",
            "inputSchema": none,
        },
        {
            "name": "list_api_keys",
            "description": "List the account's active API keys (ids and creation dates, never the secrets). `current` marks the key this machine uses.",
            "inputSchema": none,
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "create_api_key",
            "description": "Create a new account API key, for CI or another machine (set it as JCODE_API_KEY there). The secret is shown only in this result: hand it to the user or a secret store, never write it into the repository.",
            "inputSchema": none,
        },
        {
            "name": "revoke_api_key",
            "description": "Revoke one API key by key_id (from list_api_keys). Refuses the key this machine uses.",
            "inputSchema": {
                "type": "object",
                "properties": { "key_id": { "type": "string" } },
                "required": ["key_id"],
                "additionalProperties": false,
            },
            "annotations": { "destructiveHint": true },
        },
        {
            "name": "billing_link",
            "description": "Links for the user to subscribe, change plan, or raise the monthly limit. Agents cannot change billing: give the link to the user.",
            "inputSchema": none,
            "annotations": { "readOnlyHint": true },
        },
    ])
}

fn text(v: impl Into<String>, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": v.into() }], "isError": is_error })
}

fn call_tool(api: &dyn Api, name: &str, args: &Value) -> Value {
    let r: Result<String> = (|| {
        match name {
        "account_status" => Ok(serde_json::to_string_pretty(&api.status()?)?),
        "login" => Ok(match api.login_link()? {
            None => "Already signed in. Hosted builds are on: justrust check/test run remote by default.".into(),
            Some(url) => format!(
                "Ask the user to open this link and approve: {url}\nKeep working meanwhile; builds stay local until it is approved."
            ),
        }),
        "list_api_keys" => Ok(serde_json::to_string_pretty(&api.list_keys()?)?),
        "create_api_key" => {
            let k = api.create_key()?;
            let key = k["api_key"].as_str().ok_or_else(|| anyhow!("the Jcode API returned no key"))?;
            Ok(format!(
                "New API key (shown once): {key}\nkey_id: {}\nUse it as JCODE_API_KEY in CI or on another machine. Do not commit it.",
                k["key_id"].as_str().unwrap_or("?")
            ))
        }
        "revoke_api_key" => {
            let id = args["key_id"].as_str().unwrap_or("").trim();
            if id.is_empty() || id.contains(['/', '?', '#', ' ']) {
                return Err(anyhow!("key_id is required (see list_api_keys)"));
            }
            let keys = api.list_keys()?;
            let current = keys["keys"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|k| k["key_id"] == id && k["current"] == true);
            if current {
                return Err(anyhow!(
                    "that is the key this machine uses; revoking it would sign justrust out here. Use `justrust logout` instead"
                ));
            }
            api.revoke_key(id)?;
            Ok(format!("revoked {id}"))
        }
        "billing_link" => Ok(format!(
            "Give the user these links (billing needs the human):\n- subscribe or change plan: {PRICING_URL}\n- monthly limit, payment details, API keys: {ACCOUNT_URL}"
        )),
        other => Err(anyhow!("unknown tool {other}")),
    }
    })();
    match r {
        Ok(s) => text(s, false),
        Err(e) => text(format!("{e:#}"), true),
    }
}

/// One JSON-RPC message in, at most one response out (none for
/// notifications).
pub fn handle(api: &dyn Api, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg["method"].as_str().unwrap_or("");
    let result = match method {
        "initialize" => json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "justrust", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Jcode account for justrust hosted Rust builds. Build with `justrust check|test` in the shell (cargo's arguments; --local, --remote, or --auto to choose where). Use these tools for account state, sign-in, and API keys. Billing changes are links for the user.",
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call_tool(
            api,
            msg["params"]["name"].as_str().unwrap_or(""),
            &msg["params"]["arguments"],
        ),
        _ => {
            return Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("method not found: {method}") },
            }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&Live, &msg),
            Err(e) => Some(json!({
                "jsonrpc": "2.0", "id": null,
                "error": { "code": -32700, "message": format!("parse error: {e}") },
            })),
        };
        if let Some(r) = resp {
            writeln!(out, "{r}")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        revoked: RefCell<Vec<String>>,
        signed_in: bool,
    }

    impl Api for Fake {
        fn status(&self) -> Result<Value> {
            Ok(json!({ "signed_in": self.signed_in }))
        }
        fn login_link(&self) -> Result<Option<String>> {
            Ok((!self.signed_in).then(|| "https://jcode.sh/account?flow=x".to_string()))
        }
        fn list_keys(&self) -> Result<Value> {
            Ok(json!({ "keys": [
                { "key_id": "k1", "current": true },
                { "key_id": "k2", "current": false },
            ]}))
        }
        fn create_key(&self) -> Result<Value> {
            Ok(json!({ "api_key": "jck_live_secret", "key_id": "k3" }))
        }
        fn revoke_key(&self, key_id: &str) -> Result<()> {
            self.revoked.borrow_mut().push(key_id.into());
            Ok(())
        }
    }

    fn call(api: &Fake, name: &str, args: Value) -> (String, bool) {
        let r = handle(
            api,
            &json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                     "params": { "name": name, "arguments": args } }),
        )
        .unwrap();
        assert_eq!(r["id"], 7);
        (
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string(),
            r["result"]["isError"].as_bool().unwrap(),
        )
    }

    #[test]
    fn initialize_and_list() {
        let f = Fake::default();
        let r = handle(
            &f,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-03-26" } }),
        )
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "justrust");
        assert!(
            handle(
                &f,
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
            )
            .is_none()
        );
        let l = handle(
            &f,
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        )
        .unwrap();
        let names: Vec<_> = l["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "account_status",
                "login",
                "list_api_keys",
                "create_api_key",
                "revoke_api_key",
                "billing_link"
            ]
        );
        let u = handle(&f, &json!({ "jsonrpc": "2.0", "id": 3, "method": "nope" })).unwrap();
        assert_eq!(u["error"]["code"], -32601);
    }

    #[test]
    fn login_returns_link_or_already_signed_in() {
        let (t, err) = call(&Fake::default(), "login", json!({}));
        assert!(!err && t.contains("https://jcode.sh/account?flow=x"), "{t}");
        let (t, _) = call(
            &Fake {
                signed_in: true,
                ..Default::default()
            },
            "login",
            json!({}),
        );
        assert!(t.starts_with("Already signed in"), "{t}");
    }

    #[test]
    fn revoke_refuses_current_key_and_bad_ids() {
        let f = Fake::default();
        let (t, err) = call(&f, "revoke_api_key", json!({ "key_id": "k1" }));
        assert!(err && t.contains("justrust logout"), "{t}");
        let (_, err) = call(&f, "revoke_api_key", json!({ "key_id": "../me" }));
        assert!(err);
        let (_, err) = call(&f, "revoke_api_key", json!({}));
        assert!(err);
        let (t, err) = call(&f, "revoke_api_key", json!({ "key_id": "k2" }));
        assert!(!err, "{t}");
        assert_eq!(*f.revoked.borrow(), ["k2"]);
    }

    #[test]
    fn create_key_shows_secret_once_and_billing_is_links_only() {
        let (t, err) = call(&Fake::default(), "create_api_key", json!({}));
        assert!(
            !err && t.contains("jck_live_secret") && t.contains("JCODE_API_KEY"),
            "{t}"
        );
        let (t, err) = call(&Fake::default(), "billing_link", json!({}));
        assert!(!err && t.contains(PRICING_URL) && t.contains(ACCOUNT_URL));
        let (_, err) = call(&Fake::default(), "set_remote", json!({}));
        assert!(err);
    }
}
