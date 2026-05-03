<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";

  type CourseLecture = {
    id: string;
    title: string;
    url: string;
    index: number;
    duration_seconds: number | null;
    lecture_type: string;
    is_free_preview: boolean;
  };

  type CourseSection = {
    id: string;
    title: string;
    index: number;
    lectures: CourseLecture[];
  };

  type CourseInfo = {
    id: string;
    title: string;
    author: string;
    platform: string;
    course_url: string;
    thumbnail_url: string | null;
    total_lectures: number;
    total_video_lectures: number;
    total_duration_seconds: number | null;
    sections: CourseSection[];
  };

  let {
    courseInfo,
    onDismiss,
    onDownloadStarted,
  }: {
    courseInfo: CourseInfo;
    onDismiss: () => void;
    onDownloadStarted: (ids: number[]) => void;
  } = $props();

  let videoLectureIds = $derived(
    courseInfo.sections
      .flatMap((s) => s.lectures)
      .filter((l) => l.lecture_type === "video")
      .map((l) => l.id)
  );

  let selectedIds = $state<Set<string>>(new Set<string>());
  let expandedSections = $state<Set<string>>(new Set<string>());

  // Initialize selections once when courseInfo arrives (avoids Svelte 5 stale-ref warning).
  $effect(() => {
    selectedIds = new Set(videoLectureIds);
    expandedSections = new Set(courseInfo.sections.map((s) => s.id));
  });

  let quality = $state("best");
  let downloading = $state(false);
  let queuingProgress = $state<{ current: number; total: number; title: string } | null>(null);

  $effect(() => {
    if (!downloading) { queuingProgress = null; return; }
    let unlisten: (() => void) | undefined;
    listen<{ current: number; total: number; title: string }>(
      "course-queuing-progress",
      (e) => { queuingProgress = e.payload; }
    ).then((fn) => { unlisten = fn; });
    return () => unlisten?.();
  });

  let videoLectures = $derived(
    courseInfo.sections.flatMap((s) => s.lectures).filter((l) => l.lecture_type === "video")
  );

  let selectedVideoCount = $derived(
    videoLectures.filter((l) => selectedIds.has(l.id)).length
  );

  let allSelected = $derived(selectedVideoCount === videoLectures.length && videoLectures.length > 0);
  let someSelected = $derived(selectedVideoCount > 0 && !allSelected);

  function toggleSelectAll() {
    if (allSelected) {
      selectedIds = new Set();
    } else {
      selectedIds = new Set(videoLectures.map((l) => l.id));
    }
  }

  function toggleSection(section: CourseSection) {
    const videoIds = section.lectures
      .filter((l) => l.lecture_type === "video")
      .map((l) => l.id);
    const allInSection = videoIds.every((id) => selectedIds.has(id));
    const next = new Set(selectedIds);
    if (allInSection) {
      videoIds.forEach((id) => next.delete(id));
    } else {
      videoIds.forEach((id) => next.add(id));
    }
    selectedIds = next;
  }

  function toggleLecture(id: string) {
    const next = new Set(selectedIds);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    selectedIds = next;
  }

  function toggleSectionExpand(id: string) {
    const next = new Set(expandedSections);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    expandedSections = next;
  }

  function formatDuration(seconds: number | null): string {
    if (!seconds) return "";
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    if (m >= 60) {
      const h = Math.floor(m / 60);
      const rem = m % 60;
      return `${h}h ${rem}m`;
    }
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  function formatTotalDuration(seconds: number | null): string {
    if (!seconds) return "";
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    if (h > 0) return `${h}h ${m}m`;
    return `${m}m`;
  }

  function sectionVideoCount(section: CourseSection): number {
    return section.lectures.filter((l) => l.lecture_type === "video").length;
  }

  function sectionSelectedCount(section: CourseSection): number {
    return section.lectures.filter(
      (l) => l.lecture_type === "video" && selectedIds.has(l.id)
    ).length;
  }

  async function handleDownload() {
    if (selectedVideoCount === 0) return;
    downloading = true;

    try {
      const settings = getSettings();
      let outputDir = settings?.download.default_output_dir ?? "";

      if (settings?.download.always_ask_path || !outputDir) {
        const selected = await open({
          directory: true,
          title: $t("course.choose_output_dir"),
        });
        if (!selected) {
          downloading = false;
          return;
        }
        outputDir = selected;
      }

      const ids = await invoke<number[]>("start_course_download", {
        courseInfo,
        lectureIds: [...selectedIds],
        outputDir,
        quality,
      });

      onDownloadStarted(ids);
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    } finally {
      downloading = false;
    }
  }

  function getLectureTypeIcon(type: string): string {
    switch (type) {
      case "video":
        return `<path d="M23 7l-7 5 7 5V7z"/><rect x="1" y="5" width="15" height="14" rx="2" ry="2"/>`;
      case "text":
        return `<line x1="21" y1="10" x2="3" y2="10"/><line x1="21" y1="6" x2="3" y2="6"/><line x1="21" y1="14" x2="3" y2="14"/><line x1="21" y1="18" x2="3" y2="18"/>`;
      case "quiz":
        return `<circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 015.83 1c0 2-3 3-3 3"/><line x1="12" y1="17" x2="12" y2="17"/>`;
      case "pdf":
        return `<path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="8" y1="13" x2="16" y2="13"/>`;
      default:
        return `<circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="16"/><line x1="8" y1="12" x2="16" y2="12"/>`;
    }
  }
</script>

<div class="course-preview">
  <div class="course-header">
    {#if courseInfo.thumbnail_url}
      <img
        src={courseInfo.thumbnail_url}
        alt=""
        class="course-thumb"
        loading="lazy"
        onerror={(e) => { (e.target as HTMLImageElement).style.display = "none"; }}
      />
    {/if}
    <div class="course-meta">
      <div class="platform-badge">{courseInfo.platform}</div>
      <h2 class="course-title">{courseInfo.title}</h2>
      <div class="course-stats">
        <span>{$t("course.by_author", { author: courseInfo.author })}</span>
        <span class="meta-sep">&middot;</span>
        <span>{$t("course.video_count", { count: String(courseInfo.total_video_lectures) })}</span>
        {#if courseInfo.total_duration_seconds}
          <span class="meta-sep">&middot;</span>
          <span>{formatTotalDuration(courseInfo.total_duration_seconds)}</span>
        {/if}
      </div>
    </div>
    <button class="dismiss-btn" onclick={onDismiss} aria-label={$t("common.close")}>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M18 6L6 18M6 6l12 12" />
      </svg>
    </button>
  </div>

  <div class="select-all-row">
    <label class="select-all-label">
      <input
        type="checkbox"
        checked={allSelected}
        indeterminate={someSelected}
        onchange={toggleSelectAll}
        aria-label={$t("course.select_all")}
      />
      <span>{$t("course.select_all")}</span>
    </label>
    <span class="selection-count">
      {$t("course.selected_count", { count: String(selectedVideoCount), total: String(videoLectures.length) })}
    </span>
  </div>

  <div class="sections-list" role="list">
    {#each courseInfo.sections as section (section.id)}
      {@const videoCount = sectionVideoCount(section)}
      {@const selectedCount = sectionSelectedCount(section)}
      {@const expanded = expandedSections.has(section.id)}
      <div class="section" role="listitem">
        <div class="section-header">
          <label class="section-checkbox">
            <input
              type="checkbox"
              checked={videoCount > 0 && selectedCount === videoCount}
              indeterminate={selectedCount > 0 && selectedCount < videoCount}
              onchange={() => toggleSection(section)}
              disabled={videoCount === 0}
              aria-label={$t("course.select_section", { title: section.title })}
            />
          </label>
          <button
            class="section-toggle"
            onclick={() => toggleSectionExpand(section.id)}
            aria-expanded={expanded}
            aria-label={section.title}
          >
            <svg
              class="chevron"
              class:expanded
              viewBox="0 0 24 24"
              width="14"
              height="14"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <polyline points="9 18 15 12 9 6" />
            </svg>
            <span class="section-title">{section.title}</span>
            <span class="section-count">
              {#if videoCount > 0}
                {selectedCount}/{videoCount}
              {/if}
            </span>
          </button>
        </div>

        {#if expanded}
          <div class="lecture-list" role="list">
            {#each section.lectures as lecture (lecture.id)}
              {@const isVideo = lecture.lecture_type === "video"}
              <label class="lecture-item" class:non-video={!isVideo} role="listitem">
                <input
                  type="checkbox"
                  checked={selectedIds.has(lecture.id)}
                  onchange={() => toggleLecture(lecture.id)}
                  disabled={!isVideo}
                  aria-label={lecture.title}
                />
                <svg
                  class="lecture-type-icon"
                  viewBox="0 0 24 24"
                  width="13"
                  height="13"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.8"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  {@html getLectureTypeIcon(lecture.lecture_type)}
                </svg>
                <span class="lecture-title">{lecture.title}</span>
                {#if lecture.duration_seconds}
                  <span class="lecture-duration">{formatDuration(lecture.duration_seconds)}</span>
                {/if}
              </label>
            {/each}
          </div>
        {/if}
      </div>
    {/each}
  </div>

  <div class="download-footer">
    <div class="quality-row">
      <label class="quality-label" for="course-quality">{$t("course.quality")}</label>
      <select id="course-quality" class="quality-select" bind:value={quality}>
        <option value="best">{$t("course.quality_best")}</option>
        <option value="1080p">1080p</option>
        <option value="720p">720p</option>
        <option value="480p">480p</option>
        <option value="360p">360p</option>
      </select>
    </div>
    <button
      class="download-btn"
      onclick={handleDownload}
      disabled={selectedVideoCount === 0 || downloading}
    >
      {#if downloading}
        <span class="btn-spinner"></span>
        {#if queuingProgress}
          {queuingProgress.current}/{queuingProgress.total}: {queuingProgress.title.length > 28 ? queuingProgress.title.slice(0, 28) + "…" : queuingProgress.title}
        {:else}
          {$t("course.queuing")}
        {/if}
      {:else}
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
          <polyline points="7 10 12 15 17 10" />
          <line x1="12" y1="15" x2="12" y2="3" />
        </svg>
        {$t("course.download_selected", { count: String(selectedVideoCount) })}
      {/if}
    </button>
  </div>
</div>

<style>
  .course-preview {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: 560px;
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    overflow: hidden;
    animation: slideIn 250ms cubic-bezier(0.34, 1.56, 0.64, 1);
  }

  @keyframes slideIn {
    from { opacity: 0; transform: translateY(8px) scale(0.98); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  .course-header {
    display: flex;
    gap: var(--padding);
    padding: var(--padding);
    border-bottom: 1px solid var(--content-border);
    align-items: flex-start;
  }

  .course-thumb {
    width: 72px;
    height: 48px;
    object-fit: cover;
    border-radius: calc(var(--border-radius) - 4px);
    flex-shrink: 0;
    pointer-events: none;
  }

  .course-meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .platform-badge {
    font-size: 10.5px;
    font-weight: 500;
    color: var(--cta);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .course-title {
    font-size: 14px;
    font-weight: 500;
    margin-block: 0;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .course-stats {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11.5px;
    color: var(--gray);
    flex-wrap: wrap;
  }

  .meta-sep {
    opacity: 0.4;
  }

  .dismiss-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    flex-shrink: 0;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--gray);
    padding: 0;
    border-radius: calc(var(--border-radius) / 2);
  }

  @media (hover: hover) {
    .dismiss-btn:hover { color: var(--secondary); background: var(--button-stroke); }
  }

  .select-all-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px var(--padding);
    border-bottom: 1px solid var(--content-border);
  }

  .select-all-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--secondary);
    cursor: pointer;
    user-select: none;
  }

  .selection-count {
    font-size: 11.5px;
    color: var(--gray);
  }

  .sections-list {
    max-height: 320px;
    overflow-y: auto;
    scrollbar-width: none;
  }

  .sections-list::-webkit-scrollbar { display: none; }

  .section {
    border-bottom: 1px solid var(--content-border);
  }

  .section:last-child {
    border-bottom: none;
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px var(--padding);
  }

  .section-checkbox {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    cursor: pointer;
  }

  .section-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    text-align: left;
    color: var(--secondary);
  }

  .section-title {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .section-count {
    font-size: 11px;
    color: var(--gray);
    flex-shrink: 0;
  }

  .chevron {
    flex-shrink: 0;
    transition: transform 0.15s;
    color: var(--gray);
  }

  .chevron.expanded {
    transform: rotate(90deg);
  }

  @media (prefers-reduced-motion: reduce) {
    .chevron { transition: none; }
  }

  .lecture-list {
    display: flex;
    flex-direction: column;
    padding-bottom: 4px;
  }

  .lecture-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px var(--padding) 5px calc(var(--padding) + 24px);
    cursor: pointer;
    user-select: none;
  }

  .lecture-item.non-video {
    opacity: 0.45;
    cursor: default;
  }

  @media (hover: hover) {
    .lecture-item:not(.non-video):hover {
      background: var(--button-hover);
    }
  }

  .lecture-type-icon {
    flex-shrink: 0;
    color: var(--gray);
    pointer-events: none;
  }

  .lecture-title {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .lecture-duration {
    font-size: 11px;
    color: var(--gray);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }

  .download-footer {
    display: flex;
    align-items: center;
    gap: var(--padding);
    padding: var(--padding);
    border-top: 1px solid var(--content-border);
    background: var(--button);
  }

  .quality-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .quality-label {
    font-size: 12px;
    color: var(--gray);
    white-space: nowrap;
  }

  .quality-select {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--secondary);
    background: var(--input-bg);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 4px);
    padding: 4px 8px;
    cursor: pointer;
  }

  .quality-select:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }

  .download-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    flex: 1;
    padding: 10px var(--padding);
    font-size: 13.5px;
    font-weight: 500;
    background: var(--cta);
    color: var(--on-cta);
    border: none;
    border-radius: calc(var(--border-radius) - 2px);
    cursor: pointer;
    transition: background 0.15s;
  }

  @media (hover: hover) {
    .download-btn:hover:not(:disabled) { background: var(--cta-hover); }
  }

  .download-btn:active:not(:disabled) { background: var(--cta-press); }

  .download-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .download-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .download-btn svg { pointer-events: none; }

  .btn-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255,255,255,0.3);
    border-top-color: white;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  input[type="checkbox"] {
    accent-color: var(--cta);
    cursor: pointer;
    width: 14px;
    height: 14px;
    flex-shrink: 0;
  }

  input[type="checkbox"]:disabled { cursor: default; }
</style>
