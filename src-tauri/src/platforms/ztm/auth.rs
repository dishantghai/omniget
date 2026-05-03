//! Cookie extraction for ZTM/Teachable auth.
//!
//! Strategy (in order):
//!   1. CDP via browser-harness subprocess (fastest — uses the already-open Chrome session)
//!   2. yt-dlp --cookies-from-browser chrome (fallback — slower but no browser-harness dep)
//!   3. Error with clear instructions

use anyhow::anyhow;
use std::sync::LazyLock;

// Serializes all browser-harness navigations for ZTM: prevents concurrent retries from
// navigating Chrome to different lecture pages simultaneously.
static ZTM_NAV_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

const ZTM_BASE: &str = "https://academy.zerotomastery.io";
const COOKIE_NAME: &str = "_teachable_session";

/// Returns the value of the `_teachable_session` cookie from Chrome.
pub async fn get_ztm_session_cookie() -> anyhow::Result<String> {
    // Try CDP via browser-harness first
    if let Ok(cookie) = extract_via_cdp().await {
        if !cookie.is_empty() {
            tracing::info!("[ztm] session cookie extracted via CDP");
            return Ok(cookie);
        }
    }

    // Fall back to yt-dlp browser cookie extraction
    if let Ok(cookie) = extract_via_ytdlp().await {
        if !cookie.is_empty() {
            tracing::info!("[ztm] session cookie extracted via yt-dlp");
            return Ok(cookie);
        }
    }

    Err(anyhow!(
        "Could not extract ZTM session cookie from Chrome. \
         Make sure you are logged into academy.zerotomastery.io in Chrome."
    ))
}

/// Extract cookies using browser-harness (CDP bridge).
async fn extract_via_cdp() -> anyhow::Result<String> {
    // Check browser-harness is available
    let bh_path = which::which("browser-harness")
        .map_err(|_| anyhow!("browser-harness not found"))?;

    let script = format!(
        r#"
result = cdp("Network.getCookies", {{"urls": ["{base_url}"]}})
cookies = result.get("cookies", [])
match = next((c for c in cookies if c["name"] == "{name}"), None)
print(match["value"] if match else "")
"#,
        base_url = ZTM_BASE,
        name = COOKIE_NAME
    );

    let output = tokio::process::Command::new(bh_path)
        .args(["-c", &script])
        .output()
        .await
        .map_err(|e| anyhow!("browser-harness exec failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("browser-harness failed: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Extract the _teachable_session cookie using yt-dlp's --cookies-from-browser.
/// This is a fallback that works even without browser-harness.
async fn extract_via_ytdlp() -> anyhow::Result<String> {
    let ytdlp_path = omniget_core::core::ytdlp::find_ytdlp_cached()
        .await
        .ok_or_else(|| anyhow!("yt-dlp not found"))?;

    // Use yt-dlp to dump the cookie file for the domain
    let tmp_cookie_file = std::env::temp_dir().join("ztm_cookies.txt");

    let output = tokio::process::Command::new(&ytdlp_path)
        .args([
            "--cookies-from-browser",
            "chrome",
            "--cookies",
            tmp_cookie_file.to_str().unwrap_or("/tmp/ztm_cookies.txt"),
            "--skip-download",
            "--quiet",
            ZTM_BASE,
        ])
        .output()
        .await
        .map_err(|e| anyhow!("yt-dlp exec failed: {}", e))?;

    if !output.status.success() && tmp_cookie_file.exists() {
        // May still have written the cookie file even on "error"
    }

    // Parse the Netscape cookie file
    if tmp_cookie_file.exists() {
        let content = std::fs::read_to_string(&tmp_cookie_file)?;
        let _ = std::fs::remove_file(&tmp_cookie_file);

        for line in content.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 7 {
                let domain = parts[0].trim_start_matches('.');
                let name = parts[5];
                let value = parts[6];
                if domain.contains("zerotomastery.io") && name == COOKIE_NAME {
                    return Ok(value.to_string());
                }
            }
        }
    }

    Err(anyhow!("Cookie not found in yt-dlp extracted cookies"))
}

/// Use browser-harness CDP to find the current Vimeo embed URL from any open
/// ZTM lecture page. Inspects all iframe targets for player.vimeo.com URLs.
pub async fn find_vimeo_embed_url_in_browser() -> anyhow::Result<Option<String>> {
    let bh_path = match which::which("browser-harness") {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };

    let script = r#"
result = cdp("Target.getTargets", {})
targets = result.get("targetInfos", [])
vimeo = next(
    (t["url"] for t in targets
     if "player.vimeo.com/video/" in t.get("url", "")),
    None
)
print(vimeo or "")
"#;

    let output = tokio::process::Command::new(bh_path)
        .args(["-c", script])
        .output()
        .await
        .map_err(|e| anyhow!("browser-harness exec failed: {}", e))?;

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        Ok(None)
    } else {
        Ok(Some(url))
    }
}

/// Navigate to a ZTM lecture page in the user's running Chrome and wait for
/// the Hotmart player to load, then extract the Vimeo URL from CDP targets.
///
/// The Hotmart player iframe is a React SPA — the Vimeo URL is never in the
/// raw HTML but always appears as a CDP iframe target after ~4-6 seconds.
/// Returns the HLS master playlist URL for a ZTM lecture.
///
/// ZTM lectures are served via Hotmart's own HLS CDN (vod-akm.play.hotmart.com).
/// The Hotmart player fetches the master .m3u8 on load (autoplay). We read it
/// from performance.getEntriesByType("resource") inside the Hotmart iframe.
/// The URL carries an Akamai token (~8 min window for manifest fetch, ~24 h for
/// segments), so it must be used promptly after extraction.
/// HLS stream info returned by navigate_and_get_stream_info.
pub struct HotmartStream {
    pub hls_url: String,
    /// Raw Cookie header value extracted from Chrome (e.g. "k1=v1; k2=v2").
    pub cookie_header: Option<String>,
}

pub async fn navigate_and_get_vimeo_url(lecture_url: &str) -> anyhow::Result<HotmartStream> {
    // Hold the global lock for the duration of this call so concurrent retries
    // don't navigate Chrome to different lecture pages at the same time.
    let _nav_guard = ZTM_NAV_LOCK.lock().await;

    let bh_path = which::which("browser-harness")
        .map_err(|_| anyhow!(
            "browser-harness not found. Install it so OmniGet can extract video URLs. \
             See: https://github.com/browser-use/browser-harness"
        ))?;

    let script = format!(r#"
import time

ensure_real_tab()
goto_url("{url}")
wait_for_load()
time.sleep(5)

# Locate the Hotmart player iframe CDP target
hotmart_tid = iframe_target("player.hotmart.com")
if not hotmart_tid:
    time.sleep(4)
    hotmart_tid = iframe_target("player.hotmart.com")

if not hotmart_tid:
    print("")
else:
    # Force video play in case autoplay was blocked by the browser.
    try:
        js("""
          const vid = document.querySelector('video');
          if (vid && vid.paused) {{ vid.play().catch(() => {{}}); }}
        """, target_id=hotmart_tid)
        time.sleep(3)
    except Exception:
        pass

    def get_hls_url():
        entries = js("""
          return performance.getEntriesByType("resource").map(e => e.name);
        """, target_id=hotmart_tid)

        # Prefer the master playlist: CDN validates the hdntl token only when the
        # request follows the standard master→quality→segments chain. Requesting a
        # quality URL directly (skipping master) gets a 403 from Akamai even with
        # a valid token, because the CDN hasn't established a session for this stream.
        master = next(
            (e for e in entries
             if "vod-akm.play.hotmart.com" in e and "master" in e and ".m3u8" in e),
            ""
        )
        if master:
            return master

        # Fallback: quality-specific URL (only if master is not in performance entries)
        quality_urls = [
            e for e in entries
            if "vod-akm.play.hotmart.com" in e
            and "audio=" in e
            and "video=" in e
            and ".m3u8" in e
            and "hdntl=" in e
            and "textstream" not in e
        ]
        if quality_urls:
            def get_vbr(url):
                try:
                    for part in url.split("?")[0].split("-"):
                        if part.startswith("video="):
                            return int(part.split("=")[1].replace(".m3u8", ""))
                except Exception:
                    pass
                return 0
            return max(quality_urls, key=get_vbr)

        return ""

    hls_url = get_hls_url()
    if not hls_url:
        time.sleep(6)
        hls_url = get_hls_url()

    if not hls_url:
        print("")
    else:
        import json as _json, sys as _sys
        # Chrome 118+ partitions third-party iframe cookies (CHIPS).
        # The Akamai _abck cookie set inside the player.hotmart.com iframe is
        # stored in a partition keyed to academy.zerotomastery.io, so it is NOT
        # visible from the main ZTM page session. We must query from a session
        # attached to the Hotmart iframe target itself.
        cookie_str = ""
        try:
            hotmart_attach = cdp("Target.attachToTarget", {{
                "targetId": hotmart_tid,
                "flatten": True
            }})
            hotmart_session = hotmart_attach.get("sessionId")
            if hotmart_session:
                cdn_cookies = cdp("Network.getCookies", {{
                    "urls": [
                        "https://vod-akm.play.hotmart.com",
                        "https://player.hotmart.com",
                        "https://hotmart.com"
                    ]
                }}, session_id=hotmart_session)
                all_cookies = cdn_cookies.get("cookies", [])
                print(f"[ztm-dbg] got {{len(all_cookies)}} cookies from hotmart iframe session", file=_sys.stderr)
                cookie_str = "; ".join(
                    f"{{c['name']}}={{c['value']}}"
                    for c in all_cookies
                    if c.get("value")
                )
            else:
                print("[ztm-dbg] could not attach to hotmart iframe target", file=_sys.stderr)
        except Exception as e:
            print(f"[ztm] cookie extraction failed: {{e}}", file=_sys.stderr)
        print(_json.dumps({{"url": hls_url, "cookies": cookie_str}}))
"#, url = lecture_url);

    let mut last_err = String::new();
    for attempt in 0..3u8 {
        if attempt > 0 {
            // IPC timeout: Chrome is still busy from the previous navigation.
            // Wait longer each retry to let it settle.
            let wait = if attempt == 1 { 8 } else { 15 };
            tracing::warn!("[ztm] browser-harness timeout for {} (attempt {}), retrying in {}s…", lecture_url, attempt, wait);
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        }

        let output = tokio::process::Command::new(&bh_path)
            .args(["-c", &script])
            .output()
            .await
            .map_err(|e| anyhow!("browser-harness exec failed: {}", e))?;

        // Always log stderr lines so [ztm-dbg] diagnostics surface in the Rust trace.
        let stderr_text = String::from_utf8_lossy(&output.stderr).to_string();
        for line in stderr_text.lines() {
            let l = line.trim();
            if !l.is_empty() {
                tracing::debug!("[ztm/bh] {}", l);
            }
        }

        if !output.status.success() {
            last_err = stderr_text.clone();
            // Retry on IPC timeout — the daemon can become briefly unresponsive.
            if stderr_text.contains("TimeoutError") {
                continue;
            }
            return Err(anyhow!("browser-harness failed for {}: {}", lecture_url, stderr_text.trim()));
        }

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() {
            return Err(anyhow!(
                "No HLS stream found for lecture {}. \
                 Make sure Chrome is open and you are logged into academy.zerotomastery.io.",
                lecture_url
            ));
        }

        // Parse JSON output {"url": "...", "cookies": "..."}
        let decoded = raw.replace("&amp;", "&");
        let parsed: serde_json::Value = serde_json::from_str(&decoded)
            .unwrap_or(serde_json::Value::Null);

        let hls_url = parsed.get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!(
                "No HLS stream found for lecture {}. \
                 Make sure Chrome is open and you are logged into academy.zerotomastery.io.",
                lecture_url
            ))?
            .to_string();

        let cookie_header = parsed.get("cookies")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        return Ok(HotmartStream { hls_url, cookie_header });
    }

    Err(anyhow!("browser-harness timed out for {}: {}", lecture_url, last_err.trim()))
}

/// Use browser-harness to navigate to the course page and extract the full
/// course structure from the live DOM (`.slim-section` elements).
///
/// ZTM's course sidebar is rendered by client-side JavaScript, so a plain
/// HTTP GET returns empty HTML — we need the browser's rendered DOM.
pub async fn extract_course_structure_via_browser(
    course_url: &str,
) -> anyhow::Result<omniget_core::models::course::CourseInfo> {
    let bh_path = which::which("browser-harness")
        .map_err(|_| anyhow!("browser-harness not found"))?;

    // Note: JS is embedded in Python triple-quotes inside a Rust raw string.
    // Avoid regex with backslashes — use String methods instead.
    // Use "a[href*=lectures]" (no quotes in selector) to avoid escaping issues.
    let script = format!(r#"
import time, json

goto_url("{url}")
wait_for_load()
time.sleep(4)

data = js("""
  const sections = document.querySelectorAll(".slim-section");
  if (sections.length === 0) return null;

  // Extract course title from page heading or document title
  const headingEl = document.querySelector("h1, h2.course-title, [class*=course-heading], [class*=course-title]");
  let courseTitle = headingEl ? headingEl.innerText.trim().split("\\n")[0].trim() : "";
  if (!courseTitle) {{
    // Fall back to document.title, stripping trailing " | Zero To Mastery" etc.
    courseTitle = document.title.split("|")[0].split("-")[0].trim();
  }}

  const result = [];
  for (let secIdx = 0; secIdx < sections.length; secIdx++) {{
    const sec = sections[secIdx];
    const titleEl = sec.querySelector("h2, h3, [class*=title]");
    const secTitle = titleEl ? titleEl.innerText.trim().split("\\n")[0].trim() : "Section " + (secIdx+1);
    const seenHrefs = new Set();
    const lectures = [];
    for (const a of sec.querySelectorAll("a[href*=lectures]")) {{
      if (!seenHrefs.has(a.href)) {{
        seenHrefs.add(a.href);
        const row = a.closest("li, tr") || a.parentElement;
        const rowText = row ? row.innerText : a.innerText;
        const lines = rowText.split("\\n").map(s => s.trim()).filter(Boolean);
        const lecTitle = lines[0] || a.innerText.trim();
        // Extract duration like "5:43" or "1:05:12" — look for m:ss or h:mm:ss pattern
        let duration = null;
        for (const part of lines) {{
          if (part.length >= 4 && part.length <= 8 && part.includes(":")) {{
            const segs = part.replace("(","").replace(")","").split(":");
            if (segs.every(s => s.length > 0 && !isNaN(Number(s)))) {{
              duration = part.replace("(","").replace(")","").trim();
              break;
            }}
          }}
        }}
        lectures.push({{
          id: a.href.split("/lectures/")[1],
          lecTitle: lecTitle,
          url: a.href,
          duration: duration
        }});
      }}
    }}
    if (lectures.length > 0) {{
      result.push({{ secTitle: secTitle, lectures: lectures }});
    }}
  }}
  return JSON.stringify({{ courseTitle: courseTitle, sections: result }});
""")
print(data or "{{}}")
"#, url = course_url);

    let output = tokio::process::Command::new(&bh_path)
        .args(["-c", &script])
        .output()
        .await
        .map_err(|e| anyhow!("browser-harness exec failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("browser-harness failed: {}", stderr.trim()));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() || raw == "null" || raw == "[]" || raw == "{}" {
        return Err(anyhow!(
            "No course sections found on {}. \
             Make sure Chrome is open and you are logged into academy.zerotomastery.io.",
            course_url
        ));
    }

    parse_browser_course_data(&raw, course_url)
}

fn parse_browser_course_data(
    json_str: &str,
    course_url: &str,
) -> anyhow::Result<omniget_core::models::course::CourseInfo> {
    use omniget_core::models::course::{CourseInfo, CourseSection, CourseLecture, LectureType};

    #[derive(serde::Deserialize)]
    struct JsRoot {
        #[serde(rename = "courseTitle", default)]
        course_title: String,
        sections: Vec<JsSection>,
    }
    #[derive(serde::Deserialize)]
    struct JsSection {
        #[serde(rename = "secTitle")]
        sec_title: String,
        lectures: Vec<JsLecture>,
    }
    #[derive(serde::Deserialize)]
    struct JsLecture {
        id: String,
        #[serde(rename = "lecTitle")]
        lec_title: String,
        url: String,
        duration: Option<String>,
    }

    let root: JsRoot = serde_json::from_str(json_str)
        .map_err(|e| anyhow!("Failed to parse course DOM data: {}", e))?;
    let raw = root.sections;

    let mut sections: Vec<CourseSection> = Vec::new();
    let mut global_idx: u32 = 0;

    for (sec_idx, sec) in raw.iter().enumerate() {
        let mut lectures: Vec<CourseLecture> = Vec::new();
        for lec in &sec.lectures {
            let duration_secs = lec.duration.as_deref().and_then(parse_duration_str);
            let lecture_type = if duration_secs.is_some() {
                LectureType::Video
            } else {
                LectureType::Text
            };
            lectures.push(CourseLecture {
                id: lec.id.clone(),
                title: clean_lecture_title(&lec.lec_title, lec.duration.as_deref()),
                url: lec.url.clone(),
                index: global_idx,
                duration_seconds: duration_secs,
                lecture_type,
                is_free_preview: false,
            });
            global_idx += 1;
        }
        sections.push(CourseSection {
            id: format!("section_{}", sec_idx),
            title: sec.sec_title.clone(),
            index: sec_idx as u32,
            lectures,
        });
    }

    let total_lectures: u32 = sections.iter().map(|s| s.lectures.len() as u32).sum();
    let total_video: u32 = sections
        .iter()
        .flat_map(|s| &s.lectures)
        .filter(|l| l.lecture_type == LectureType::Video)
        .count() as u32;
    let total_duration: f64 = sections
        .iter()
        .flat_map(|s| &s.lectures)
        .filter_map(|l| l.duration_seconds)
        .sum();

    let course_id = url::Url::parse(course_url)
        .ok()
        .and_then(|u| u.path_segments().and_then(|mut s| s.nth(1).map(|x| x.to_string())))
        .unwrap_or_else(|| "unknown".to_string());

    // Prefer title from DOM; fall back to title-casing the URL slug
    let course_title = if !root.course_title.is_empty() {
        root.course_title
    } else {
        course_id
            .replace('-', " ")
            .split_whitespace()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    };

    Ok(CourseInfo {
        id: course_id,
        title: course_title,
        author: "Zero To Mastery".to_string(),
        platform: "zerotomastery".to_string(),
        course_url: course_url.to_string(),
        thumbnail_url: None,
        total_lectures,
        total_video_lectures: total_video,
        total_duration_seconds: if total_duration > 0.0 { Some(total_duration) } else { None },
        sections,
    })
}

fn clean_lecture_title(raw: &str, duration: Option<&str>) -> String {
    // Remove duration string like "(5:43)" and trailing whitespace
    let cleaned = if let Some(dur) = duration {
        raw.replace(&format!("({})", dur), "")
            .replace(dur, "")
    } else {
        raw.to_string()
    };
    cleaned.trim().to_string()
}

fn parse_duration_str(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        2 => {
            let m: f64 = parts[0].parse().ok()?;
            let s: f64 = parts[1].parse().ok()?;
            Some(m * 60.0 + s)
        }
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let s: f64 = parts[2].parse().ok()?;
            Some(h * 3600.0 + m * 60.0 + s)
        }
        _ => None,
    }
}
