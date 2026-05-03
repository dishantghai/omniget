//! OmniGet CLI — download videos and courses from the terminal.
//!
//! Usage:
//!   omniget download <URL> [--quality 1080p] [--output ./downloads]
//!   omniget course <URL> [--output ./downloads] [--quality 720p]
//!   omniget info <URL> [--json]
//!   omniget platforms [--courses] [--json]
//!   omniget task <URL>

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use tokio::sync::mpsc;

use omniget_core::core::known_course_platforms;
use omniget_core::models::course::CoursePlatformStatus;

#[derive(Parser)]
#[command(
    name = "omniget",
    about = "Download videos and courses from 20+ platforms",
    version = env!("CARGO_PKG_VERSION"),
    long_about = "OmniGet CLI — works with any URL OmniGet supports.\nFor AI agent use, add --json for machine-readable output."
)]
struct Cli {
    /// Output machine-readable JSON (for AI agent/scripting use)
    #[arg(long, global = true)]
    json: bool,

    /// Suppress progress bars and non-essential output
    #[arg(long, short, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Download a single video URL
    Download {
        /// The video URL to download
        url: String,

        /// Output directory (default: ~/Downloads)
        #[arg(long, short)]
        output: Option<String>,

        /// Video quality, e.g. "1080p", "720p", "best" (default: best)
        #[arg(long, short)]
        quality: Option<String>,

        /// Download audio only
        #[arg(long)]
        audio_only: bool,

        /// Custom filename template (yt-dlp format)
        #[arg(long)]
        filename: Option<String>,
    },

    /// Download an entire course (requires supported platform + browser login)
    Course {
        /// The course URL (course index page, not a single lecture)
        url: String,

        /// Output directory (default: ~/Downloads)
        #[arg(long, short)]
        output: Option<String>,

        /// Video quality (default: best)
        #[arg(long, short)]
        quality: Option<String>,

        /// Comma-separated section indices to download (e.g. "1,2,5"). Default: all.
        #[arg(long)]
        sections: Option<String>,

        /// Start from this lecture number (1-indexed, skips earlier lectures)
        #[arg(long)]
        from_lecture: Option<u32>,

        /// Skip lectures already downloaded
        #[arg(long)]
        skip_existing: bool,

        /// List course structure without downloading
        #[arg(long)]
        list_only: bool,
    },

    /// Inspect a URL without downloading — shows title, platform, formats
    Info {
        /// The URL to inspect
        url: String,
    },

    /// List all supported platforms
    Platforms {
        /// Show only course platforms
        #[arg(long)]
        courses: bool,
    },

    /// Generate an AI agent task file for adding support for an unsupported course platform
    Task {
        /// The unsupported course URL
        url: String,

        /// Output path for the task file (default: ~/omniget/omniget-tasks/)
        #[arg(long, short)]
        output: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Download {
            url,
            output,
            quality,
            audio_only,
            filename,
        } => {
            cmd_download(&url, output.as_deref(), quality.as_deref(), audio_only, filename.as_deref(), cli.json, cli.quiet).await?;
        }
        Commands::Course {
            url,
            output,
            quality,
            sections,
            from_lecture,
            skip_existing,
            list_only,
        } => {
            cmd_course(
                &url,
                output.as_deref(),
                quality.as_deref(),
                sections.as_deref(),
                from_lecture,
                skip_existing,
                list_only,
                cli.json,
                cli.quiet,
            )
            .await?;
        }
        Commands::Info { url } => {
            cmd_info(&url, cli.json).await?;
        }
        Commands::Platforms { courses } => {
            cmd_platforms(courses, cli.json);
        }
        Commands::Task { url, output } => {
            cmd_task(&url, output.as_deref(), cli.json).await?;
        }
    }

    Ok(())
}

async fn cmd_download(
    url: &str,
    output: Option<&str>,
    quality: Option<&str>,
    audio_only: bool,
    filename: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let output_dir = resolve_output_dir(output);
    std::fs::create_dir_all(&output_dir)?;

    let ytdlp_path = omniget_core::core::ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| anyhow!("yt-dlp unavailable: {}", e))?;

    if !quiet && !json {
        println!("{} {}", "Fetching info:".dimmed(), url);
    }

    let json_info = omniget_core::core::ytdlp::get_video_info(&ytdlp_path, url, &[]).await?;
    let title = json_info
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("video");

    if json {
        println!("{}", serde_json::to_string_pretty(&json_info)?);
        return Ok(());
    }

    if !quiet {
        println!("{} {}", "Downloading:".green().bold(), title);
    }

    let (tx, mut rx) = mpsc::channel::<f64>(32);

    let pb = if !quiet {
        let pb = ProgressBar::new(100);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {percent}% {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );
        Some(pb)
    } else {
        None
    };

    let quality_height = quality.and_then(|q| {
        q.trim_end_matches('p').parse::<u32>().ok()
    });

    let cancel_token = tokio_util::sync::CancellationToken::new();
    let dl_mode = if audio_only { Some("audio") } else { None };

    tokio::spawn({
        let pb = pb.clone();
        async move {
            while let Some(pct) = rx.recv().await {
                if let Some(ref pb) = pb {
                    if pct >= 0.0 {
                        pb.set_position(pct as u64);
                    }
                }
            }
        }
    });

    let result = omniget_core::core::ytdlp::download_video(
        &ytdlp_path,
        url,
        &output_dir,
        quality_height,
        tx,
        dl_mode,
        None,
        filename,
        None,
        cancel_token,
        None,
        4,
        false,
        &[],
    )
    .await?;

    if let Some(pb) = pb {
        pb.finish_with_message("Done");
    }

    if !quiet && !json {
        println!(
            "{} {} ({})",
            "✓".green().bold(),
            result.file_path.display(),
            format_bytes(result.file_size_bytes)
        );
    }
    if json {
        println!(
            "{}",
            serde_json::json!({
                "event": "complete",
                "file": result.file_path.to_string_lossy(),
                "bytes": result.file_size_bytes,
            })
        );
    }

    Ok(())
}

async fn cmd_course(
    url: &str,
    _output: Option<&str>,
    _quality: Option<&str>,
    _sections_filter: Option<&str>,
    _from_lecture: Option<u32>,
    _skip_existing: bool,
    _list_only: bool,
    json: bool,
    _quiet: bool,
) -> Result<()> {
    // Check if it's a known supported course platform
    let known = known_course_platforms::detect_course_platform(url);
    let is_supported = matches!(
        known.as_ref().map(|p| &p.support_status),
        Some(CoursePlatformStatus::Supported { .. })
    );

    if !is_supported {
        let platform_name = known
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("this platform");
        eprintln!(
            "{} {} is not yet supported.",
            "✗".red().bold(),
            platform_name
        );
        eprintln!(
            "  Run {} to generate an AI agent task to add support.",
            format!("omniget task {}", url).cyan()
        );
        std::process::exit(1);
    }

    // For now, ZTM is the only supported course platform
    if !url.contains("academy.zerotomastery.io") {
        eprintln!("{} Unsupported course platform.", "✗".red());
        std::process::exit(1);
    }

    if !json {
        println!("{} Fetching course structure...", "→".cyan());
    }

    // Since ZtmDownloader lives in the main omniget crate (not omniget-core), the CLI
    // uses a subprocess approach for course downloads:
    // It calls itself recursively via the ZTM platform. For the MVP, we document this.
    println!(
        "{}",
        "Course downloads via CLI require the OmniGet app to be running as a daemon.".yellow()
    );
    println!("This feature is in development. Use the OmniGet GUI to download courses for now.");
    println!();
    println!("Planned CLI workflow:");
    println!("  omniget course {} --list-only", url);
    println!("  omniget course {} --output ~/courses --quality 1080p", url);

    Ok(())
}

async fn cmd_info(url: &str, json: bool) -> Result<()> {
    let ytdlp_path = omniget_core::core::ytdlp::ensure_ytdlp().await?;

    // Check if it's a course platform
    let course_platform = known_course_platforms::detect_course_platform(url);

    if let Some(ref platform) = course_platform {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "type": "course_platform",
                    "platform": platform.name,
                    "platform_id": platform.platform_id,
                    "support_status": format!("{:?}", platform.support_status),
                    "url": url,
                }))?
            );
            return Ok(());
        }

        let status_str = match &platform.support_status {
            CoursePlatformStatus::Supported { since_version } => {
                format!("{} (since v{})", "Supported".green(), since_version)
            }
            CoursePlatformStatus::Unsupported { .. } => "Not yet supported".red().to_string(),
            CoursePlatformStatus::PartiallySupported { limitations } => {
                format!("{}: {}", "Partial".yellow(), limitations)
            }
        };

        println!("{}: {}", "Platform".bold(), platform.name);
        println!("{}: {}", "Status".bold(), status_str);
        println!("{}: {}", "Notes".bold(), platform.notes);
        if matches!(
            platform.support_status,
            CoursePlatformStatus::Supported { .. }
        ) {
            println!();
            println!(
                "Use {} to download this course.",
                "omniget course <URL>".cyan()
            );
        }
        return Ok(());
    }

    // Regular video URL
    if !json {
        println!("{} {}", "Fetching info:".dimmed(), url);
    }
    let info = omniget_core::core::ytdlp::get_video_info(&ytdlp_path, url, &[]).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }

    let title = info.get("title").and_then(|v| v.as_str()).unwrap_or("?");
    let uploader = info
        .get("uploader")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let duration = info.get("duration").and_then(|v| v.as_f64());

    println!("{}: {}", "Title".bold(), title);
    println!("{}: {}", "Uploader".bold(), uploader);
    if let Some(dur) = duration {
        println!("{}: {}", "Duration".bold(), format_duration(dur));
    }

    let formats = omniget_core::core::ytdlp::parse_formats(&info);
    if !formats.is_empty() {
        println!("{}", "\nFormats:".bold());
        for fmt in &formats {
            if fmt.has_video {
                println!(
                    "  {} — {} {:?}",
                    fmt.format_id,
                    fmt.resolution
                        .as_deref()
                        .unwrap_or("?"),
                    fmt.vcodec.as_deref().unwrap_or("?"),
                );
            }
        }
    }

    Ok(())
}

fn cmd_platforms(courses_only: bool, json: bool) {
    let platforms = known_course_platforms::all_known_platforms();

    if json {
        println!("{}", serde_json::to_string_pretty(&platforms).unwrap_or_default());
        return;
    }

    println!("{}", "Course Platforms:".bold().underline());
    println!();
    for p in &platforms {
        let status = match &p.support_status {
            CoursePlatformStatus::Supported { since_version } => {
                format!("{} (v{})", "✓ Supported".green(), since_version)
            }
            CoursePlatformStatus::Unsupported { yt_dlp_extractor: Some(ext) } => {
                format!("{} [yt-dlp: {}]", "✗ Unsupported".red(), ext)
            }
            CoursePlatformStatus::Unsupported { .. } => "✗ Unsupported".red().to_string(),
            CoursePlatformStatus::PartiallySupported { limitations } => {
                format!("{} ({})", "~ Partial".yellow(), limitations)
            }
        };
        println!("  {} — {}", p.name.bold(), status);
        for domain in &p.domains {
            println!("    {}", domain.dimmed());
        }
    }

    if !courses_only {
        println!();
        println!("{}", "Video Platforms (via yt-dlp):".bold().underline());
        println!("  YouTube, Vimeo, Twitter/X, TikTok, Instagram, Reddit, Twitch,");
        println!("  Bluesky, Pinterest, Bilibili, and 1000+ others via yt-dlp generic");
    }
}

async fn cmd_task(url: &str, output: Option<&str>, json: bool) -> Result<()> {
    let platform = known_course_platforms::detect_course_platform(url)
        .ok_or_else(|| anyhow!("Could not detect course platform for URL: {}", url))?;

    let task_dir = if let Some(out) = output {
        PathBuf::from(out)
    } else {
        dirs::home_dir()
            .ok_or_else(|| anyhow!("Could not find home directory"))?
            .join("omniget")
            .join("omniget-tasks")
    };

    std::fs::create_dir_all(&task_dir)?;

    let filename = format!(
        "omniget-task-{}.md",
        platform.platform_id.replace(' ', "_").to_lowercase()
    );
    let task_path = task_dir.join(&filename);
    let domain = platform.domains.first().map(|s| s.as_str()).unwrap_or("");
    let content = build_task_content(&platform.name, &platform.platform_id, url, domain);
    std::fs::write(&task_path, &content)?;

    let path_str = task_path.to_string_lossy().to_string();

    if json {
        println!(
            "{}",
            serde_json::json!({
                "event": "task_generated",
                "path": path_str,
                "platform": platform.name,
            })
        );
    } else {
        println!("{} Task file created:", "✓".green().bold());
        println!("  {}", path_str.cyan());
        println!();
        println!(
            "Open with: {} '{}'",
            "open".dimmed(),
            path_str
        );
    }

    let _ = open::that(&task_path);
    Ok(())
}

fn build_task_content(name: &str, id: &str, url: &str, domain: &str) -> String {
    format!(
        r#"# OmniGet: Add {name} Course Download Support

Generated: {date}
Sample URL: {url}
Domain: {domain}

## Quick Start for AI Agent (Claude Code)

```bash
cd /Users/Shared/ALL_WORKSPACE/ai_tools/omniget
```

## Investigation Steps

Use browser-harness to map the video tech stack while logged into {name}:

```bash
browser-harness -c '
new_tab("{url}")
wait_for_load()
import time; time.sleep(3)
iframes = js("return Array.from(document.querySelectorAll(\"iframe\")).map(f => ({{src: f.src}}))")
print("Iframes:", iframes)
targets = cdp("Target.getTargets", {{}})
print("Targets:", [t for t in targets.get("targetInfos",[]) if "player" in t.get("url","")])
'
```

## Implementation

Follow the ZTM reference implementation at `src-tauri/src/platforms/ztm/`.

Files to create:
- `src-tauri/src/platforms/{id}/mod.rs`
- `src-tauri/src/platforms/{id}/auth.rs`
- `src-tauri/src/platforms/{id}/api.rs`

Files to modify:
- `src-tauri/src/platforms/mod.rs` — add module
- `src-tauri/src/lib.rs` — register in registry + course_registry
- `src-tauri/omniget-core/src/platforms/mod.rs` — add Platform variant
- `src-tauri/omniget-core/src/core/known_course_platforms.rs` — update to Supported
- `docs/COURSE_PLATFORMS.md` — add platform section

## Acceptance Criteria

- [ ] `cargo check` passes
- [ ] Course structure returns correct section/lecture count for sample URL
- [ ] All video lectures download correctly with proper filenames
- [ ] Update known_course_platforms.rs to Supported status
"#,
        name = name,
        id = id.to_lowercase().replace(' ', "_"),
        url = url,
        domain = domain,
        date = chrono::Utc::now().format("%Y-%m-%d"),
    )
}

fn resolve_output_dir(output: Option<&str>) -> PathBuf {
    if let Some(path) = output {
        PathBuf::from(path)
    } else {
        dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn format_duration(secs: f64) -> String {
    let s = secs as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, sec)
    } else {
        format!("{}:{:02}", m, sec)
    }
}
