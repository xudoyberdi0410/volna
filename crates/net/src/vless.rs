//! `vless://` share links and the xray config generated from them.
//!
//! Link format (the de-facto standard used by Xray clients and 3x-ui):
//!
//! ```text
//! vless://<uuid>@<host>:<port>?type=xhttp&security=reality&pbk=..&sid=..&sni=..&fp=chrome
//!        &path=%2F..&host=..&mode=auto&flow=..&encryption=none#<name>
//! ```

use std::collections::HashMap;

use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};
use thiserror::Error;

/// Why a link could not be understood.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LinkError {
    #[error("link must start with vless://")]
    NotVless,
    #[error("missing user id (the part before @)")]
    MissingId,
    #[error("user id is neither a UUID nor a 1–30 character string")]
    BadId,
    #[error("missing host")]
    MissingHost,
    #[error("missing or invalid port")]
    BadPort,
    #[error("unsupported transport type `{0}`")]
    UnsupportedTransport(String),
    #[error("unsupported security `{0}`")]
    UnsupportedSecurity(String),
    #[error("REALITY needs a public key (pbk)")]
    MissingPublicKey,
    #[error("xhttp `extra` is not a JSON object")]
    BadExtra,
}

/// A parsed `vless://` link.
#[derive(Debug, Clone, PartialEq)]
pub struct VlessLink {
    pub id: String,
    pub address: String,
    pub port: u16,
    /// Fragment of the link, usually a human label.
    pub name: Option<String>,
    /// `none`, or an ML-KEM encryption string from newer Xray servers.
    pub encryption: String,
    pub flow: Option<String>,
    pub transport: Transport,
    pub security: Security,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    /// `tcp` (alias `raw`). `header_type=http` is passed through as-is.
    Tcp {
        header_type: Option<String>,
    },
    Ws {
        path: String,
        host: Option<String>,
    },
    Grpc {
        service_name: String,
        authority: Option<String>,
        multi_mode: bool,
    },
    Xhttp {
        path: String,
        host: Option<String>,
        mode: String,
        /// `extra` query parameter: raw JSON merged into `xhttpSettings.extra`.
        extra: Option<Value>,
    },
    HttpUpgrade {
        path: String,
        host: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Security {
    None,
    Tls {
        server_name: Option<String>,
        fingerprint: Option<String>,
        alpn: Vec<String>,
        allow_insecure: bool,
    },
    Reality {
        server_name: Option<String>,
        fingerprint: String,
        public_key: String,
        short_id: String,
        spider_x: Option<String>,
    },
}

/// Local SOCKS inbound that xray exposes for the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocksInbound {
    pub port: u16,
    pub user: String,
    pub pass: String,
}

impl VlessLink {
    pub fn parse(link: &str) -> Result<Self, LinkError> {
        let rest = link
            .trim()
            .strip_prefix("vless://")
            .ok_or(LinkError::NotVless)?;

        let (rest, name) = match rest.split_once('#') {
            Some((r, frag)) => (r, Some(decode(frag)).filter(|s| !s.is_empty())),
            None => (rest, None),
        };
        let (authority, query) = rest.split_once('?').unwrap_or((rest, ""));
        let authority = authority.trim_end_matches('/');

        let (id, hostport) = authority.rsplit_once('@').ok_or(LinkError::MissingId)?;
        let id = decode(id);
        validate_id(&id)?;

        let (address, port) = split_host_port(hostport)?;
        let params = parse_query(query);
        let get = |key: &str| params.get(key).filter(|v| !v.is_empty()).cloned();

        let transport = parse_transport(&get)?;
        let security = parse_security(&get)?;

        Ok(VlessLink {
            id,
            address,
            port,
            name,
            encryption: get("encryption").unwrap_or_else(|| "none".into()),
            flow: get("flow"),
            transport,
            security,
        })
    }

    /// The xray outbound object (`tag: "proxy"`) for this link.
    pub fn outbound(&self) -> Value {
        let mut user = Map::new();
        user.insert("id".into(), json!(self.id));
        user.insert("encryption".into(), json!(self.encryption));
        if let Some(flow) = &self.flow {
            user.insert("flow".into(), json!(flow));
        }

        let mut stream = Map::new();
        let (network, key, settings) = self.transport.stream_settings();
        stream.insert("network".into(), json!(network));
        if let Some(key) = key {
            stream.insert(key.into(), settings);
        }
        match &self.security {
            Security::None => {
                stream.insert("security".into(), json!("none"));
            }
            Security::Tls {
                server_name,
                fingerprint,
                alpn,
                allow_insecure,
            } => {
                let mut tls = Map::new();
                tls.insert(
                    "serverName".into(),
                    json!(server_name.as_deref().unwrap_or(&self.address)),
                );
                if let Some(fp) = fingerprint {
                    tls.insert("fingerprint".into(), json!(fp));
                }
                if !alpn.is_empty() {
                    tls.insert("alpn".into(), json!(alpn));
                }
                if *allow_insecure {
                    tls.insert("allowInsecure".into(), json!(true));
                }
                stream.insert("security".into(), json!("tls"));
                stream.insert("tlsSettings".into(), Value::Object(tls));
            }
            Security::Reality {
                server_name,
                fingerprint,
                public_key,
                short_id,
                spider_x,
            } => {
                let mut reality = Map::new();
                reality.insert(
                    "serverName".into(),
                    json!(server_name.as_deref().unwrap_or(&self.address)),
                );
                reality.insert("fingerprint".into(), json!(fingerprint));
                reality.insert("publicKey".into(), json!(public_key));
                reality.insert("shortId".into(), json!(short_id));
                if let Some(spx) = spider_x {
                    reality.insert("spiderX".into(), json!(spx));
                }
                stream.insert("security".into(), json!("reality"));
                stream.insert("realitySettings".into(), Value::Object(reality));
            }
        }

        json!({
            "tag": "proxy",
            "protocol": "vless",
            "settings": {
                "vnext": [{
                    "address": self.address,
                    "port": self.port,
                    "users": [Value::Object(user)],
                }]
            },
            "streamSettings": Value::Object(stream),
        })
    }

    /// A complete xray config: one password-protected SOCKS inbound on
    /// 127.0.0.1 and this link as the only outbound.
    pub fn xray_config(&self, inbound: &SocksInbound) -> Value {
        json!({
            "log": { "loglevel": "warning" },
            "inbounds": [{
                "tag": "socks-in",
                "listen": "127.0.0.1",
                "port": inbound.port,
                "protocol": "socks",
                "settings": {
                    "auth": "password",
                    "accounts": [{ "user": inbound.user, "pass": inbound.pass }],
                    "udp": true,
                },
            }],
            "outbounds": [self.outbound()],
        })
    }
}

impl Transport {
    /// `(network, settings key, settings)` for `streamSettings`.
    fn stream_settings(&self) -> (&'static str, Option<&'static str>, Value) {
        fn path_host(path: &str, host: &Option<String>) -> Map<String, Value> {
            let mut m = Map::new();
            m.insert("path".into(), json!(path));
            if let Some(h) = host {
                m.insert("host".into(), json!(h));
            }
            m
        }

        match self {
            Transport::Tcp { header_type: None } => ("tcp", None, Value::Null),
            Transport::Tcp {
                header_type: Some(t),
            } => (
                "tcp",
                Some("tcpSettings"),
                json!({ "header": { "type": t } }),
            ),
            Transport::Ws { path, host } => (
                "ws",
                Some("wsSettings"),
                Value::Object(path_host(path, host)),
            ),
            Transport::HttpUpgrade { path, host } => (
                "httpupgrade",
                Some("httpupgradeSettings"),
                Value::Object(path_host(path, host)),
            ),
            Transport::Grpc {
                service_name,
                authority,
                multi_mode,
            } => {
                let mut m = Map::new();
                m.insert("serviceName".into(), json!(service_name));
                if let Some(a) = authority {
                    m.insert("authority".into(), json!(a));
                }
                if *multi_mode {
                    m.insert("multiMode".into(), json!(true));
                }
                ("grpc", Some("grpcSettings"), Value::Object(m))
            }
            Transport::Xhttp {
                path,
                host,
                mode,
                extra,
            } => {
                let mut m = path_host(path, host);
                m.insert("mode".into(), json!(mode));
                if let Some(extra) = extra {
                    m.insert("extra".into(), extra.clone());
                }
                ("xhttp", Some("xhttpSettings"), Value::Object(m))
            }
        }
    }
}

fn parse_transport(get: &impl Fn(&str) -> Option<String>) -> Result<Transport, LinkError> {
    let kind = get("type").unwrap_or_else(|| "tcp".into()).to_lowercase();
    let path = || get("path").unwrap_or_else(|| "/".into());
    Ok(match kind.as_str() {
        "tcp" | "raw" => Transport::Tcp {
            header_type: get("headerType").filter(|t| t != "none"),
        },
        "ws" => Transport::Ws {
            path: path(),
            host: get("host"),
        },
        "httpupgrade" => Transport::HttpUpgrade {
            path: path(),
            host: get("host"),
        },
        "grpc" => Transport::Grpc {
            // Older links put the service name in `path`.
            service_name: get("serviceName")
                .or_else(|| get("path"))
                .unwrap_or_default(),
            authority: get("authority"),
            multi_mode: get("mode").is_some_and(|m| m.eq_ignore_ascii_case("multi")),
        },
        "xhttp" | "splithttp" => Transport::Xhttp {
            path: path(),
            host: get("host"),
            mode: get("mode").unwrap_or_else(|| "auto".into()),
            extra: get("extra")
                .map(|raw| match serde_json::from_str::<Value>(&raw) {
                    Ok(v @ Value::Object(_)) => Ok(v),
                    _ => Err(LinkError::BadExtra),
                })
                .transpose()?,
        },
        other => return Err(LinkError::UnsupportedTransport(other.into())),
    })
}

fn parse_security(get: &impl Fn(&str) -> Option<String>) -> Result<Security, LinkError> {
    let kind = get("security")
        .unwrap_or_else(|| "none".into())
        .to_lowercase();
    Ok(match kind.as_str() {
        "none" => Security::None,
        "tls" => Security::Tls {
            server_name: get("sni").or_else(|| get("peer")),
            fingerprint: get("fp"),
            alpn: get("alpn")
                .map(|a| {
                    a.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
            allow_insecure: get("allowInsecure").is_some_and(|v| v == "1" || v == "true"),
        },
        "reality" => Security::Reality {
            server_name: get("sni"),
            fingerprint: get("fp").unwrap_or_else(|| "chrome".into()),
            public_key: get("pbk").ok_or(LinkError::MissingPublicKey)?,
            short_id: get("sid").unwrap_or_default(),
            spider_x: get("spx"),
        },
        other => return Err(LinkError::UnsupportedSecurity(other.into())),
    })
}

fn validate_id(id: &str) -> Result<(), LinkError> {
    if id.is_empty() {
        return Err(LinkError::MissingId);
    }
    if is_uuid(id) {
        return Ok(());
    }
    // Xray maps a short arbitrary string to a UUIDv5; it allows 1–30 bytes.
    if id.len() <= 30 && !id.chars().any(char::is_whitespace) {
        return Ok(());
    }
    Err(LinkError::BadId)
}

fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, len)| g.len() == len && g.chars().all(|c| c.is_ascii_hexdigit()))
}

fn split_host_port(hostport: &str) -> Result<(String, u16), LinkError> {
    let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
        // IPv6: [::1]:443
        let (host, after) = rest.split_once(']').ok_or(LinkError::MissingHost)?;
        let port = after.strip_prefix(':').ok_or(LinkError::BadPort)?;
        (host, port)
    } else {
        hostport.rsplit_once(':').ok_or(LinkError::BadPort)?
    };
    if host.is_empty() {
        return Err(LinkError::MissingHost);
    }
    let port: u16 = port.parse().map_err(|_| LinkError::BadPort)?;
    if port == 0 {
        return Err(LinkError::BadPort);
    }
    Ok((decode(host), port))
}

/// Splits a query string. `+` stays a literal plus (share links percent-encode
/// spaces), and for repeated keys the first value wins.
fn parse_query(query: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.entry(decode(k)).or_insert_with(|| decode(v));
    }
    out
}

fn decode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "92cc34c0-f305-4528-9473-28bf84c6862b";

    /// REALITY + xhttp, the shape 3x-ui exports for the Hetzner server.
    pub(crate) const REALITY_XHTTP: &str = "vless://92cc34c0-f305-4528-9473-28bf84c6862b@203.0.113.10:443\
        ?type=xhttp&encryption=none&path=%2Fapi%2Fv1&host=&mode=auto&security=reality\
        &pbk=wigF4NuMt2kUCb5_gX1rosrHLsVAjNkWx6JhM9ntJ_s&fp=chrome&sni=www.microsoft.com\
        &sid=41e126e9ee78dfbb&spx=%2F#Test%20%F0%9F%A7%AA";

    #[test]
    fn parses_reality_xhttp() {
        let link = VlessLink::parse(REALITY_XHTTP).unwrap();
        assert_eq!(link.id, UUID);
        assert_eq!(link.address, "203.0.113.10");
        assert_eq!(link.port, 443);
        assert_eq!(link.name.as_deref(), Some("Hetzner 🇩🇪"));
        assert_eq!(link.encryption, "none");
        assert_eq!(link.flow, None);
        assert_eq!(
            link.transport,
            Transport::Xhttp {
                path: "/api/v1".into(),
                host: None,
                mode: "auto".into(),
                extra: None,
            }
        );
        assert_eq!(
            link.security,
            Security::Reality {
                server_name: Some("www.microsoft.com".into()),
                fingerprint: "chrome".into(),
                public_key: "wigF4NuMt2kUCb5_gX1rosrHLsVAjNkWx6JhM9ntJ_s".into(),
                short_id: "41e126e9ee78dfbb".into(),
                spider_x: Some("/".into()),
            }
        );
    }

    #[test]
    fn parses_reality_tcp_vision() {
        let link = VlessLink::parse(&format!(
            "vless://{UUID}@example.com:8443?security=reality&pbk=KEY&sni=dl.google.com\
             &flow=xtls-rprx-vision&type=tcp&headerType=none"
        ))
        .unwrap();
        assert_eq!(link.flow.as_deref(), Some("xtls-rprx-vision"));
        assert_eq!(link.transport, Transport::Tcp { header_type: None });
        let Security::Reality {
            fingerprint,
            short_id,
            ..
        } = &link.security
        else {
            panic!("expected reality, got {:?}", link.security);
        };
        assert_eq!(fingerprint, "chrome", "fp defaults to chrome");
        assert_eq!(short_id, "");
        assert_eq!(link.name, None);
    }

    #[test]
    fn parses_tls_ws() {
        let link = VlessLink::parse(&format!(
            "vless://{UUID}@cdn.example.com:443/?type=ws&security=tls&path=%2Fws%3Fed%3D2048\
             &host=front.example.com&sni=front.example.com&alpn=h2,http%2F1.1&fp=firefox#ws"
        ))
        .unwrap();
        assert_eq!(
            link.transport,
            Transport::Ws {
                path: "/ws?ed=2048".into(),
                host: Some("front.example.com".into()),
            }
        );
        assert_eq!(
            link.security,
            Security::Tls {
                server_name: Some("front.example.com".into()),
                fingerprint: Some("firefox".into()),
                alpn: vec!["h2".into(), "http/1.1".into()],
                allow_insecure: false,
            }
        );
    }

    #[test]
    fn parses_grpc_and_ipv6() {
        let link = VlessLink::parse(&format!(
            "vless://{UUID}@[2001:db8::1]:2053?type=grpc&serviceName=svc&mode=multi&security=none"
        ))
        .unwrap();
        assert_eq!(link.address, "2001:db8::1");
        assert_eq!(link.port, 2053);
        assert_eq!(
            link.transport,
            Transport::Grpc {
                service_name: "svc".into(),
                authority: None,
                multi_mode: true,
            }
        );
        assert_eq!(link.security, Security::None);
    }

    #[test]
    fn xhttp_extra_and_plus_sign() {
        let link = VlessLink::parse(&format!(
            "vless://{UUID}@h.example:443?type=xhttp&mode=packet-up&path=%2Fa+b\
             &extra=%7B%22xPaddingBytes%22%3A%22100-1000%22%7D&security=tls"
        ))
        .unwrap();
        let Transport::Xhttp {
            path, mode, extra, ..
        } = link.transport
        else {
            panic!()
        };
        assert_eq!(path, "/a+b", "plus is literal");
        assert_eq!(mode, "packet-up");
        assert_eq!(extra, Some(json!({"xPaddingBytes": "100-1000"})));
    }

    #[test]
    fn short_string_id_is_allowed() {
        let link = VlessLink::parse("vless://mypassword@h.example:443").unwrap();
        assert_eq!(link.id, "mypassword");
        assert_eq!(link.transport, Transport::Tcp { header_type: None });
        assert_eq!(link.security, Security::None);
    }

    #[test]
    fn rejects_malformed_links() {
        let long = "x".repeat(31);
        let cases: Vec<(String, LinkError)> = vec![
            ("vmess://abc".into(), LinkError::NotVless),
            ("vless://h.example:443".into(), LinkError::MissingId),
            ("vless://@h.example:443".into(), LinkError::MissingId),
            (format!("vless://{long}@h:443"), LinkError::BadId),
            (format!("vless://{UUID}@h.example"), LinkError::BadPort),
            (format!("vless://{UUID}@h.example:0"), LinkError::BadPort),
            (
                format!("vless://{UUID}@h.example:99999"),
                LinkError::BadPort,
            ),
            (format!("vless://{UUID}@:443"), LinkError::MissingHost),
            (
                format!("vless://{UUID}@h:443?type=kcp"),
                LinkError::UnsupportedTransport("kcp".into()),
            ),
            (
                format!("vless://{UUID}@h:443?security=xtls"),
                LinkError::UnsupportedSecurity("xtls".into()),
            ),
            (
                format!("vless://{UUID}@h:443?security=reality&sni=a"),
                LinkError::MissingPublicKey,
            ),
            (
                format!("vless://{UUID}@h:443?type=xhttp&extra=%5B1%5D"),
                LinkError::BadExtra,
            ),
        ];
        for (link, want) in cases {
            assert_eq!(VlessLink::parse(&link), Err(want), "{link}");
        }
    }

    #[test]
    fn config_snapshot_reality_xhttp() {
        let link = VlessLink::parse(REALITY_XHTTP).unwrap();
        let inbound = SocksInbound {
            port: 41234,
            user: "volna".into(),
            pass: "secret".into(),
        };
        let got = link.xray_config(&inbound);
        let want: Value =
            serde_json::from_str(include_str!("../tests/fixtures/reality_xhttp.json")).unwrap();
        assert_eq!(
            got,
            want,
            "generated config changed:\n{}",
            serde_json::to_string_pretty(&got).unwrap()
        );
    }

    #[test]
    fn tls_sni_defaults_to_address() {
        let link = VlessLink::parse(&format!("vless://{UUID}@h.example:443?security=tls")).unwrap();
        let out = link.outbound();
        assert_eq!(
            out["streamSettings"]["tlsSettings"]["serverName"],
            "h.example"
        );
        assert_eq!(out["streamSettings"]["network"], "tcp");
        assert!(out["streamSettings"].get("tcpSettings").is_none());
    }
}
