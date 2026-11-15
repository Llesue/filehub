<script lang="ts">
  let {
    name,
    isDir = false,
    size = 20,
  }: { name: string; isDir?: boolean; size?: number } = $props();

  const category = $derived.by(() => {
    if (isDir) return 'folder';
    const ext = name.split('.').pop()?.toLowerCase() ?? '';
    if (
      ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'tif', 'tiff', 'avif', 'heic', 'heif'].includes(
        ext,
      )
    )
      return 'image';
    if (['mp4', 'mov', 'mkv', 'webm', 'avi', 'm4v'].includes(ext)) return 'video';
    if (['mp3', 'wav', 'flac', 'aac', 'ogg', 'm4a'].includes(ext)) return 'audio';
    if (['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz'].includes(ext)) return 'archive';
    if (
      [
        'js',
        'ts',
        'tsx',
        'jsx',
        'rs',
        'py',
        'go',
        'java',
        'c',
        'cpp',
        'h',
        'css',
        'html',
        'json',
        'toml',
        'yaml',
        'yml',
        'sh',
      ].includes(ext)
    )
      return 'code';
    if (ext === 'pdf') return 'pdf';
    if (['doc', 'docx'].includes(ext)) return 'doc';
    if (['xls', 'xlsx', 'csv'].includes(ext)) return 'sheet';
    if (['ppt', 'pptx'].includes(ext)) return 'slide';
    if (['txt', 'md', 'markdown', 'log'].includes(ext)) return 'text';
    return 'file';
  });
</script>

<svg
  class={`type-icon type-${category}`}
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.6"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if category === 'folder'}
    <path
      d="M3.5 7.5A1.5 1.5 0 0 1 5 6h4l2 2h7a1.5 1.5 0 0 1 1.5 1.5v7A1.5 1.5 0 0 1 18 18.5H5A1.5 1.5 0 0 1 3.5 17z"
      fill="currentColor"
      fill-opacity="0.9"
      stroke="currentColor"
    />
  {:else if category === 'image'}
    <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
    <circle cx="8.5" cy="9" r="1.5" />
    <path d="m4 17 5-5 4 4 3-3 4 4" />
  {:else if category === 'video'}
    <rect x="3.5" y="5" width="17" height="14" rx="2" />
    <path d="m10 9 5 3-5 3z" fill="currentColor" stroke="none" />
  {:else if category === 'audio'}
    <path d="M9 18V6l9-2v12" />
    <circle cx="6.5" cy="18" r="2.5" />
    <circle cx="15.5" cy="16" r="2.5" />
  {:else if category === 'archive'}
    <path d="M4 7h16v13H4z" />
    <path d="M4 7l2-3h12l2 3M10 10h4M10 14h4" />
  {:else if category === 'code'}
    <path d="m8 8-4 4 4 4M16 8l4 4-4 4M14 5l-4 14" />
  {:else if category === 'pdf' || category === 'doc' || category === 'text' || category === 'file'}
    <path d="M6 3h7l5 5v13H6z" />
    <path d="M13 3v5h5" />
    {#if category === 'text'}<path d="M9 13h6M9 16h6" />{/if}
    {#if category === 'pdf'}<path d="M9 13h2.2a1.3 1.3 0 0 1 0 2.6H9z" />{/if}
  {:else if category === 'sheet'}
    <path d="M6 3h7l5 5v13H6z" />
    <path d="M13 3v5h5M9 12h6M9 15h6M9 18h6M12 12v6" />
  {:else if category === 'slide'}
    <rect x="4" y="5" width="16" height="11" rx="1.5" />
    <path d="M12 16v3M8 21h8" />
  {/if}
</svg>

<style>
  .type-icon {
    flex: 0 0 auto;
  }

  .type-folder {
    color: var(--file-folder);
  }

  .type-image {
    color: var(--file-image);
  }

  .type-video {
    color: var(--file-video);
  }

  .type-audio {
    color: var(--accent-text);
  }

  .type-archive {
    color: var(--file-archive);
  }

  .type-code {
    color: var(--file-code);
  }

  .type-pdf {
    color: var(--file-pdf);
  }

  .type-doc {
    color: var(--file-doc);
  }

  .type-sheet {
    color: var(--file-sheet);
  }

  .type-slide {
    color: var(--file-slide);
  }

  .type-text,
  .type-file {
    color: var(--text-2);
  }
</style>
