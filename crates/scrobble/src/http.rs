//! A tiny HTTP seam: services build [`Request`]s (tested without a network),
//! [`Http`] sends them. The daemon passes [`ReqwestHttp`] over the
//! `crates/net` client, so the app proxy applies.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde_json::Value;

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Empty,
    /// `application/x-www-form-urlencoded`, in this order.
    Form(Vec<(String, String)>),
    Json(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: vec![],
            body: Body::Empty,
        }
    }

    pub fn post(url: impl Into<String>, body: Body) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: vec![],
            body,
        }
    }

    pub fn header(mut self, k: &str, v: impl Into<String>) -> Self {
        self.headers.push((k.to_owned(), v.into()));
        self
    }

    /// The form fields as a map, for tests.
    pub fn form(&self) -> std::collections::BTreeMap<&str, &str> {
        match &self.body {
            Body::Form(f) => f.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect(),
            _ => Default::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Sends a request; only transport failures are errors ([`Error::Network`]),
/// any HTTP status is a [`Response`].
pub trait Http: Send + Sync {
    fn send(&self, req: Request) -> BoxFuture<'_, Result<Response, Error>>;
}

const TIMEOUT: Duration = Duration::from_secs(20);

/// [`Http`] over reqwest. The client is asked for on every request because
/// the daemon swaps it when the proxy setting changes.
pub struct ReqwestHttp {
    client: Box<dyn Fn() -> reqwest::Client + Send + Sync>,
}

impl ReqwestHttp {
    pub fn new(client: impl Fn() -> reqwest::Client + Send + Sync + 'static) -> Self {
        Self {
            client: Box::new(client),
        }
    }
}

impl Http for ReqwestHttp {
    fn send(&self, req: Request) -> BoxFuture<'_, Result<Response, Error>> {
        let client = (self.client)();
        Box::pin(async move {
            let mut b = match req.method {
                Method::Get => client.get(&req.url),
                Method::Post => client.post(&req.url),
            }
            .timeout(TIMEOUT);
            for (k, v) in &req.headers {
                b = b.header(k, v);
            }
            b = match req.body {
                Body::Empty => b,
                Body::Form(f) => b.form(&f),
                Body::Json(v) => b.json(&v),
            };
            let resp = b.send().await.map_err(|e| Error::Network(e.to_string()))?;
            let status = resp.status().as_u16();
            let body = resp
                .text()
                .await
                .map_err(|e| Error::Network(e.to_string()))?;
            Ok(Response { status, body })
        })
    }
}
