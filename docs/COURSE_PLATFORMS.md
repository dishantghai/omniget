# OmniGet — Course Platform Downloads

OmniGet supports batch downloading of entire courses from supported LMS platforms.

## How It Works

When you paste a course URL into OmniGet, it:
1. Detects the platform from the URL
2. Checks if the platform is supported
3. If supported: fetches course structure (sections + lectures) using your browser's login cookies
4. Shows a preview panel with all sections and lectures to select
5. Downloads selected lectures sequentially with proper folder organization

Output structure:
```
~/Downloads/Course Title/
  S01 - Introduction/
    01 - Welcome.mp4
    02 - Overview.mp4
  S02 - Getting Started/
    03 - Setup.mp4
    ...
```

## Authentication

OmniGet reads your login cookies directly from Chrome. **You must be logged in to the course platform in Chrome** before using OmniGet's course download feature.

Cookie extraction uses two methods (in order):
1. **CDP via browser-harness** — connects to the running Chrome instance via Chrome DevTools Protocol (fastest, requires browser-harness to be installed)
2. **yt-dlp** — uses `--cookies-from-browser chrome` as a fallback

## Supported Platforms

### Zero To Mastery Academy (`academy.zerotomastery.io`)

**Status**: Supported since v0.5.0

**Tech stack**: Teachable LMS → Hotmart video player → Private Vimeo embed (HLS, no DRM)

**Usage**:
- Paste any ZTM course URL into OmniGet (e.g. `https://academy.zerotomastery.io/courses/machine-learning-with-hugging-face`)
- Or paste a single lecture URL — OmniGet detects it as ZTM and offers course mode
- Select sections/lectures in the preview panel
- Choose output directory and quality

**Limitations**:
- Requires Chrome login to academy.zerotomastery.io
- Rate limiting may apply if downloading many lectures concurrently (OmniGet downloads sequentially to avoid this)

---

## Unsupported Platforms

These platforms are known but not yet supported. Use **"Generate AI Task"** in OmniGet to create a precise implementation task file.

| Platform | Status | Notes |
|----------|--------|-------|
| Udemy | Unsupported | yt-dlp extractor broken; Widevine DRM on some content |
| Coursera | Unsupported | Enrollment required |
| Skillshare | Unsupported | HLS, no DRM — good candidate |
| LinkedIn Learning | Unsupported | Kaltura CDN |
| Pluralsight | Unsupported | Subscription required |
| Thinkific | Unsupported | Vimeo/Wistia embed |
| Kajabi | Unsupported | Wistia/Vimeo embed |
| Rocketseat | Unsupported | Panda Video CDN |

## Adding a New Platform

1. Paste a course URL from the unsupported platform into OmniGet
2. OmniGet detects it as a known-but-unsupported platform
3. Click **"Generate AI Task"** — creates `~/omniget/omniget-tasks/omniget-task-{platform}.md`
4. Open the task file with Claude Code: `claude` then `/review ~/omniget/omniget-tasks/omniget-task-{platform}.md`
5. Claude Code will investigate the platform, implement the plugin, and test it

Or from the CLI:
```bash
omniget task https://www.skillshare.com/en/courses/...
# Creates ~/omniget/omniget-tasks/omniget-task-skillshare.md
```

## CLI Reference

```bash
# Download a course
omniget course https://academy.zerotomastery.io/courses/...

# List course structure without downloading
omniget course https://academy.zerotomastery.io/courses/... --list-only

# Download specific sections
omniget course ... --sections "1,2,3"

# Start from a specific lecture
omniget course ... --from-lecture 15

# JSON output (for AI agents)
omniget course ... --json

# Generate task for unsupported platform
omniget task https://www.udemy.com/course/...
```

## Architecture (for contributors)

The course platform system uses two Rust traits:

**`PlatformDownloader`** (omniget-core) — handles single lecture downloads via the normal queue.

**`CourseDownloader`** (omniget-core) — extends PlatformDownloader with:
- `get_course_structure(url)` → `CourseInfo` (sections + lectures)
- `resolve_lecture_media(url)` → `LectureMedia` (final download URL + auth headers)
- `is_course_url(url)` → bool
- `can_handle(url)` → bool

Reference implementation: `src-tauri/src/platforms/ztm/`
