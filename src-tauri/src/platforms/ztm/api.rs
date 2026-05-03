//! ZTM Teachable API + Hotmart embed parsing.

use anyhow::anyhow;
use serde::Deserialize;

use omniget_core::models::course::{
    CourseInfo, CourseSection, CourseLecture, LectureType,
};

#[derive(Debug, Deserialize)]
pub struct HotmartCredentials {
    pub video_id: String,
    pub signature: String,
    pub teachable_application_key: String, // this is the token
    #[allow(dead_code)]
    pub user_email: Option<String>,
    #[allow(dead_code)]
    pub duration: Option<f64>,
}

/// Scrape the course sidebar to get all sections and lectures.
pub async fn fetch_course_structure(
    client: &reqwest::Client,
    course_url: &str,
) -> anyhow::Result<CourseInfo> {
    let resp = client
        .get(course_url)
        .send()
        .await
        .map_err(|e| anyhow!("Failed to fetch course page: {}", e))?;

    if resp.status() == 401 || resp.status() == 403 {
        return Err(anyhow!(
            "Not authenticated. Please log in to academy.zerotomastery.io in Chrome."
        ));
    }

    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {} fetching course page", resp.status()));
    }

    let html = resp.text().await?;
    parse_course_from_html(&html, course_url)
}

fn parse_course_from_html(html: &str, course_url: &str) -> anyhow::Result<CourseInfo> {
    use regex::Regex;

    // Extract course title from <title> or og:title
    let title_re = Regex::new(r#"<title[^>]*>([^<]+)</title>"#).unwrap();
    let course_title = title_re
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| {
            m.as_str()
                .trim()
                .trim_end_matches("| Zero To")
                .trim()
                .to_string()
        })
        .unwrap_or_else(|| "Course".to_string());

    // Extract base URL for lecture links
    let base_url = {
        let parsed = url::Url::parse(course_url)?;
        format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""))
    };

    // Parse course sections and lectures from the sidebar HTML
    // Teachable sidebar pattern: .course-section > .section-title + a.item links
    let section_re = Regex::new(
        r#"<div[^>]*class="[^"]*course-section[^"]*"[^>]*>([\s\S]*?)(?=<div[^>]*class="[^"]*course-section|$)"#
    ).unwrap();

    let section_title_re = Regex::new(r#"<span[^>]*class="[^"]*section-title[^"]*"[^>]*>([\s\S]*?)</span>"#).unwrap();
    let lecture_re = Regex::new(
        r#"<a[^>]*href="(/courses/[^"]+/lectures/(\d+))"[^>]*>[\s\S]*?<span[^>]*class="[^"]*lecture-name[^"]*"[^>]*>([\s\S]*?)</span>"#
    ).unwrap();
    let duration_re = Regex::new(r#"\((\d+:\d+)\)"#).unwrap();

    let mut sections: Vec<CourseSection> = Vec::new();
    let mut section_idx: u32 = 0;
    let mut lecture_global_idx: u32 = 0;

    for sec_cap in section_re.captures_iter(html) {
        let section_html = &sec_cap[1];

        let sec_title = section_title_re
            .captures(section_html)
            .and_then(|c| c.get(1))
            .map(|m| strip_html_tags(m.as_str()).trim().to_string())
            .unwrap_or_else(|| format!("Section {}", section_idx + 1));

        let mut lectures: Vec<CourseLecture> = Vec::new();

        for lec_cap in lecture_re.captures_iter(section_html) {
            let path = &lec_cap[1];
            let id = lec_cap[2].to_string();
            let raw_title = strip_html_tags(&lec_cap[3]);
            let title_clean = raw_title.trim().to_string();

            // Extract duration if present
            let duration_seconds = duration_re
                .captures(&title_clean)
                .and_then(|c| c.get(1))
                .and_then(|m| parse_duration(m.as_str()));

            // Clean duration from title
            let title = duration_re.replace(&title_clean, "").trim().to_string();

            let has_duration = duration_seconds.is_some();
            let lecture_type = if has_duration {
                LectureType::Video
            } else {
                LectureType::Text
            };

            lectures.push(CourseLecture {
                id: id.clone(),
                title,
                url: format!("{}{}", base_url, path),
                index: lecture_global_idx,
                duration_seconds,
                lecture_type,
                is_free_preview: false,
            });
            lecture_global_idx += 1;
        }

        if !lectures.is_empty() {
            sections.push(CourseSection {
                id: format!("section_{}", section_idx),
                title: sec_title,
                index: section_idx,
                lectures,
            });
            section_idx += 1;
        }
    }

    let total_lectures = sections.iter().map(|s| s.lectures.len() as u32).sum();
    let total_video = sections
        .iter()
        .flat_map(|s| &s.lectures)
        .filter(|l| l.lecture_type == LectureType::Video)
        .count() as u32;

    let total_duration = sections
        .iter()
        .flat_map(|s| &s.lectures)
        .filter_map(|l| l.duration_seconds)
        .sum::<f64>();

    // Extract course ID from URL
    let course_id = url::Url::parse(course_url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut segs| segs.nth(1).map(|s| s.to_string()))
        })
        .unwrap_or_else(|| "unknown".to_string());

    Ok(CourseInfo {
        id: course_id,
        title: course_title,
        author: "Zero To Mastery".to_string(),
        platform: "zerotomastery".to_string(),
        course_url: course_url.to_string(),
        thumbnail_url: None,
        total_lectures,
        total_video_lectures: total_video,
        total_duration_seconds: if total_duration > 0.0 {
            Some(total_duration)
        } else {
            None
        },
        sections,
    })
}

/// Get the attachment_id for a lecture by scraping the lecture page.
pub async fn get_attachment_id(
    client: &reqwest::Client,
    lecture_url: &str,
) -> anyhow::Result<String> {
    let html = client
        .get(lecture_url)
        .send()
        .await
        .map_err(|e| anyhow!("Failed to fetch lecture page: {}", e))?
        .text()
        .await?;

    // Look for attachment_id in the page JSON/scripts
    let re = regex::Regex::new(r#"["\']attachment_id["\']\s*[=:]\s*["']?(\d+)["']?"#).unwrap();
    if let Some(cap) = re.captures(&html) {
        return Ok(cap[1].to_string());
    }

    // Alternative: look for hotmart private_video API call pattern
    let re2 =
        regex::Regex::new(r#"hotmart/private_video\?attachment_id=(\d+)"#).unwrap();
    if let Some(cap) = re2.captures(&html) {
        return Ok(cap[1].to_string());
    }

    // Try to get from iframe src in page
    let re3 = regex::Regex::new(
        r#"metadata.*?attachment_id.*?([0-9]{6,})"#,
    ).unwrap();
    if let Some(cap) = re3.captures(&html) {
        return Ok(cap[1].to_string());
    }

    Err(anyhow!(
        "Could not find attachment_id on lecture page: {}",
        lecture_url
    ))
}

/// Call ZTM's private_video API to get Hotmart credentials.
pub async fn get_hotmart_credentials(
    client: &reqwest::Client,
    base_url: &str,
    attachment_id: &str,
) -> anyhow::Result<HotmartCredentials> {
    let api_url = format!(
        "{}/api/v2/hotmart/private_video?attachment_id={}",
        base_url, attachment_id
    );

    let resp = client
        .get(&api_url)
        .header("Accept", "application/json")
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .map_err(|e| anyhow!("Hotmart credentials API failed: {}", e))?;

    if resp.status() == 401 || resp.status() == 403 {
        return Err(anyhow!(
            "Session expired or not authenticated. Please log in to ZTM in Chrome."
        ));
    }
    if !resp.status().is_success() {
        return Err(anyhow!(
            "HTTP {} from Hotmart credentials API",
            resp.status()
        ));
    }

    let creds: HotmartCredentials = resp
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse Hotmart credentials: {}", e))?;

    Ok(creds)
}

/// Load the Hotmart embed page to extract the nested Vimeo player URL.
pub async fn resolve_vimeo_url(
    client: &reqwest::Client,
    creds: &HotmartCredentials,
) -> anyhow::Result<String> {
    // First try: use browser-harness to find already-loaded Vimeo iframe in Chrome
    if let Ok(Some(vimeo_url)) = crate::platforms::ztm::auth::find_vimeo_embed_url_in_browser().await {
        tracing::info!("[ztm] resolved Vimeo URL from browser CDP");
        return Ok(vimeo_url);
    }

    // Second try: fetch the Hotmart embed page and extract Vimeo iframe src
    let embed_url = format!(
        "https://player.hotmart.com/embed/{}?signature={}&token={}&user=",
        creds.video_id, creds.signature, creds.teachable_application_key
    );

    let resp = client
        .get(&embed_url)
        .header("Referer", "https://academy.zerotomastery.io/")
        .header(
            "Origin",
            "https://academy.zerotomastery.io",
        )
        .send()
        .await
        .map_err(|e| anyhow!("Failed to fetch Hotmart embed: {}", e))?;

    if !resp.status().is_success() {
        return Err(anyhow!(
            "Hotmart embed returned HTTP {} — may need user_id parameter",
            resp.status()
        ));
    }

    let html = resp.text().await?;

    // Extract Vimeo iframe src from Hotmart player HTML
    let re = regex::Regex::new(
        r#"(?:src|href)=["'](https://player\.vimeo\.com/video/\d+[^"']+)["']"#,
    ).unwrap();
    if let Some(cap) = re.captures(&html) {
        return Ok(cap[1].to_string());
    }

    // Try JSON data in page scripts
    let re2 = regex::Regex::new(
        r#"["\']url["\']\s*:\s*["'](https://player\.vimeo\.com/video/[^"']+)["']"#,
    ).unwrap();
    if let Some(cap) = re2.captures(&html) {
        return Ok(cap[1].to_string());
    }

    Err(anyhow!(
        "Could not extract Vimeo URL from Hotmart embed page. \
         Try opening the lecture in Chrome first so OmniGet can detect it automatically."
    ))
}

/// Extract the lecture title from the page HTML.
pub async fn get_lecture_title(
    client: &reqwest::Client,
    lecture_url: &str,
) -> anyhow::Result<String> {
    let html = client.get(lecture_url).send().await?.text().await?;
    let re = regex::Regex::new(r#"<title[^>]*>([^<]+)</title>"#).unwrap();
    Ok(re
        .captures(&html)
        .and_then(|c| c.get(1))
        .map(|m| {
            m.as_str()
                .trim()
                .split('|')
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_else(|| "Lecture".to_string()))
}

fn strip_html_tags(html: &str) -> String {
    let re = regex::Regex::new(r#"<[^>]+>"#).unwrap();
    re.replace_all(html, "").to_string()
}

fn parse_duration(s: &str) -> Option<f64> {
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
