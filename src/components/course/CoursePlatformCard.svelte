<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type KnownCoursePlatform = {
    name: string;
    platform_id: string;
    domains: string[];
    support_status: { type: string };
    notes: string;
    login_url: string | null;
  };

  let {
    platform,
    sampleUrl,
    onDismiss,
  }: {
    platform: KnownCoursePlatform | null;
    sampleUrl: string;
    onDismiss: () => void;
  } = $props();

  let generating = $state(false);
  let taskPath = $state<string | null>(null);

  async function generateTask() {
    generating = true;
    taskPath = null;

    try {
      const path = await invoke<string>("generate_platform_task", {
        platformName: platform?.name ?? "Unknown",
        platformId: platform?.platform_id ?? "unknown",
        sampleUrl,
        detectedDomain: extractDomain(sampleUrl),
      });
      taskPath = path;
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    } finally {
      generating = false;
    }
  }

  function extractDomain(url: string): string {
    try {
      return new URL(url).hostname;
    } catch {
      return url;
    }
  }
</script>

<div class="platform-card">
  <div class="card-header">
    <div class="card-icon">
      <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <rect x="2" y="3" width="20" height="14" rx="2" ry="2"/>
        <line x1="8" y1="21" x2="16" y2="21"/>
        <line x1="12" y1="17" x2="12" y2="21"/>
      </svg>
    </div>
    <div class="card-info">
      <span class="platform-name">{platform?.name ?? extractDomain(sampleUrl)}</span>
      <span class="platform-domain">{extractDomain(sampleUrl)}</span>
    </div>
    <button class="dismiss-btn" onclick={onDismiss} aria-label={$t("common.close")}>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M18 6L6 18M6 6l12 12" />
      </svg>
    </button>
  </div>

  <div class="card-body">
    <div class="unsupported-badge">
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10" />
        <path d="M12 8v4m0 4h.01" />
      </svg>
      {$t("course.platform_not_supported")}
    </div>

    <p class="card-desc">
      {$t("course.platform_not_supported_desc")}
    </p>

    {#if platform?.notes}
      <p class="card-notes">{platform.notes}</p>
    {/if}

    {#if taskPath}
      <div class="task-success">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M22 11.08V12a10 10 0 11-5.93-9.14" />
          <polyline points="22 4 12 14.01 9 11.01" />
        </svg>
        <div class="task-success-text">
          <span>{$t("course.task_generated")}</span>
          <code class="task-path">{taskPath}</code>
        </div>
      </div>
    {:else}
      <button
        class="generate-btn"
        onclick={generateTask}
        disabled={generating}
      >
        {#if generating}
          <span class="btn-spinner"></span>
          {$t("course.generating_task")}
        {:else}
          <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 20h9" />
            <path d="M16.5 3.5a2.121 2.121 0 013 3L7 19l-4 1 1-4L16.5 3.5z" />
          </svg>
          {$t("course.generate_task")}
        {/if}
      </button>
    {/if}
  </div>
</div>

<style>
  .platform-card {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: 560px;
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    border-left: 3px solid var(--warning);
    overflow: hidden;
    animation: slideIn 250ms cubic-bezier(0.34, 1.56, 0.64, 1);
  }

  @keyframes slideIn {
    from { opacity: 0; transform: translateY(8px) scale(0.98); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: var(--padding);
    padding: var(--padding);
    border-bottom: 1px solid var(--content-border);
  }

  .card-icon {
    width: 40px;
    height: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--warning) 15%, transparent);
    color: var(--warning);
    border-radius: calc(var(--border-radius) - 2px);
    flex-shrink: 0;
  }

  .card-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .platform-name {
    font-size: 14px;
    font-weight: 500;
    color: var(--secondary);
  }

  .platform-domain {
    font-size: 11.5px;
    color: var(--gray);
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

  .card-body {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: var(--padding);
  }

  .unsupported-badge {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--warning);
  }

  .unsupported-badge svg { flex-shrink: 0; pointer-events: none; }

  .card-desc {
    font-size: 13px;
    color: var(--secondary);
    line-height: 1.5;
    margin: 0;
  }

  .card-notes {
    font-size: 12px;
    color: var(--gray);
    margin: 0;
    font-style: italic;
  }

  .generate-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 10px var(--padding);
    font-size: 13.5px;
    font-weight: 500;
    background: var(--button);
    color: var(--secondary);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    cursor: pointer;
    transition: background 0.15s;
  }

  @media (hover: hover) {
    .generate-btn:hover:not(:disabled) { background: var(--button-hover); }
  }

  .generate-btn:active:not(:disabled) { background: var(--button-press); }

  .generate-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .generate-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .generate-btn svg { pointer-events: none; }

  .task-success {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px;
    background: color-mix(in srgb, var(--success) 10%, transparent);
    border-radius: calc(var(--border-radius) - 2px);
  }

  .task-success svg {
    flex-shrink: 0;
    color: var(--success);
    margin-top: 2px;
    pointer-events: none;
  }

  .task-success-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .task-success-text span {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--secondary);
  }

  .task-path {
    font-size: 11px;
    color: var(--gray);
    word-break: break-all;
    font-family: var(--font-mono);
  }

  .btn-spinner {
    width: 13px;
    height: 13px;
    border: 2px solid var(--input-border);
    border-top-color: var(--secondary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }
</style>
