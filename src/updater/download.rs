use std::path::Path;

use futures::StreamExt;
use tokio::io::AsyncWriteExt;

use super::check::{download_client, http_client};

const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

pub(crate) fn is_allowed_download_url(url: &url::Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() || url.host().is_none() {
        return false;
    }
    if url.scheme() == "https" {
        return true;
    }
    if url.scheme() != "http" {
        return false;
    }
    // Support local test servers, not prefix matches such as localhost.example.org.
    match url.host() {
        Some(url::Host::Domain(host)) => host == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn validated_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw).map_err(|_| "invalid update URL")?;
    if !is_allowed_download_url(&url) {
        return Err("refusing non-https or credential-bearing download URL".into());
    }
    Ok(url)
}

pub async fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    let url = validated_url(url)?;
    let client = download_client()?;
    let mut last_error = String::new();

    for attempt in 1..=3 {
        let response = client.get(url.clone()).send().await;
        match response {
            Ok(resp) if resp.status().is_success() => {
                match write_artifact(resp, destination).await {
                    Ok(()) => return Ok(()),
                    Err(error) => last_error = error,
                }
            }
            Ok(resp) => {
                let status = resp.status();
                last_error = format!("download failed with status {status}");
                if !status.is_server_error()
                    && status != reqwest::StatusCode::REQUEST_TIMEOUT
                    && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                {
                    return Err(last_error);
                }
            }
            Err(error) => {
                last_error = format!("download network error: {}", error.without_url());
            }
        }
        if attempt < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * attempt)).await;
        }
    }
    Err(last_error)
}

async fn write_artifact(resp: reqwest::Response, destination: &Path) -> Result<(), String> {
    if resp.content_length().is_some_and(|size| size > MAX_ARTIFACT_BYTES) {
        return Err("update artifact exceeds 256 MiB size limit".into());
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // Stage beside the destination: failed downloads never truncate an existing file.
    // TempPath also cleans up if this future is cancelled.
    let temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("failed to create temporary download: {e}"))?
        .into_temp_path();
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|e| format!("failed to open temporary download: {e}"))?;
    let mut stream = resp.bytes_stream();
    let mut received = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|e| format!("download chunk stream error: {}", e.without_url()))?;
        if chunk.len() as u64 > MAX_ARTIFACT_BYTES - received {
            return Err("update artifact exceeds 256 MiB size limit".into());
        }
        received += chunk.len() as u64;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("failed to write download data: {e}"))?;
    }
    if received == 0 {
        return Err("empty update artifact".into());
    }
    file.flush()
        .await
        .map_err(|e| format!("failed to flush download: {e}"))?;
    file.sync_all()
        .await
        .map_err(|e| format!("failed to sync download: {e}"))?;
    drop(file);
    temporary
        .persist(destination)
        .map_err(|e| format!("failed to commit download: {e}"))?;
    Ok(())
}

pub async fn download_text(url: &str) -> Result<String, String> {
    let url = validated_url(url)?;
    let resp = http_client()?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("failed to download text: {}", e.without_url()))?;
    if !resp.status().is_success() {
        return Err(format!("failed to download text, status: {}", resp.status()));
    }
    read_text(resp, MAX_TEXT_BYTES).await
}

async fn read_text(resp: reqwest::Response, limit: usize) -> Result<String, String> {
    if resp.content_length().is_some_and(|size| size > limit as u64) {
        return Err("update text exceeds size limit".into());
    }
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|e| format!("failed to read update text: {}", e.without_url()))?;
        if chunk.len() > limit - body.len() {
            return Err("update text exceeds size limit".into());
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| "update text is not valid UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    async fn serve(response: &'static [u8]) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            let _ = socket.write_all(response).await;
        });
        (url, task)
    }

    #[test]
    fn validates_hosts_instead_of_prefixes() {
        for raw in [
            "https://github.com/owner/repo/asset",
            "http://localhost:8080/asset",
            "http://127.0.0.1:8080/asset",
            "http://[::1]:8080/asset",
        ] {
            assert!(validated_url(raw).is_ok(), "{raw}");
        }
        for raw in [
            "http://localhost.example.com/asset",
            "http://127.0.0.1.example.com/asset",
            "http://localhost@evil.example/asset",
            "http://example.com/asset",
            "https://user:secret@example.com/asset",
            "file:///etc/passwd",
            "not a URL",
        ] {
            assert!(validated_url(raw).is_err(), "{raw}");
        }
    }

    #[tokio::test]
    async fn rejects_unsafe_redirect_before_requesting_target() {
        let (url, task) = serve(
            b"HTTP/1.1 302 Found\r\nLocation: http://localhost.example.com/asset\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(download_text(&url).await.is_err());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn enforces_limit_without_content_length() {
        let (url, task) = serve(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
        )
        .await;
        let resp = http_client().unwrap().get(url).send().await.unwrap();
        assert!(read_text(resp, 4).await.unwrap_err().contains("size limit"));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn failed_artifact_preserves_destination_and_removes_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset");
        std::fs::write(&destination, b"existing").unwrap();
        let (url, task) = serve(
            b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\ntruncated",
        )
        .await;
        let resp = download_client().unwrap().get(url).send().await.unwrap();
        assert!(write_artifact(resp, &destination).await.is_err());
        task.await.unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn commits_complete_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset");
        let (url, task) = serve(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
        )
        .await;
        download_file(&url, &destination).await.unwrap();
        task.await.unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), b"hello");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
