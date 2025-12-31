<script lang="ts">
  import { createEventDispatcher } from 'svelte';

  const dispatch = createEventDispatcher();
  let isDragging = false;
  let files: FileList | null = null;

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    isDragging = true;
  }

  function handleDragLeave(e: DragEvent) {
    e.preventDefault();
    isDragging = false;
  }

  async function handleDrop(e: DragEvent) {
    e.preventDefault();
    isDragging = false;

    const items = e.dataTransfer?.items;
    if (!items) return;

    const droppedFiles: File[] = [];

    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      if (item.kind === 'file') {
        const file = item.getAsFile();
        if (file) droppedFiles.push(file);
      }
    }

    if (droppedFiles.length > 0) {
      dispatch('filesDropped', { files: droppedFiles });
    }
  }

  function handleFileInput(e: Event) {
    const target = e.target as HTMLInputElement;
    if (target.files && target.files.length > 0) {
      dispatch('filesDropped', { files: Array.from(target.files) });
    }
  }
</script>

<div
  class="drop-zone"
  class:dragging={isDragging}
  on:dragover={handleDragOver}
  on:dragleave={handleDragLeave}
  on:drop={handleDrop}
  role="button"
  tabindex="0"
>
  <div class="drop-content">
    <svg
      class="drop-icon"
      xmlns="http://www.w3.org/2000/svg"
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
    >
      <path
        stroke-linecap="round"
        stroke-linejoin="round"
        stroke-width="2"
        d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12"
      />
    </svg>

    <h3 class="drop-title">Drop files or folders here</h3>
    <p class="drop-subtitle">or click to browse</p>

    <input
      type="file"
      multiple
      on:change={handleFileInput}
      class="file-input"
      aria-label="Select files to lock"
    />
  </div>
</div>

<style>
  .drop-zone {
    position: relative;
    min-height: 300px;
    border: 2px dashed #333;
    border-radius: 12px;
    background: #0f0f0f;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.3s ease;
  }

  .drop-zone:hover,
  .drop-zone.dragging {
    border-color: #6366f1;
    background: #1a1a1a;
  }

  .drop-content {
    text-align: center;
    padding: 2rem;
    pointer-events: none;
  }

  .drop-icon {
    width: 64px;
    height: 64px;
    color: #6366f1;
    margin: 0 auto 1rem;
  }

  .drop-title {
    font-size: 1.25rem;
    font-weight: 600;
    color: #fff;
    margin: 0 0 0.5rem;
  }

  .drop-subtitle {
    font-size: 0.875rem;
    color: #888;
    margin: 0;
  }

  .file-input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
    pointer-events: auto;
  }
</style>
