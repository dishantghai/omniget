use crate::models::course::{CoursePlatformStatus, KnownCoursePlatform};

/// Static database of known course platforms and their support status.
/// Used to detect unsupported platforms and generate AI agent tasks.
pub fn all_known_platforms() -> Vec<KnownCoursePlatform> {
    vec![
        KnownCoursePlatform {
            name: "Zero To Mastery Academy".to_string(),
            platform_id: "zerotomastery".to_string(),
            domains: vec!["academy.zerotomastery.io".to_string()],
            support_status: CoursePlatformStatus::Supported {
                since_version: "0.5.0".to_string(),
            },
            notes: "Teachable-based LMS. Videos served via Hotmart player embedding private Vimeo. Auth via browser cookies. No DRM.".to_string(),
            login_url: Some("https://academy.zerotomastery.io/sign_in".to_string()),
        },
        KnownCoursePlatform {
            name: "Udemy".to_string(),
            platform_id: "udemy".to_string(),
            domains: vec!["udemy.com".to_string(), "www.udemy.com".to_string()],
            support_status: CoursePlatformStatus::Unsupported {
                yt_dlp_extractor: Some("Udemy (CURRENTLY BROKEN)".to_string()),
            },
            notes: "Uses Widevine DRM on some content. Requires account auth. yt-dlp extractor is broken as of 2026.".to_string(),
            login_url: Some("https://www.udemy.com/join/login-popup/".to_string()),
        },
        KnownCoursePlatform {
            name: "Coursera".to_string(),
            platform_id: "coursera".to_string(),
            domains: vec!["coursera.org".to_string(), "www.coursera.org".to_string()],
            support_status: CoursePlatformStatus::Unsupported {
                yt_dlp_extractor: Some("Coursera".to_string()),
            },
            notes: "Video hosted on Coursera CDN. Requires enrollment. Auth via browser cookies.".to_string(),
            login_url: Some("https://www.coursera.org/login".to_string()),
        },
        KnownCoursePlatform {
            name: "Skillshare".to_string(),
            platform_id: "skillshare".to_string(),
            domains: vec!["skillshare.com".to_string(), "www.skillshare.com".to_string()],
            support_status: CoursePlatformStatus::Unsupported {
                yt_dlp_extractor: None,
            },
            notes: "HLS streams, no DRM. Requires active subscription. Auth via browser cookies.".to_string(),
            login_url: Some("https://www.skillshare.com/login".to_string()),
        },
        KnownCoursePlatform {
            name: "LinkedIn Learning".to_string(),
            platform_id: "linkedin_learning".to_string(),
            domains: vec!["linkedin.com".to_string(), "www.linkedin.com".to_string()],
            support_status: CoursePlatformStatus::Unsupported {
                yt_dlp_extractor: Some("LinkedInLearning".to_string()),
            },
            notes: "Videos via Kaltura CDN. Requires LinkedIn Premium or Learning license.".to_string(),
            login_url: Some("https://www.linkedin.com/learning/".to_string()),
        },
        KnownCoursePlatform {
            name: "Pluralsight".to_string(),
            platform_id: "pluralsight".to_string(),
            domains: vec!["pluralsight.com".to_string(), "www.pluralsight.com".to_string(), "app.pluralsight.com".to_string()],
            support_status: CoursePlatformStatus::Unsupported {
                yt_dlp_extractor: Some("Pluralsight".to_string()),
            },
            notes: "Requires active subscription. Videos via HLS.".to_string(),
            login_url: Some("https://app.pluralsight.com/id/signin/".to_string()),
        },
        KnownCoursePlatform {
            name: "Teachable (Generic)".to_string(),
            platform_id: "teachable".to_string(),
            domains: vec!["teachable.com".to_string()],
            support_status: CoursePlatformStatus::Unsupported {
                yt_dlp_extractor: Some("Teachable (CURRENTLY BROKEN)".to_string()),
            },
            notes: "Generic Teachable schools. Video embedding varies (Vimeo, Hotmart, Wistia, direct). Auth via browser cookies. The zerotomastery implementation is a reference for adding other Teachable schools.".to_string(),
            login_url: None,
        },
        KnownCoursePlatform {
            name: "Thinkific".to_string(),
            platform_id: "thinkific".to_string(),
            domains: vec!["thinkific.com".to_string()],
            support_status: CoursePlatformStatus::Unsupported {
                yt_dlp_extractor: None,
            },
            notes: "Videos via Vimeo or Wistia. Auth via browser cookies.".to_string(),
            login_url: None,
        },
        KnownCoursePlatform {
            name: "Kajabi".to_string(),
            platform_id: "kajabi".to_string(),
            domains: vec!["kajabi.com".to_string(), "mykajabi.com".to_string()],
            support_status: CoursePlatformStatus::Unsupported {
                yt_dlp_extractor: None,
            },
            notes: "Videos via Wistia or Vimeo. Auth via browser cookies.".to_string(),
            login_url: None,
        },
        KnownCoursePlatform {
            name: "Rocketseat".to_string(),
            platform_id: "rocketseat".to_string(),
            domains: vec!["rocketseat.com.br".to_string(), "app.rocketseat.com.br".to_string()],
            support_status: CoursePlatformStatus::Unsupported {
                yt_dlp_extractor: None,
            },
            notes: "Brazilian dev education platform. Videos via Panda Video CDN.".to_string(),
            login_url: Some("https://app.rocketseat.com.br/login".to_string()),
        },
    ]
}

/// Check if a URL matches any known course platform domain.
/// Returns the matching platform if found.
pub fn detect_course_platform(url: &str) -> Option<KnownCoursePlatform> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_lowercase();

    for platform in all_known_platforms() {
        for domain in &platform.domains {
            if host == *domain || host.ends_with(&format!(".{}", domain)) {
                return Some(platform);
            }
        }
    }

    // Heuristic: URL path contains course patterns
    let path = parsed.path().to_lowercase();
    if path.contains("/courses/") || path.contains("/course/") || path.contains("/learn/") || path.contains("/curriculum/") {
        return Some(KnownCoursePlatform {
            name: host.clone(),
            platform_id: "unknown_course".to_string(),
            domains: vec![host],
            support_status: CoursePlatformStatus::Unsupported { yt_dlp_extractor: None },
            notes: "Detected as course URL by path pattern. Platform not in known database.".to_string(),
            login_url: None,
        });
    }

    None
}

/// Returns true if the URL looks like a course (not just a single video lecture).
pub fn is_course_index_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else { return false; };
    let path = parsed.path();

    // ZTM pattern: /courses/{slug} without /lectures/
    // Single lecture: /courses/{slug}/lectures/{id}
    let has_courses = path.contains("/courses/");
    let has_lectures = path.contains("/lectures/");

    has_courses && !has_lectures
}
