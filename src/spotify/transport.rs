use std::{
    io::Write,
    process::{Command, Stdio},
};

pub type Result<T> = std::result::Result<T, String>;

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub retry_after: u64,
}

pub trait Http: Send {
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<Response>;
}

pub struct Curl;

fn quoted(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

impl Http for Curl {
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<Response> {
        // Pass authorization and request data through stdin, never process arguments.
        let mut config = format!(
            "url = \"{}\"\nrequest = \"{}\"\n",
            quoted(url),
            quoted(method)
        );
        for (key, value) in headers {
            config.push_str(&format!(
                "header = \"{}: {}\"\n",
                quoted(key),
                quoted(value)
            ));
        }
        if let Some(body) = body {
            config.push_str(&format!("data = \"{}\"\n", quoted(body)));
        }
        let mut child = Command::new("curl")
            .args([
                "-q",
                "--silent",
                "--show-error",
                "--include",
                "--proto",
                "=https",
                "--connect-timeout",
                "10",
                "--max-time",
                "25",
                "--max-filesize",
                "10000000",
                "--config",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "curl is required. Install curl to connect to Spotify.".to_owned())?;
        child
            .stdin
            .take()
            .ok_or("Could not open HTTP request input")?
            .write_all(config.as_bytes())
            .map_err(|_| "Could not send HTTP request")?;
        let output = child
            .wait_with_output()
            .map_err(|_| "Could not finish HTTP request")?;
        if !output.status.success() {
            return Err(
                "Could not reach Spotify. Check your internet connection and try again.".into(),
            );
        }
        parse_response(&output.stdout)
    }
}

fn parse_response(mut raw: &[u8]) -> Result<Response> {
    loop {
        let split = raw
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .ok_or("Invalid HTTP response")?;
        let headers = std::str::from_utf8(&raw[..split]).map_err(|_| "Invalid HTTP headers")?;
        let status = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|status| status.parse().ok())
            .ok_or("Missing HTTP status")?;
        let retry_after = headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case("retry-after"))
            .and_then(|(_, value)| value.trim().parse().ok())
            .unwrap_or(30);
        raw = &raw[split + 4..];
        // A proxy's CONNECT response or interim 100 response may precede the real response.
        if raw.starts_with(b"HTTP/") {
            continue;
        }
        return Ok(Response {
            status,
            retry_after,
            body: raw.to_vec(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_proxy_headers_and_rate_limit() {
        let response = parse_response(
            b"HTTP/1.1 200 Connection established\r\n\r\nHTTP/2 429\r\nRetry-After: 12\r\n\r\n{} ",
        )
        .unwrap();
        assert_eq!(response.status, 429);
        assert_eq!(response.retry_after, 12);
        assert_eq!(response.body, b"{} ");
    }
    #[test]
    fn curl_values_cannot_inject_config_lines() {
        assert_eq!(quoted("x\"\ny"), "x\\\"\\ny");
    }
}
