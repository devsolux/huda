//! Fixed read-only Wikimedia endpoints; no arbitrary URL executor, redirects or crawling.
use super::{PropertyMetadata, ProviderPolicy};
use crate::acquisition::admission::Capture;
use crate::error::{require, Error, Result};
use crate::relation::{canonical_bytes, now_ms, sha256_hex, text};
use reqwest::blocking::Client;
use serde_json::Value;
use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: String,
    pub label: String,
}
#[derive(Debug, Clone)]
pub struct Budget {
    pub requests: u32,
    pub bytes: u64,
    pub deadline_ms: u64,
}
impl Budget {
    pub fn new() -> Self {
        Self {
            requests: 0,
            bytes: 0,
            deadline_ms: now_ms() + 120_000,
        }
    }
    pub fn check(&self) -> Result<()> {
        require(
            self.requests < 12 && self.bytes < 8 * 1024 * 1024 && now_ms() < self.deadline_ms,
            "BUDGET_EXHAUSTED",
            "Expansion exhausted its request, byte or time budget",
        )
    }
}
impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}
struct RateState {
    next: Instant,
}
/// One shared limiter for this Core, stricter than separate per-job concurrency limits.
pub struct Wikimedia {
    rate: Mutex<RateState>,
}
impl Default for Wikimedia {
    fn default() -> Self {
        Self {
            rate: Mutex::new(RateState {
                next: Instant::now(),
            }),
        }
    }
}
impl Wikimedia {
    fn read(
        &self,
        host: &str,
        segments: &[&str],
        parameters: &[(&str, &str)],
        policy: &ProviderPolicy,
        budget: &mut Budget,
        checkpoint: &dyn Fn() -> Result<()>,
    ) -> Result<Value> {
        budget.check()?;
        checkpoint()?;
        require(
            allowed_host(host),
            "POLICY_DENIED",
            "Host is outside the fixed Wikimedia allowlist",
        )?;
        let mut rate = self
            .rate
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "Provider limiter unavailable"))?;
        while Instant::now() < rate.next {
            checkpoint()?;
            budget.check()?;
            std::thread::sleep(Duration::from_millis(50));
        }
        checkpoint()?;
        // Resolve only an approved host, reject private addresses, and pin those exact addresses
        // for the TLS request. The URL host remains unchanged for certificate verification.
        let addresses = (host, 443)
            .to_socket_addrs()
            .map_err(|_| Error::new("TEMPORARY_FAILURE", "Provider DNS lookup failed"))?
            .collect::<Vec<_>>();
        require(
            !addresses.is_empty() && addresses.iter().all(|address| public_address(address.ip())),
            "POLICY_DENIED",
            "Provider resolved to a non-public address",
        )?;
        budget.check()?;
        checkpoint()?;
        let timeout =
            Duration::from_millis(budget.deadline_ms.saturating_sub(now_ms()).clamp(1, 15_000));
        let client = Client::builder()
            .retry(reqwest::retry::never())
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(timeout)
            .resolve_to_addrs(host, &addresses)
            .build()?;
        let mut url = reqwest::Url::parse(&format!("https://{host}/"))
            .map_err(|_| Error::invalid("Invalid fixed provider URL"))?;
        url.path_segments_mut()
            .map_err(|_| Error::invalid("Invalid provider path"))?
            .extend(segments);
        url.query_pairs_mut()
            .extend_pairs(parameters.iter().copied());
        budget.requests += 1;
        rate.next = Instant::now() + Duration::from_secs(1);
        checkpoint()?;
        let response = client
            .get(url)
            .header(reqwest::header::USER_AGENT, &policy.user_agent)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()?;
        if response.status().as_u16() == 429 || response.status().as_u16() == 503 {
            // An unparseable HTTP-date is conservatively a one-day stop, not a retry loop.
            let delay = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(86400)
                .clamp(60, 604800);
            rate.next = Instant::now() + Duration::from_secs(delay);
            return Err(Error::new(
                "RATE_LIMITED",
                "Provider requested backoff; this expansion stopped",
            ));
        }
        require(
            response.status().is_success(),
            "TEMPORARY_FAILURE",
            "Provider returned an unsuccessful status; existing assertions are retained",
        )?;
        require(
            response
                .headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .is_none_or(|v| v == "identity"),
            "UNSUPPORTED_SCHEMA",
            "Compressed provider responses are not admitted",
        )?;
        require(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("application/json")),
            "UNSUPPORTED_SCHEMA",
            "Provider did not return JSON",
        )?;
        require(
            response.content_length().is_none_or(|n| n <= 1024 * 1024),
            "LIMIT_EXCEEDED",
            "Provider response is too large",
        )?;
        let mut bytes = Vec::new();
        response.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        require(
            bytes.len() <= 1024 * 1024,
            "LIMIT_EXCEEDED",
            "Provider response is too large",
        )?;
        budget.bytes += bytes.len() as u64;
        checkpoint()?;
        let value: Value = serde_json::from_slice(&bytes)?;
        if let Some(error) = value.get("error") {
            if error.get("code").and_then(Value::as_str) == Some("maxlag") {
                rate.next = Instant::now() + Duration::from_secs(60);
                return Err(Error::new(
                    "RATE_LIMITED",
                    "Provider maxlag requires backoff",
                ));
            }
            return Err(Error::new(
                "UNSUPPORTED_SCHEMA",
                "Provider returned an API error",
            ));
        }
        Ok(value)
    }
    pub fn discover(
        &self,
        policy: &ProviderPolicy,
        language: &str,
        public_term: &str,
        budget: &mut Budget,
        checkpoint: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<Candidate>> {
        require(
            policy.languages.contains(language)
                && policy.approved_public_terms.contains(public_term),
            "POLICY_DENIED",
            "Search phrase or language is not approved",
        )?;
        let value = if policy.provider_id == "wikidata" {
            self.read(
                "www.wikidata.org",
                &["w", "api.php"],
                &[
                    ("action", "wbsearchentities"),
                    ("search", public_term),
                    ("language", language),
                    ("type", "item"),
                    ("limit", "3"),
                    ("format", "json"),
                    ("maxlag", "5"),
                ],
                policy,
                budget,
                checkpoint,
            )?
        } else {
            self.read(
                &format!("{language}.wikipedia.org"),
                &["w", "rest.php", "v1", "search", "page"],
                &[("q", public_term), ("limit", "3")],
                policy,
                budget,
                checkpoint,
            )?
        };
        let results = value
            .get(if policy.provider_id == "wikidata" {
                "search"
            } else {
                "pages"
            })
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new("UNSUPPORTED_SCHEMA", "Search result schema changed"))?;
        require(
            results.len() <= 3,
            "UNSUPPORTED_SCHEMA",
            "Provider exceeded the requested result bound",
        )?;
        let mut candidates = Vec::new();
        for result in results {
            let (id, label) = if policy.provider_id == "wikidata" {
                (
                    result.get("id").and_then(Value::as_str),
                    result.get("label").and_then(Value::as_str),
                )
            } else {
                (
                    result.get("key").and_then(Value::as_str),
                    result.get("title").and_then(Value::as_str),
                )
            };
            if let (Some(id), Some(label)) = (id, label) {
                text(id, 256)?;
                text(label, 256)?;
                if policy.provider_id == "wikidata" && !external_id(id, b'Q') {
                    return Err(Error::new("UNSUPPORTED_SCHEMA", "Invalid Wikidata item ID"));
                }
                candidates.push(Candidate {
                    id: id.into(),
                    label: label.into(),
                });
            }
        }
        Ok(candidates)
    }
    pub fn fetch(
        &self,
        policy: &ProviderPolicy,
        language: &str,
        candidate: &Candidate,
        budget: &mut Budget,
        checkpoint: &dyn Fn() -> Result<()>,
    ) -> Result<Capture> {
        let (value, revision, uri, external_id, license) = if policy.provider_id == "wikidata" {
            require(
                external_id(&candidate.id, b'Q'),
                "INVALID_INPUT",
                "Invalid item ID",
            )?;
            let body = self.read(
                "www.wikidata.org",
                &["w", "api.php"],
                &[
                    ("action", "wbgetentities"),
                    ("ids", &candidate.id),
                    ("props", "info|labels|aliases|claims"),
                    ("languages", language),
                    ("format", "json"),
                    ("maxlag", "5"),
                ],
                policy,
                budget,
                checkpoint,
            )?;
            let entity = body
                .get("entities")
                .and_then(|v| v.get(&candidate.id))
                .filter(|v| v.get("missing").is_none())
                .ok_or_else(|| Error::new("NOT_FOUND", "Provider item was not found"))?
                .clone();
            require(
                entity.get("id").and_then(Value::as_str) == Some(&candidate.id),
                "UNSUPPORTED_SCHEMA",
                "Provider returned the wrong item",
            )?;
            let rev = entity
                .get("lastrevid")
                .and_then(Value::as_u64)
                .ok_or_else(|| Error::new("UNSUPPORTED_SCHEMA", "Provider revision is missing"))?
                .to_string();
            (
                entity,
                rev,
                format!("https://www.wikidata.org/wiki/{}", candidate.id),
                candidate.id.clone(),
                "CC0-1.0".to_owned(),
            )
        } else {
            let body = self.read(
                &format!("{language}.wikipedia.org"),
                &["w", "rest.php", "v1", "page", &candidate.id, "with_html"],
                &[],
                policy,
                budget,
                checkpoint,
            )?;
            let id = body
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| Error::new("UNSUPPORTED_SCHEMA", "Page identifier missing"))?;
            let rev = body
                .pointer("/latest/id")
                .and_then(Value::as_u64)
                .ok_or_else(|| Error::new("UNSUPPORTED_SCHEMA", "Page revision missing"))?
                .to_string();
            let license = body
                .pointer("/license/url")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    Error::new("UNSUPPORTED_SCHEMA", "Page license metadata is required")
                })?
                .to_string();
            require(
                body.get("html").and_then(Value::as_str).is_some(),
                "UNSUPPORTED_SCHEMA",
                "Page HTML missing",
            )?;
            (
                body,
                rev,
                format!("https://{language}.wikipedia.org/?curid={id}"),
                id.to_string(),
                license,
            )
        };
        let sha256 = sha256_hex(&canonical_bytes(&value)?);
        Ok(Capture {
            source_uri: Some(uri),
            provider_id: Some(policy.provider_id.clone()),
            external_id: Some(external_id),
            revision,
            retrieved_at: now_ms(),
            expires_at: Some(now_ms() + policy.ttl_seconds * 1000),
            sha256,
            license: Some(license),
            attribution: Some(format!(
                "{} contributors; exact revision retained in this capture",
                policy.provider_id
            )),
            payload: value,
        })
    }
    pub(crate) fn properties(
        &self,
        ids: &[String],
        policy: &ProviderPolicy,
        language: &str,
        budget: &mut Budget,
        checkpoint: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<PropertyMetadata>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        require(
            ids.len() <= 16 && ids.iter().all(|id| external_id(id, b'P')),
            "INVALID_INPUT",
            "Invalid property batch",
        )?;
        let joined = ids.join("|");
        let value = self.read(
            "www.wikidata.org",
            &["w", "api.php"],
            &[
                ("action", "wbgetentities"),
                ("ids", &joined),
                ("props", "info|datatype|labels"),
                ("languages", language),
                ("format", "json"),
                ("maxlag", "5"),
            ],
            policy,
            budget,
            checkpoint,
        )?;
        let mut properties = Vec::new();
        for pid in ids {
            let Some(property) = value.get("entities").and_then(|v| v.get(pid)) else {
                continue;
            };
            if property.get("id").and_then(Value::as_str) != Some(pid) {
                continue;
            }
            let Some(datatype) = property.get("datatype").and_then(Value::as_str) else {
                continue;
            };
            if !matches!(datatype, "wikibase-item" | "string") {
                continue;
            }
            let Some(revision) = property.get("lastrevid").and_then(Value::as_u64) else {
                continue;
            };
            let label = property
                .get("labels")
                .and_then(|v| v.get(language))
                .and_then(|v| v.get("value"))
                .and_then(Value::as_str)
                .unwrap_or(pid)
                .to_string();
            text(&label, 512)?;
            properties.push(PropertyMetadata {
                pid: pid.clone(),
                datatype: datatype.into(),
                revision,
                label,
                sha256: sha256_hex(&canonical_bytes(property)?),
            });
        }
        Ok(properties)
    }
}
pub fn external_id(id: &str, prefix: u8) -> bool {
    let bytes = id.as_bytes();
    (2..=32).contains(&bytes.len())
        && bytes[0] == prefix
        && bytes[1] != b'0'
        && bytes[1..].iter().all(u8::is_ascii_digit)
}
fn allowed_host(host: &str) -> bool {
    matches!(
        host,
        "www.wikidata.org"
            | "en.wikipedia.org"
            | "de.wikipedia.org"
            | "tr.wikipedia.org"
            | "ar.wikipedia.org"
    )
}
fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let a = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_multicast()
                || v.is_broadcast()
                || v.is_documentation()
                || a[0] == 0
                || a[0] >= 240
                || a[0] == 127
                || (a[0] == 100 && (64..=127).contains(&a[1]))
                || (a[0] == 198 && (a[1] == 18 || a[1] == 19)))
        }
        IpAddr::V6(v) => {
            let a = v.segments();
            v.to_ipv4_mapped().is_none()
                && (a[0] & 0xe000) == 0x2000
                && !(a[0] == 0x2001 && (a[1] == 0 || a[1] == 0xdb8))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifiers_are_not_urls() {
        assert!(external_id("P31", b'P'));
        assert!(!external_id("P01", b'P'));
        assert!(!external_id("https://localhost", b'Q'));
    }
    #[test]
    fn rejects_private_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!public_address(ip.parse().unwrap()));
        }
    }
}
