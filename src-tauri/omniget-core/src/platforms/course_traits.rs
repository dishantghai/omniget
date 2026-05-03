use async_trait::async_trait;

use crate::models::course::{CourseInfo, LectureMedia};

/// Implemented by platforms that support full-course batch downloads.
/// Must also implement PlatformDownloader for single-lecture downloads.
#[async_trait]
pub trait CourseDownloader: Send + Sync {
    /// Human-readable platform name.
    fn course_platform_name(&self) -> &str;

    /// Returns true if this URL is a course-level URL (not single lecture).
    fn is_course_url(&self, url: &str) -> bool;

    /// Returns true if the downloader can handle this URL at all
    /// (course or lecture level). Delegates to PlatformDownloader::can_handle.
    fn can_handle(&self, url: &str) -> bool;

    /// Returns structured course info (sections + lectures) without downloading.
    /// Requires the user to be logged in via browser cookies.
    async fn get_course_structure(&self, course_url: &str) -> anyhow::Result<CourseInfo>;

    /// Resolves the actual downloadable media URL for one lecture.
    async fn resolve_lecture_media(&self, lecture_url: &str) -> anyhow::Result<LectureMedia>;

    /// Whether this platform requires browser cookie auth.
    fn requires_browser_auth(&self) -> bool {
        true
    }

    /// Login URL to show if not authenticated.
    fn login_url(&self) -> Option<&str> {
        None
    }
}
