//! Tauri commands for course platform operations.

use std::sync::Arc;

use serde::Serialize;
use tauri::{Emitter, State};

use crate::AppState;
use omniget_core::models::course::{CourseInfo, KnownCoursePlatform};
use omniget_core::core::known_course_platforms;
use omniget_core::platforms::course_traits::CourseDownloader;

/// Response for detect_course_platform command.
#[derive(Clone, Serialize)]
pub struct CoursePlatformDetection {
    pub is_course_url: bool,
    pub is_supported: bool,
    pub platform: Option<KnownCoursePlatform>,
    pub can_generate_task: bool,
}

/// Detect whether a URL is a course platform URL and if it's supported.
#[tauri::command]
pub async fn detect_course_platform(
    url: String,
    state: State<'_, AppState>,
) -> Result<CoursePlatformDetection, String> {
    // Check if any registered course downloader handles this URL
    let course_downloaders = state.course_registry.read()
        .map_err(|e| format!("Registry lock error: {}", e))?;

    let is_supported = course_downloaders.iter().any(|d| d.can_handle(&url));
    let is_course_url = is_supported ||
        known_course_platforms::detect_course_platform(&url).is_some();

    let platform = known_course_platforms::detect_course_platform(&url);
    let can_generate_task = platform.as_ref().map(|p| {
        !matches!(p.support_status, omniget_core::models::course::CoursePlatformStatus::Supported { .. })
    }).unwrap_or(false);

    Ok(CoursePlatformDetection {
        is_course_url,
        is_supported,
        platform,
        can_generate_task,
    })
}

/// Fetch full course structure (sections + lectures) for a supported course URL.
#[tauri::command]
pub async fn get_course_info(
    url: String,
    state: State<'_, AppState>,
) -> Result<CourseInfo, String> {
    // Scoped block ensures the RwLockReadGuard is dropped before any .await
    let downloader = {
        let course_downloaders = state.course_registry.read()
            .map_err(|e| format!("Registry lock error: {}", e))?;
        course_downloaders
            .iter()
            .find(|d| d.can_handle(&url))
            .cloned()
    };

    match downloader {
        Some(d) => {
            let course_url = normalize_to_course_url(&url);
            d.get_course_structure(&course_url)
                .await
                .map_err(|e| e.to_string())
        }
        None => Err(format!(
            "No course downloader supports this URL: {}",
            url
        )),
    }
}

/// Start downloading selected lectures from a course.
/// Returns a list of queue item IDs (one per lecture video).
#[tauri::command]
pub async fn start_course_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    course_info: CourseInfo,
    lecture_ids: Vec<String>, // empty = all video lectures
    output_dir: String,
    quality: Option<String>,
) -> Result<Vec<u64>, String> {
    use crate::core::queue;
    use omniget_core::models::course::LectureType;

    // Scoped block ensures the RwLockReadGuard is dropped before any .await
    let downloader = {
        let course_downloaders = state.course_registry.read()
            .map_err(|e| format!("Registry lock error: {}", e))?;
        course_downloaders
            .iter()
            .find(|d| d.can_handle(&course_info.course_url))
            .cloned()
    };

    let downloader = downloader.ok_or_else(|| {
        format!("No course downloader for: {}", course_info.course_url)
    })?;

    // Collect lectures to download
    let all_lectures: Vec<_> = course_info
        .sections
        .iter()
        .enumerate()
        .flat_map(|(sec_idx, section)| {
            section.lectures.iter().enumerate().map(move |(lec_idx, lec)| {
                (sec_idx, section.title.clone(), lec_idx, lec.clone())
            })
        })
        .filter(|(_, _, _, lec)| {
            lec.lecture_type == LectureType::Video
                && (lecture_ids.is_empty() || lecture_ids.contains(&lec.id))
        })
        .collect();

    if all_lectures.is_empty() {
        return Err("No video lectures selected".to_string());
    }

    // Build output path: {base}/{course_title}/
    let course_base_dir = std::path::PathBuf::from(&output_dir)
        .join(sanitize_filename::sanitize(&course_info.title));

    // Tell the frontend how many lectures are about to be queued so the toast
    // can show the right count before the background task starts resolving.
    let total_count = all_lectures.len();
    let _ = app.emit("course-queuing-started", serde_json::json!({ "total": total_count }));

    // Spawn the resolution loop as a background task so this command returns
    // immediately. Downloads start as each lecture is resolved — the frontend
    // doesn't need to wait for all 106 to be queued before anything happens.
    let download_queue = state.download_queue.clone();
    tokio::spawn(async move {
        static QUEUE_ID: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(10000);

        let all_lectures_count = all_lectures.len();
        let mut enqueued: usize = 0;

        for (sec_idx, section_title, _lec_idx, lecture) in all_lectures {
            let section_dir = course_base_dir.join(format!(
                "S{:02} - {}",
                sec_idx + 1,
                sanitize_filename::sanitize(&section_title)
            ));

            // Before doing the expensive browser navigation, check whether this
            // lecture has already been completely downloaded. This lets the user
            // re-run a course download after an interruption and skip all lectures
            // that finished successfully, without touching Chrome at all.
            let expected_path = section_dir.join(format!(
                "{}.mp4",
                sanitize_filename::sanitize(&lecture.title)
            ));
            if crate::platforms::ztm::is_complete_download(
                &expected_path,
                lecture.duration_seconds,
            )
            .await
            {
                tracing::info!("[ztm] '{}' already complete, skipping", lecture.title);
                let _ = app.emit(
                    "course-queuing-progress",
                    serde_json::json!({
                        "current": enqueued + 1,
                        "total": all_lectures_count,
                        "title": lecture.title,
                        "skipped": true
                    }),
                );
                continue;
            }

            // Pace navigations: give Chrome time to finish rendering the previous
            // lecture before we navigate away. Too fast = IPC daemon TimeoutErrors.
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            let media = match downloader.resolve_lecture_media(&lecture.url).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("[ztm] skipping lecture '{}': {}", lecture.title, e);
                    continue;
                }
            };

            let id = QUEUE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            let media_info = omniget_core::models::media::MediaInfo {
                title: lecture.title.clone(),
                author: "Zero To Mastery".to_string(),
                platform: "zerotomastery".to_string(),
                duration_seconds: lecture.duration_seconds,
                thumbnail_url: None,
                available_qualities: vec![omniget_core::models::media::VideoQuality {
                    label: quality.clone().unwrap_or_else(|| "best".to_string()),
                    width: 0,
                    height: 0,
                    url: media.video_url.clone(),
                    format: "ytdlp".to_string(),
                }],
                media_type: omniget_core::models::media::MediaType::Video,
                file_size_bytes: None,
            };

            let platform_downloader: Arc<dyn crate::platforms::traits::PlatformDownloader> =
                Arc::new(crate::platforms::ztm::ZtmDownloader::new());

            let mut extra_headers = std::collections::HashMap::new();
            extra_headers.insert(
                "Referer".to_string(),
                media.referer.clone().unwrap_or_else(|| "https://player.hotmart.com/".to_string()),
            );
            if let Some(ref cookies) = media.cookie_header {
                extra_headers.insert("Cookie".to_string(), cookies.clone());
            }

            let mut q = download_queue.lock().await;
            q.enqueue(
                id,
                media.video_url.clone(),
                "zerotomastery".to_string(),
                lecture.title.clone(),
                section_dir.to_string_lossy().to_string(),
                None,
                quality.clone(),
                None,
                media.referer.clone(),
                Some(extra_headers),
                Some(lecture.url.clone()), // page_url: preserved for token-expiry re-resolution
                None,
                Some(media_info),
                None,
                None,
                platform_downloader,
                None,
                false,
            );
            enqueued += 1;

            let state_snapshot = q.get_state();
            drop(q);

            queue::emit_queue_state_from_state(&app, state_snapshot);

            let _ = app.emit("course-queuing-progress", serde_json::json!({
                "current": enqueued,
                "total": all_lectures_count,
                "title": lecture.title
            }));

            // Kick the queue after each new item so it starts downloading
            // without waiting for all lectures to be resolved.
            let app_c = app.clone();
            let queue_c = download_queue.clone();
            tokio::spawn(async move {
                queue::try_start_next(app_c, queue_c).await;
            });
        }

        tracing::info!("[ztm] queuing complete: {}/{} lectures enqueued", enqueued, all_lectures_count);
    });

    // Return immediately — the frontend is free, queuing and downloading run in background.
    Ok(vec![])
}

/// Returns all known course platforms (supported + unsupported).
#[tauri::command]
pub fn get_supported_course_platforms() -> Vec<KnownCoursePlatform> {
    known_course_platforms::all_known_platforms()
}

/// Generate an AI agent task file for adding support for an unsupported course platform.
/// Saves to ~/omniget/omniget-tasks/omniget-task-{platform_id}.md
#[tauri::command]
pub async fn generate_platform_task(
    platform_name: String,
    platform_id: String,
    sample_url: String,
    detected_domain: String,
) -> Result<String, String> {
    let task_dir = dirs::home_dir()
        .ok_or_else(|| "Could not find home directory".to_string())?
        .join("omniget")
        .join("omniget-tasks");

    std::fs::create_dir_all(&task_dir)
        .map_err(|e| format!("Failed to create task directory: {}", e))?;

    let filename = format!("omniget-task-{}.md", platform_id.replace(' ', "_").to_lowercase());
    let task_path = task_dir.join(&filename);

    let content = generate_task_content(&platform_name, &platform_id, &sample_url, &detected_domain);

    std::fs::write(&task_path, content)
        .map_err(|e| format!("Failed to write task file: {}", e))?;

    // Try to open the file in the default editor
    let path_str = task_path.to_string_lossy().to_string();
    let _ = open::that(&path_str);

    Ok(path_str)
}

fn normalize_to_course_url(url: &str) -> String {
    // If this is a lecture URL (contains /lectures/), strip back to course URL
    if let Ok(parsed) = url::Url::parse(url) {
        let path = parsed.path();
        if let Some(idx) = path.find("/lectures/") {
            let course_path = &path[..idx];
            return format!(
                "{}://{}{}",
                parsed.scheme(),
                parsed.host_str().unwrap_or(""),
                course_path
            );
        }
    }
    url.to_string()
}

fn generate_task_content(
    platform_name: &str,
    platform_id: &str,
    sample_url: &str,
    detected_domain: &str,
) -> String {
    format!(
        r#"# OmniGet: Add {name} Course Download Support

**Generated by OmniGet** on {date}
**Sample URL**: {url}
**Detected Domain**: {domain}

---

## Context for the AI Agent

OmniGet is a Tauri 2.0 desktop app (Rust backend + SvelteKit frontend) at:
`/Users/Shared/ALL_WORKSPACE/ai_tools/omniget`

The course platform architecture is already in place. You only need to add the platform plugin.

### Existing reference implementation
The ZTM/Teachable+Hotmart plugin is the canonical example to follow:
- `src-tauri/src/platforms/ztm/mod.rs` — main platform (implements `PlatformDownloader` + `CourseDownloader`)
- `src-tauri/src/platforms/ztm/auth.rs` — browser cookie extraction via browser-harness CDP
- `src-tauri/src/platforms/ztm/api.rs` — platform API + HTML parsing

### Traits to implement
- `omniget_core::platforms::traits::PlatformDownloader` (single lecture download)
- `omniget_core::platforms::course_traits::CourseDownloader` (full course download)
- Both in `src-tauri/src/platforms/{{id}}/mod.rs`

---

## Step 1: Investigation (REQUIRED BEFORE CODING)

Run browser-harness while logged into {name} to map the video tech stack:

```bash
browser-harness -c '
new_tab("{url}")
wait_for_load()
import time; time.sleep(3)

# Find video player iframes
iframes = js("""
  return Array.from(document.querySelectorAll("iframe")).map(f => ({{
    src: f.src, id: f.id
  }}));
""")
print("Iframes:", iframes)

# Check CDP targets for embedded players
from pprint import pprint
targets = cdp("Target.getTargets", {{}})
video_targets = [t for t in targets.get("targetInfos", [])
                 if any(x in t.get("url","") for x in ["vimeo","player","video"])]
pprint(video_targets)

# Check for API calls to get video credentials
perf = js("return performance.getEntriesByType(\"resource\").map(e => e.name).filter(n => n.includes(\"video\") || n.includes(\"stream\") || n.includes(\"media\"));")
print("Video API calls:", perf)
'
```

Document what you find:
- [ ] Video player type (Vimeo / JW Player / Kaltura / Wistia / custom HLS)
- [ ] API endpoint that returns video credentials or stream URL
- [ ] Auth mechanism (session cookie name, JWT header, etc.)
- [ ] Course structure API (or HTML scraping needed)
- [ ] Whether yt-dlp can handle the final video URL directly

---

## Step 2: Implementation Checklist

### Files to create
- [ ] `src-tauri/src/platforms/{{id}}/mod.rs` — main platform struct + both trait impls
- [ ] `src-tauri/src/platforms/{{id}}/auth.rs` — cookie/token extraction
- [ ] `src-tauri/src/platforms/{{id}}/api.rs` — platform-specific API calls

### Files to modify
- [ ] `src-tauri/src/platforms/mod.rs` — add `#[cfg(not(target_os = "android"))] pub mod {{id}};`
- [ ] `src-tauri/src/lib.rs` — register in both `registry` and `course_registry`
- [ ] `src-tauri/omniget-core/src/platforms/mod.rs` — add `Platform::{{Name}}` enum variant + URL detection
- [ ] `src-tauri/omniget-core/src/core/known_course_platforms.rs` — update support status to `Supported`
- [ ] `src-tauri/omniget-cli/src/main.rs` — add platform display name if needed

### Pattern to follow in mod.rs
```rust
pub struct {{Name}}Downloader;

#[async_trait]
impl PlatformDownloader for {{Name}}Downloader {{
    fn name(&self) -> &str {{ "{id}" }}
    fn can_handle(&self, url: &str) -> bool {{ /* match {domain} */ }}
    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {{ /* single lecture */ }}
    async fn download(&self, info: &MediaInfo, opts: &DownloadOptions, progress: mpsc::Sender<f64>) -> anyhow::Result<DownloadResult> {{ /* yt-dlp */ }}
}}

#[async_trait]
impl CourseDownloader for {{Name}}Downloader {{
    fn is_course_url(&self, url: &str) -> bool {{ /* path pattern for course vs lecture */ }}
    async fn get_course_structure(&self, course_url: &str) -> anyhow::Result<CourseInfo> {{ /* scrape/API */ }}
    async fn resolve_lecture_media(&self, lecture_url: &str) -> anyhow::Result<LectureMedia> {{ /* resolve to stream URL */ }}
}}
```

---

## Step 3: Acceptance Criteria

1. `cargo check` passes with no errors
2. Given a course URL, `get_course_structure()` returns correct section/lecture count
3. `resolve_lecture_media()` returns a downloadable URL for a test lecture
4. `omniget course {{url}}` (CLI) lists sections and downloads all lectures
5. Each lecture saved as `S{{n:02}} - {{section}}/{{title}}.mp4`
6. Re-running skips already-downloaded files
7. Update `known_course_platforms.rs` status to `Supported {{ since_version: "0.5.x" }}`
8. Add platform doc section to `docs/COURSE_PLATFORMS.md`
"#,
        name = platform_name,
        id = platform_id,
        url = sample_url,
        domain = detected_domain,
        date = chrono::Utc::now().format("%Y-%m-%d"),
    )
}
