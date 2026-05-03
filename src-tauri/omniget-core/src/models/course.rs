use serde::{Deserialize, Serialize};

/// Structured info about an entire course returned by get_course_structure().
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseInfo {
    pub id: String,
    pub title: String,
    pub author: String,
    pub platform: String,
    pub course_url: String,
    pub thumbnail_url: Option<String>,
    pub total_lectures: u32,
    pub total_video_lectures: u32,
    pub total_duration_seconds: Option<f64>,
    pub sections: Vec<CourseSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseSection {
    pub id: String,
    pub title: String,
    pub index: u32,
    pub lectures: Vec<CourseLecture>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseLecture {
    pub id: String,
    pub title: String,
    pub url: String,
    pub index: u32,
    pub duration_seconds: Option<f64>,
    pub lecture_type: LectureType,
    pub is_free_preview: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LectureType {
    Video,
    Text,
    Quiz,
    Assignment,
    Pdf,
    ExternalLink,
    Unknown,
}

/// The resolved downloadable media for a single lecture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LectureMedia {
    pub lecture_id: String,
    pub title: String,
    pub video_url: String,
    pub referer: Option<String>,
    pub duration_seconds: Option<f64>,
    /// Raw `Cookie: ...` header value extracted from the browser session.
    /// Required for CDNs that validate session cookies in addition to URL tokens.
    pub cookie_header: Option<String>,
}

/// Info about a known course platform (supported or not).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownCoursePlatform {
    pub name: String,
    pub platform_id: String,
    pub domains: Vec<String>,
    pub support_status: CoursePlatformStatus,
    pub notes: String,
    pub login_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoursePlatformStatus {
    Supported { since_version: String },
    Unsupported { yt_dlp_extractor: Option<String> },
    PartiallySupported { limitations: String },
}
