use super::models::Channel;
use futures::StreamExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::io::AsyncReadExt;

type PlaylistError = Box<dyn std::error::Error + Send + Sync>;
const MAX_PLAYLIST_BYTES: usize = 15 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

pub struct M3UParser {
    cache_dir: PathBuf,
    client: reqwest::Client,
}

impl Default for M3UParser {
    fn default() -> Self {
        Self::new()
    }
}

impl M3UParser {
    pub fn new() -> Self {
        let cache_dir = crate::config::cache_dir().join("tv_playlists");
        std::fs::create_dir_all(&cache_dir).ok();
        let client = crate::net::http_client_builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self { cache_dir, client }
    }

    pub async fn fetch_playlist(&self, url: &str) -> Result<Vec<Channel>, PlaylistError> {
        let trimmed = url.trim().trim_matches(|c| c == '"' || c == '\'').trim();
        if trimmed.is_empty() {
            return Err("playlist source is empty".into());
        }
        if crate::net::is_http_url(trimmed) {
            let file_path = self.cache_dir.join(cache_filename(trimmed));
            if cache_is_fresh(&file_path).await {
                if let Ok(content) = read_playlist_file(&file_path).await {
                    let channels = self.parse_m3u(&content);
                    if !channels.is_empty() {
                        return Ok(channels);
                    }
                }
                // Corrupt, empty, or oversized caches must take the same bounded
                // download path as a cache miss. Never cache an unsuccessful fetch.
                let _ = tokio::fs::remove_file(&file_path).await;
            }
            let response = self.client.get(trimmed).send().await?.error_for_status()?;
            let content = read_playlist_response(response, MAX_PLAYLIST_BYTES).await?;
            let channels = self.parse_m3u(&content);
            if !channels.is_empty() {
                let _ = crate::cache::atomic_write_file_async(&file_path, content.as_bytes()).await;
            }
            return Ok(channels);
        }

        let path = if let Some(stripped) = trimmed
            .strip_prefix("~/")
            .or_else(|| trimmed.strip_prefix("~\\"))
        {
            dirs::home_dir()
                .map(|home| home.join(stripped))
                .unwrap_or_else(|| PathBuf::from(trimmed))
        } else if trimmed == "~" {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from(trimmed))
        } else {
            PathBuf::from(trimmed)
        };
        let content = read_playlist_file(&path).await?;
        Ok(self.parse_m3u(&content))
    }

    pub fn parse_m3u(&self, content: &str) -> Vec<Channel> {
        let mut channels = Vec::new();
        let mut current_channel = Channel::default();

        for line in content.lines() {
            let line = line.trim().trim_start_matches('\u{feff}');
            if line.is_empty() {
                continue;
            }
            if line.starts_with("#EXTINF:") {
                // A missing URL must not leak metadata into the next entry.
                current_channel = Channel::default();
                current_channel.id = extract_attr_slice(line, "tvg-id").to_string();
                current_channel.logo = extract_attr_slice(line, "tvg-logo").to_string();
                current_channel.group = extract_attr_slice(line, "group-title").to_string();
                if let Some(idx) = find_extinf_title_comma(line) {
                    current_channel.name = line[idx + 1..].trim().to_string();
                }
            } else if !line.starts_with('#') {
                current_channel.stream_url = line.to_string();
                if current_channel.id.is_empty() {
                    current_channel.id = current_channel.name.clone();
                }
                channels.push(std::mem::take(&mut current_channel));
            }
        }
        channels
    }
}

async fn cache_is_fresh(path: &Path) -> bool {
    let Ok(metadata) = tokio::fs::metadata(path).await else {
        return false;
    };
    metadata.is_file()
        && metadata.len() <= MAX_PLAYLIST_BYTES as u64
        && metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age < CACHE_TTL)
}

async fn read_playlist_file(path: &Path) -> Result<String, PlaylistError> {
    let file = tokio::fs::File::open(path).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() {
        return Err("playlist source must be a regular file".into());
    }
    if metadata.len() > MAX_PLAYLIST_BYTES as u64 {
        return Err("local playlist exceeds maximum 15MB size limit".into());
    }
    // A metadata check alone is insufficient: the file can grow while being read.
    let mut bytes = Vec::new();
    file.take((MAX_PLAYLIST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > MAX_PLAYLIST_BYTES {
        return Err("local playlist exceeds maximum 15MB size limit".into());
    }
    Ok(String::from_utf8(bytes)?)
}

async fn read_playlist_response(
    response: reqwest::Response,
    limit: usize,
) -> Result<String, PlaylistError> {
    if response.content_length().is_some_and(|size| size > limit as u64) {
        return Err("remote playlist exceeds maximum 15MB size limit".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        // Enforce the bound before appending, including chunked responses with
        // no Content-Length header. Do not trust server-provided lengths.
        if chunk.len() > limit - bytes.len() {
            return Err("remote playlist exceeds maximum 15MB size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8(bytes)?)
}

fn extract_attr_slice<'a>(line: &'a str, attr_name: &str) -> &'a str {
    let bytes = line.as_bytes();
    let name_bytes = attr_name.as_bytes();
    let mut i = 0;
    while i + name_bytes.len() + 2 <= bytes.len() {
        let boundary = i == 0 || bytes[i - 1].is_ascii_whitespace();
        if boundary && &bytes[i..i + name_bytes.len()] == name_bytes {
            let next_byte = bytes[i + name_bytes.len()];
            if next_byte == b'=' {
                let quote = bytes[i + name_bytes.len() + 1];
                if quote == b'"' || quote == b'\'' {
                    let start = i + name_bytes.len() + 2;
                    if let Some(end_rel) = bytes[start..].iter().position(|&b| b == quote) {
                        return &line[start..start + end_rel];
                    }
                }
            }
        }
        i += 1;
    }
    ""
}

fn find_extinf_title_comma(line: &str) -> Option<usize> {
    let mut in_quote: Option<char> = None;
    for (idx, ch) in line.char_indices() {
        match in_quote {
            Some(quote) if ch == quote => in_quote = None,
            None if ch == '"' || ch == '\'' => in_quote = Some(ch),
            None if ch == ',' => return Some(idx),
            _ => {}
        }
    }
    line.find(',')
}

fn cache_filename(raw: &str) -> String {
    format!("{}.m3u", crate::cache::md5_hex(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn test_parse_m3u_double_and_single_quotes() {
        let parser = M3UParser::new();
        let content = r#"#EXTM3U
#EXTINF:-1 tvg-id="cnn.us" tvg-logo="http://logo.png/cnn.png" group-title="News",CNN HD
http://example.com/cnn.m3u8
#EXTINF:-1 tvg-id='bbc.uk' tvg-logo='http://logo.png/bbc.png' group-title='News',BBC World News
http://example.com/bbc.m3u8
#EXTINF:-1,Discovery Channel
http://example.com/discovery.m3u8
"#;
        let channels = parser.parse_m3u(content);
        assert_eq!(channels.len(), 3);
        assert_eq!(channels[0].id, "cnn.us");
        assert_eq!(channels[0].name, "CNN HD");
        assert_eq!(channels[0].logo, "http://logo.png/cnn.png");
        assert_eq!(channels[0].group, "News");
        assert_eq!(channels[0].stream_url, "http://example.com/cnn.m3u8");
        assert_eq!(channels[1].id, "bbc.uk");
        assert_eq!(channels[1].name, "BBC World News");
        assert_eq!(channels[1].logo, "http://logo.png/bbc.png");
        assert_eq!(channels[1].group, "News");
        assert_eq!(channels[1].stream_url, "http://example.com/bbc.m3u8");
        assert_eq!(channels[2].id, "Discovery Channel");
        assert_eq!(channels[2].name, "Discovery Channel");
        assert_eq!(channels[2].group, "");
        assert_eq!(channels[2].stream_url, "http://example.com/discovery.m3u8");
    }

    #[test]
    fn test_parse_m3u_title_with_commas_and_quoted_attributes() {
        let content = r#"#EXTM3U
#EXTINF:-1 tvg-id="cnn.us" group-title="News, International",CNN, The Worldwide News Leader
http://example.com/cnn.m3u8
#EXTINF:0,Movie Channel, HD (US), 24/7
http://example.com/movie.m3u8
"#;
        let channels = M3UParser::new().parse_m3u(content);
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].name, "CNN, The Worldwide News Leader");
        assert_eq!(channels[0].group, "News, International");
        assert_eq!(channels[1].name, "Movie Channel, HD (US), 24/7");
    }

    #[tokio::test]
    async fn test_load_channels_local_file_size_limit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("oversized.m3u");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len((MAX_PLAYLIST_BYTES + 1) as u64).unwrap();
        let result = M3UParser::new().fetch_playlist(path.to_str().unwrap()).await;
        assert!(result.unwrap_err().to_string().contains("size limit"));
    }

    #[test]
    fn test_parse_m3u_handles_utf8_bom_and_stream_inf_headers() {
        let content = "\u{feff}#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=2500000\n#EXTINF:-1 tvg-id=\"sp1\" group-title=\"Sports\",Sports Live\n#EXTVLCOPT:http-user-agent=CustomUA\nhttps://live.example.com/sports.m3u8\n";
        let channels = M3UParser::new().parse_m3u(content);
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].id, "sp1");
        assert_eq!(channels[0].name, "Sports Live");
        assert_eq!(channels[0].group, "Sports");
        assert_eq!(channels[0].stream_url, "https://live.example.com/sports.m3u8");
    }

    #[test]
    fn missing_stream_does_not_leak_metadata() {
        let content = "#EXTINF:-1 tvg-id=\"old\" tvg-logo=\"old.png\" group-title=\"Old\",Missing\n#EXTINF:-1,New\nhttps://example.com/new.m3u8\n";
        let channels = M3UParser::new().parse_m3u(content);
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].id, "New");
        assert!(channels[0].logo.is_empty());
        assert!(channels[0].group.is_empty());
    }

    #[test]
    fn attribute_names_must_match_whole_tokens() {
        assert_eq!(
            extract_attr_slice("#EXTINF:-1 not-tvg-id=\"wrong\" tvg-id=\"right\",Name", "tvg-id"),
            "right"
        );
    }

    async fn serve(response: &'static [u8]) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/playlist", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            let _ = socket.write_all(response).await;
        });
        (url, task)
    }

    #[tokio::test]
    async fn bounds_chunked_responses() {
        let (url, task) = serve(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
        )
        .await;
        let response = reqwest::Client::new().get(url).send().await.unwrap();
        let error = read_playlist_response(response, 4).await.unwrap_err();
        assert!(error.to_string().contains("size limit"));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn cache_recovery_uses_bounded_downloads() {
        let temp = tempfile::tempdir().unwrap();
        let parser = M3UParser {
            cache_dir: temp.path().to_path_buf(),
            client: reqwest::Client::new(),
        };
        let (url, task) = serve(
            b"HTTP/1.1 200 OK\r\nContent-Length: 15728641\r\nConnection: close\r\n\r\n",
        )
        .await;
        let cache = temp.path().join(cache_filename(&url));
        std::fs::write(&cache, b"#EXTM3U\n").unwrap();
        let error = parser.fetch_playlist(&url).await.unwrap_err();
        assert!(error.to_string().contains("size limit"));
        assert!(!cache.exists());
        task.await.unwrap();
    }
}
