//! Zero To Mastery Academy (Teachable + Hotmart + Vimeo) course downloader.
//!
//! Download chain (browser-harness required, Chrome must be open and logged in):
//!   1. Course URL → browser-harness DOM extraction → course structure (sections/lectures)
//!   2. Lecture URL → browser-harness navigate → CDP target: player.vimeo.com/video/{id}?h={hash}
//!   3. Vimeo URL → yt-dlp --referer https://player.hotmart.com/

pub mod api;
pub mod auth;

use anyhow::anyhow;
use async_trait::async_trait;
use std::path::Path;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::ytdlp;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};
use crate::platforms::traits::PlatformDownloader;
use omniget_core::models::course::{
    CourseInfo, LectureMedia,
};
use omniget_core::platforms::course_traits::CourseDownloader;

pub struct ZtmDownloader;

impl ZtmDownloader {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ZtmDownloader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PlatformDownloader for ZtmDownloader {
    fn name(&self) -> &str {
        "zerotomastery"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                return host == "academy.zerotomastery.io";
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        // For single lecture URLs, resolve to Vimeo and return info
        let media = CourseDownloader::resolve_lecture_media(self, url).await?;

        let ytdlp_path = ytdlp::ensure_ytdlp().await?;
        let referer = media.referer.clone().unwrap_or_else(|| "https://player.hotmart.com/".to_string());
        let extra_args: Vec<String> = vec!["--referer".to_string(), referer];
        let json = ytdlp::get_video_info(&ytdlp_path, &media.video_url, &extra_args).await?;

        let title = json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(&media.title)
            .to_string();
        let author = json
            .get("uploader")
            .and_then(|v| v.as_str())
            .unwrap_or("ZTM")
            .to_string();
        let duration = json.get("duration").and_then(|v| v.as_f64());
        let thumbnail = json
            .get("thumbnail")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut qualities: Vec<VideoQuality> = Vec::new();
        if let Some(formats) = json.get("formats").and_then(|v| v.as_array()) {
            let mut seen_heights = std::collections::HashSet::new();
            for f in formats {
                let height = f.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let width = f.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let vcodec = f.get("vcodec").and_then(|v| v.as_str()).unwrap_or("none");
                if vcodec == "none" || height == 0 { continue; }
                if seen_heights.insert(height) {
                    qualities.push(VideoQuality {
                        label: format!("{}p", height),
                        width,
                        height,
                        url: media.video_url.clone(),
                        format: "ytdlp".to_string(),
                    });
                }
            }
        }
        qualities.sort_by(|a, b| b.height.cmp(&a.height));
        if qualities.is_empty() {
            qualities.push(VideoQuality {
                label: "best".to_string(),
                width: 0,
                height: 0,
                url: media.video_url.clone(),
                format: "ytdlp".to_string(),
            });
        }

        Ok(MediaInfo {
            title,
            author,
            platform: "zerotomastery".to_string(),
            duration_seconds: duration,
            thumbnail_url: thumbnail,
            available_qualities: qualities,
            media_type: MediaType::Video,
            file_size_bytes: None,
        })
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let _ = progress.send(0.0).await;

        let first = info
            .available_qualities
            .first()
            .ok_or_else(|| anyhow!("No quality available"))?;

        let selected = if let Some(ref wanted) = opts.quality {
            info.available_qualities
                .iter()
                .find(|q| q.label == *wanted)
                .unwrap_or(first)
        } else {
            first
        };

        let video_url = selected.url.clone();
        let referer = opts
            .referer
            .clone()
            .unwrap_or_else(|| "https://player.hotmart.com/".to_string());

        // Use ffmpeg directly for Hotmart HLS. yt-dlp's generic extractor gets
        // blocked by Akamai's bot detection (TLS fingerprint mismatch). ffmpeg
        // downloads the m3u8 with a standard TLS stack that Akamai accepts.
        let cookie_header = opts.extra_headers
            .as_ref()
            .and_then(|h| h.get("Cookie"))
            .cloned();

        let expected_dur = info.duration_seconds;

        let result = download_hotmart_hls(
            &video_url,
            &referer,
            cookie_header.as_deref(),
            &opts.output_dir,
            info.title.as_str(),
            opts.filename_template.as_deref(),
            expected_dur,
            opts.cancel_token.clone(),
            progress.clone(),
        )
        .await;

        // Akamai hdntl tokens embedded in the master.m3u8 URL expire after ~15–60 min.
        // Lectures queued late in a large course often fail with 403 because the token
        // extracted during queue-building has long since expired by the time they run.
        // When this happens and the original lecture page URL is available (page_url),
        // re-navigate to it in Chrome to obtain a fresh token and retry once.
        if let Err(ref e) = result {
            let msg = e.to_string().to_lowercase();
            if msg.contains("403") || msg.contains("ffmpeg failed") {
                if let Some(ref lecture_url) = opts.page_url {
                    tracing::info!(
                        "[ztm] token expired for '{}', re-resolving via browser…",
                        info.title
                    );
                    match auth::navigate_and_get_vimeo_url(lecture_url).await {
                        Ok(stream) => {
                            tracing::info!(
                                "[ztm] re-resolved fresh HLS URL for '{}', retrying download…",
                                info.title
                            );
                            return download_hotmart_hls(
                                &stream.hls_url,
                                &referer,
                                stream.cookie_header.as_deref(),
                                &opts.output_dir,
                                info.title.as_str(),
                                opts.filename_template.as_deref(),
                                expected_dur,
                                opts.cancel_token.clone(),
                                progress,
                            )
                            .await;
                        }
                        Err(re_err) => {
                            tracing::warn!(
                                "[ztm] re-resolution failed for '{}': {}",
                                info.title,
                                re_err
                            );
                        }
                    }
                }
            }
        }

        result
    }
}

#[async_trait]
impl CourseDownloader for ZtmDownloader {
    fn course_platform_name(&self) -> &str {
        "Zero To Mastery Academy"
    }

    fn is_course_url(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            let path = parsed.path();
            return path.contains("/courses/") && !path.contains("/lectures/");
        }
        false
    }

    fn can_handle(&self, url: &str) -> bool {
        PlatformDownloader::can_handle(self, url)
    }

    fn login_url(&self) -> Option<&str> {
        Some("https://academy.zerotomastery.io/sign_in")
    }

    async fn get_course_structure(&self, course_url: &str) -> anyhow::Result<CourseInfo> {
        // ZTM's sidebar is rendered by client-side JavaScript, so we use
        // browser-harness to extract the DOM rather than HTTP GET + regex.
        auth::extract_course_structure_via_browser(course_url).await
    }

    async fn resolve_lecture_media(&self, lecture_url: &str) -> anyhow::Result<LectureMedia> {
        // Navigate to the lecture in Chrome and extract the Hotmart HLS master URL
        // from the Hotmart player iframe's performance entries.
        let stream = auth::navigate_and_get_vimeo_url(lecture_url).await?;

        match &stream.cookie_header {
            Some(c) if !c.is_empty() => tracing::info!("[ztm] cookies extracted ({} bytes)", c.len()),
            _ => tracing::warn!("[ztm] no cookies extracted from Chrome for this lecture"),
        }

        let lecture_id = lecture_url
            .split("/lectures/")
            .nth(1)
            .unwrap_or("unknown")
            .to_string();

        Ok(LectureMedia {
            lecture_id,
            title: "Lecture".to_string(),
            video_url: stream.hls_url,
            // Hotmart CDN requires this referer for the HLS token validation
            referer: Some("https://player.hotmart.com/".to_string()),
            duration_seconds: None,
            cookie_header: stream.cookie_header,
        })
    }
}

/// Returns the video duration of an existing MP4 via ffprobe, or None if the file
/// is missing, unreadable, or not a valid video.
async fn ffprobe_duration(path: &Path) -> Option<f64> {
    use omniget_core::core::dependencies::find_tool;
    let ffprobe = find_tool("ffprobe").await?;
    let output = tokio::process::Command::new(ffprobe)
        .args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_format",
            path.to_str()?,
        ])
        .output()
        .await
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    json.pointer("/format/duration")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|&d| d > 0.0)
}

/// Returns true if the MP4 at `path` exists and is a complete download.
///
/// When `expected_secs` is provided (from lecture metadata), the file's actual
/// duration must be within 5 seconds or 5% of the expected value — whichever
/// tolerance is larger. Without an expected duration, any readable file with a
/// valid video stream and size > 512 KB is accepted.
pub async fn is_complete_download(path: &Path, expected_secs: Option<f64>) -> bool {
    if !path.exists() {
        return false;
    }
    let size = path.metadata().map(|m| m.len()).unwrap_or(0);
    if size < 512 * 1024 {
        return false; // < 512 KB is certainly a partial/empty file
    }
    match ffprobe_duration(path).await {
        Some(actual) => {
            if let Some(expected) = expected_secs.filter(|&d| d > 0.0) {
                let tolerance = (expected * 0.05_f64).max(5.0);
                (actual - expected).abs() < tolerance
            } else {
                true // no expected duration: just being readable counts
            }
        }
        None => false, // ffprobe can't open it → invalid/partial
    }
}

/// Download a Hotmart HLS stream using ffmpeg instead of yt-dlp.
///
/// Akamai bot detection blocks yt-dlp's TLS fingerprint on the Hotmart CDN
/// (vod-akm.play.hotmart.com). ffmpeg uses a standard TLS stack and passes
/// the Akamai token validation without triggering bot detection.
async fn download_hotmart_hls(
    hls_url: &str,
    referer: &str,
    cookie_header: Option<&str>,
    output_dir: &Path,
    title: &str,
    filename_template: Option<&str>,
    expected_duration_secs: Option<f64>,
    cancel_token: CancellationToken,
    progress: mpsc::Sender<f64>,
) -> anyhow::Result<omniget_core::models::media::DownloadResult> {
    use omniget_core::core::dependencies::find_tool;
    use omniget_core::models::media::DownloadResult;
    use tokio::process::Command;
    use tokio::io::AsyncBufReadExt;

    let ffmpeg = find_tool("ffmpeg").await
        .ok_or_else(|| anyhow!("ffmpeg not found — install ffmpeg to download ZTM lectures"))?;

    std::fs::create_dir_all(output_dir)?;

    // Ignore yt-dlp format templates (contain `%(`) — they can't be expanded
    // by ffmpeg, so always use the resolved lecture title for ZTM downloads.
    let effective_name = filename_template
        .filter(|t| !t.contains("%("))
        .unwrap_or(title);
    let safe_title = sanitize_filename::sanitize(effective_name);
    let out_path = output_dir.join(format!("{}.mp4", safe_title));

    // Skip if already completely downloaded: verify via ffprobe duration rather
    // than just file size, so partial files left by a killed ffmpeg are re-downloaded.
    if is_complete_download(&out_path, expected_duration_secs).await {
        tracing::info!("[ztm] skipping '{}' — already complete", title);
        let size = out_path.metadata().map(|m| m.len()).unwrap_or(0);
        return Ok(DownloadResult {
            file_path: out_path.clone(),
            file_size_bytes: size,
            duration_seconds: expected_duration_secs.unwrap_or(0.0),
            torrent_id: None,
        });
    }

    let _ = progress.send(-2.0).await; // "Connecting"

    let out_str = out_path.to_str().ok_or_else(|| anyhow!("invalid output path"))?;

    // Use ffmpeg's dedicated HTTP options rather than raw -headers so they apply
    // correctly to all m3u8 and segment requests on macOS.
    // Build the cookie header value: browser session cookies are required by the
    // Hotmart/Akamai CDN in addition to the URL token.
    let cookie_arg = cookie_header.map(|c| format!("Cookie: {}\r\n", c));
    let headers_arg = format!(
        "Referer: {referer}\r\nOrigin: https://player.hotmart.com\r\nUser-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36\r\nAccept: */*\r\nAccept-Language: en-US,en;q=0.9\r\n{}",
        cookie_arg.as_deref().unwrap_or("")
    );

    tracing::info!(
        "[ztm/ffmpeg] headers: Referer={} Origin=https://player.hotmart.com cookies={}",
        referer,
        if cookie_arg.is_some() { "yes" } else { "no" }
    );

    let mut child = Command::new(&ffmpeg)
        .args([
            "-y",
            "-protocol_whitelist", "file,http,https,tcp,tls,crypto",
            "-headers", &headers_arg,
            "-http_persistent", "0",        // disable keep-alive; avoids stale conn 403s
            "-i", hls_url,
            "-c", "copy",
            "-bsf:a", "aac_adtstoasc",
            "-movflags", "+faststart",
            "-progress", "pipe:2",
            "-nostats",
            out_str,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("Failed to start ffmpeg: {}", e))?;

    let stderr = child.stderr.take().ok_or_else(|| anyhow!("No stderr"))?;
    let mut lines = tokio::io::BufReader::new(stderr).lines();

    let mut duration_us: f64 = 0.0;
    let mut error_lines: Vec<String> = Vec::new();
    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                let _ = child.kill().await;
                return Err(anyhow!("Download cancelled"));
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        if l.starts_with("out_time_us=") {
                            if let Ok(us) = l.trim_start_matches("out_time_us=").parse::<f64>() {
                                if duration_us > 0.0 {
                                    let pct = (us / duration_us * 100.0).min(99.0);
                                    let _ = progress.try_send(pct);
                                }
                            }
                        } else if l.contains("Duration:") {
                            if let Some(dur_str) = l.split("Duration:").nth(1) {
                                let dur_str = dur_str.trim().split(',').next().unwrap_or("").trim();
                                let parts: Vec<&str> = dur_str.split(':').collect();
                                if parts.len() == 3 {
                                    if let (Ok(h), Ok(m), Ok(s)) = (
                                        parts[0].parse::<f64>(),
                                        parts[1].parse::<f64>(),
                                        parts[2].parse::<f64>(),
                                    ) {
                                        duration_us = (h * 3600.0 + m * 60.0 + s) * 1_000_000.0;
                                    }
                                }
                            }
                        } else if l.starts_with("progress=end") {
                            break;
                        } else if l.contains("HTTP error") || l.contains("Error opening") || l.contains("403") || l.contains("No such file") || l.contains("Invalid data") {
                            tracing::warn!("[ztm/ffmpeg] {}", l);
                            error_lines.push(l);
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
    }

    let status = child.wait().await
        .map_err(|e| anyhow!("ffmpeg wait failed: {}", e))?;

    if !status.success() {
        if out_path.exists() { let _ = std::fs::remove_file(&out_path); }
        let detail = error_lines.last().cloned().unwrap_or_default();
        return Err(anyhow!("ffmpeg failed ({}): {}", status, detail));
    }

    let _ = progress.send(100.0).await;
    let size = out_path.metadata().map(|m| m.len()).unwrap_or(0);

    Ok(DownloadResult {
        file_path: out_path.clone(),
        file_size_bytes: size,
        duration_seconds: duration_us / 1_000_000.0,
        torrent_id: None,
    })
}

fn build_ztm_client(session_cookie: &str) -> anyhow::Result<reqwest::Client> {
    let jar = std::sync::Arc::new(reqwest::cookie::Jar::default());
    let url: reqwest::Url = "https://academy.zerotomastery.io"
        .parse()
        .expect("valid url");
    jar.add_cookie_str(
        &format!("_teachable_session={}", session_cookie),
        &url,
    );
    reqwest::Client::builder()
        .cookie_provider(jar)
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| anyhow!("Failed to build HTTP client: {}", e))
}
