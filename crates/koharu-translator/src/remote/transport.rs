use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use anyhow::Context;
use koharu_runtime::RunControl;
use reqwest::{RequestBuilder, StatusCode, header::HeaderMap};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::scheduler::Admission;
use crate::{Error, Result};

#[derive(Clone)]
pub(crate) struct RequestContext {
    pub(super) control: RunControl,
    pub(super) admission: Arc<Admission>,
}

tokio::task_local! { pub(crate) static CONTEXT: RequestContext; }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Disposition {
    Success,
    Retry(Option<Duration>),
    Quota,
    Permanent,
}

pub(crate) fn http_disposition(status: StatusCode, _body: &Value) -> Disposition {
    match status.as_u16() {
        200..=299 => Disposition::Success,
        408 | 429 | 500 | 502 | 503 | 504 | 529 => Disposition::Retry(None),
        402 => Disposition::Quota,
        _ => Disposition::Permanent,
    }
}

pub(crate) async fn send_json<T: DeserializeOwned>(
    provider: &'static str,
    request: RequestBuilder,
    classify: fn(StatusCode, &Value) -> Disposition,
) -> Result<T> {
    static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let request_id = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let context = CONTEXT.try_with(Clone::clone).ok();
    let control = context
        .as_ref()
        .map(|c| c.control.clone())
        .unwrap_or_default();
    let attempts = if context.is_some() { 5 } else { 1 };
    for attempt in 0..attempts {
        let queued = Instant::now();
        let retry = request
            .try_clone()
            .context("remote request body cannot be replayed")?;
        let _slot = match &context {
            Some(context) => Some(
                context
                    .admission
                    .acquire(&control)
                    .await
                    .context("translation stopped")?,
            ),
            None => None,
        };
        let activity = control.admit().await.context("translation stopped")?;
        let started = Instant::now();
        tracing::info!(
            provider,
            request_id,
            attempt = attempt + 1,
            translation = context.is_some(),
            wait_ms = queued.elapsed().as_millis(),
            "remote request attempt started"
        );
        let retry = if context.is_some() {
            retry.timeout(Duration::from_secs(90))
        } else {
            retry
        };
        let response = tokio::select! {
            () = control.cancelled() => return Err(anyhow::anyhow!("translation stopped").into()),
            response = retry.send() => response,
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                drop(activity);
                drop(_slot);
                tracing::warn!(
                    provider,
                    request_id,
                    attempt = attempt + 1,
                    elapsed_ms = started.elapsed().as_millis(),
                    timeout = error.is_timeout(),
                    connection_error = error.is_connect(),
                    "remote request send failed"
                );
                // A response/read timeout may have completed and billed generation remotely.
                // Retry only connection establishment failures, never ambiguous read failures.
                if error.is_connect() && attempt + 1 < attempts {
                    if !control.delay(backoff(attempt)).await {
                        return Err(anyhow::anyhow!("translation stopped").into());
                    }
                    continue;
                }
                return Err(
                    anyhow::anyhow!("{provider} request failed: {}", error.without_url()).into(),
                );
            }
        };
        let status = response.status();
        tracing::info!(
            provider,
            request_id,
            attempt = attempt + 1,
            status = status.as_u16(),
            elapsed_ms = started.elapsed().as_millis(),
            "remote response headers received"
        );
        let headers = response.headers().clone();
        let text = tokio::select! {
            () = control.cancelled() => return Err(anyhow::anyhow!("translation stopped").into()),
            text = response.text() => text,
        };
        let text = text
            .inspect_err(|error| {
                tracing::warn!(
                    provider,
                    request_id,
                    attempt = attempt + 1,
                    status = status.as_u16(),
                    elapsed_ms = started.elapsed().as_millis(),
                    timeout = error.is_timeout(),
                    "remote response body failed"
                );
            })
            .with_context(|| format!("failed to read {provider} response"))?;
        drop(activity);
        let body = serde_json::from_str::<Value>(&text);
        let disposition = classify(status, body.as_ref().unwrap_or(&Value::Null));
        tracing::info!(
            provider,
            request_id,
            attempt = attempt + 1,
            status = status.as_u16(),
            elapsed_ms = started.elapsed().as_millis(),
            response_bytes = text.len(),
            ?disposition,
            "remote response body received"
        );
        if disposition == Disposition::Success {
            let body = body.with_context(|| format!("failed to decode {provider} response"))?;
            let decoded = serde_json::from_value(body)
                .with_context(|| format!("failed to decode {provider} response"))?;
            if let Some(context) = &context {
                context.admission.success();
            }
            return Ok(decoded);
        }
        if disposition == Disposition::Quota {
            tracing::warn!(
                provider,
                request_id,
                status = status.as_u16(),
                "remote quota exhausted"
            );
            return Err(Error::QuotaExceeded { provider });
        }
        let failure = || Error::Api {
            provider,
            status: status.as_u16(),
            message: text.chars().take(4096).collect(),
        };
        if !matches!(disposition, Disposition::Retry(_)) || attempt + 1 == attempts {
            tracing::warn!(
                provider,
                request_id,
                attempt = attempt + 1,
                status = status.as_u16(),
                ?disposition,
                "remote request failed without further retries"
            );
            return Err(failure());
        }
        let hint = match disposition {
            Disposition::Retry(hint) => hint,
            _ => None,
        };
        let delay = retry_after(&headers)
            .or(hint)
            .unwrap_or_else(|| backoff(attempt));
        if delay > Duration::from_secs(120) {
            tracing::warn!(
                provider,
                request_id,
                status = status.as_u16(),
                delay_ms = delay.as_millis(),
                "remote retry delay exceeds budget"
            );
            return Err(failure());
        }
        if let Some(context) = &context {
            context.admission.throttle(delay);
        }
        drop(_slot);
        tracing::warn!(
            provider,
            request_id,
            attempt = attempt + 1,
            status = status.as_u16(),
            elapsed_ms = started.elapsed().as_millis(),
            delay_ms = delay.as_millis(),
            "retrying transient translation request"
        );
        if !control.delay(delay).await {
            return Err(anyhow::anyhow!("translation stopped").into());
        }
    }
    unreachable!("bounded attempt loop returns a result")
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    if let Some(value) = headers
        .get("retry-after-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Some(Duration::from_millis(value));
    }
    let value = headers.get("retry-after")?.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(SystemTime::now()).unwrap_or_default())
}

fn backoff(attempt: usize) -> Duration {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    let jitter = (seed
        ^ SEQUENCE
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9e3779b97f4a7c15))
        % 251;
    Duration::from_millis((500u64 << attempt.min(4)) + jitter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn server(
        responses: Vec<(u16, &'static str, &'static str)>,
    ) -> (String, tokio::task::JoinHandle<usize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let count = responses.len();
            for (status, headers, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                stream.read(&mut request).await.unwrap();
                let reply = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}",
                    body.len()
                );
                stream.write_all(reply.as_bytes()).await.unwrap();
            }
            count
        });
        (url, task)
    }

    #[tokio::test]
    async fn retries_only_the_failed_request_and_returns_success_once() {
        let (url, server) = server(vec![
            (429, "Retry-After: 0\r\n", "{}"),
            (200, "", "{\"ok\":true}"),
        ])
        .await;
        let context = RequestContext {
            control: RunControl::default(),
            admission: Arc::new(Admission::new(6)),
        };
        let value: Value = CONTEXT
            .scope(
                context,
                send_json(
                    "test",
                    reqwest::Client::new()
                        .post(url)
                        .json(&serde_json::json!({})),
                    http_disposition,
                ),
            )
            .await
            .unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(server.await.unwrap(), 2);
    }

    #[tokio::test]
    async fn permanent_auth_errors_are_not_retried() {
        let (url, server) = server(vec![(401, "", "{\"error\":\"bad key\"}")]).await;
        let context = RequestContext {
            control: RunControl::default(),
            admission: Arc::new(Admission::new(6)),
        };
        let result: Result<Value> = CONTEXT
            .scope(
                context,
                send_json("test", reqwest::Client::new().post(url), http_disposition),
            )
            .await;
        assert!(matches!(result, Err(Error::Api { status: 401, .. })));
        assert_eq!(server.await.unwrap(), 1);
    }

    #[tokio::test]
    async fn pause_keeps_admission_closed_and_stop_interrupts_retry_waits() {
        let (url, mut server) = server(vec![(429, "Retry-After: 60\r\n", "{}")]).await;
        let control = RunControl::default();
        control.pause();
        let context = RequestContext {
            control: control.clone(),
            admission: Arc::new(Admission::new(6)),
        };
        let request = tokio::spawn(async move {
            CONTEXT
                .scope(
                    context,
                    send_json::<Value>("test", reqwest::Client::new().post(url), http_disposition),
                )
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut server)
                .await
                .is_err()
        );
        control.resume();
        assert_eq!(server.await.unwrap(), 1);
        control.stop();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), request)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
    }

    #[tokio::test]
    async fn stops_after_the_retry_budget_and_never_retries_quota() {
        let (url, server) = server(vec![(429, "Retry-After: 0\r\n", "{}"); 5]).await;
        let context = RequestContext {
            control: RunControl::default(),
            admission: Arc::new(Admission::new(6)),
        };
        let result: Result<Value> = CONTEXT
            .scope(
                context,
                send_json("test", reqwest::Client::new().post(url), http_disposition),
            )
            .await;
        assert!(matches!(result, Err(Error::Api { status: 429, .. })));
        assert_eq!(server.await.unwrap(), 5);
        let (url, server) = self::server(vec![(
            429,
            "",
            "{\"error\":{\"code\":\"insufficient_quota\"}}",
        )])
        .await;
        let context = RequestContext {
            control: RunControl::default(),
            admission: Arc::new(Admission::new(6)),
        };
        let result: Result<Value> = CONTEXT
            .scope(
                context,
                send_json(
                    "test",
                    reqwest::Client::new().post(url),
                    super::super::openai::disposition,
                ),
            )
            .await;
        assert!(matches!(result, Err(Error::QuotaExceeded { .. })));
        assert_eq!(server.await.unwrap(), 1);
    }

    #[test]
    fn distinguishes_throttling_auth_and_quota_and_parses_retry_hints() {
        assert_eq!(
            http_disposition(StatusCode::TOO_MANY_REQUESTS, &Value::Null),
            Disposition::Retry(None)
        );
        assert_eq!(
            http_disposition(StatusCode::UNAUTHORIZED, &Value::Null),
            Disposition::Permanent
        );
        assert_eq!(
            http_disposition(StatusCode::PAYMENT_REQUIRED, &Value::Null),
            Disposition::Quota
        );
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "12".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(12)));
        headers.insert(
            "retry-after",
            "Wed, 01 Jan 2020 00:00:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&headers), Some(Duration::ZERO));
        headers.insert("retry-after-ms", "1500".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_millis(1500)));
    }
}
