<script lang="ts">
  type IconName =
    | 'folder'
    | 'search'
    | 'upload'
    | 'list'
    | 'grid'
    | 'tile'
    | 'image'
    | 'eye'
    | 'play'
    | 'pause'
    | 'stop'
    | 'previous'
    | 'next'
    | 'folder-plus'
    | 'folder-upload'
    | 'terminal'
    | 'shield'
    | 'move'
    | 'pencil'
    | 'moon'
    | 'sun'
    | 'menu'
    | 'close'
    | 'copy'
    | 'share'
    | 'settings'
    | 'logout'
    | 'file'
    | 'trash'
    | 'download'
    | 'link'
    | 'unlink'
    | 'volume'
    | 'volume-muted'
    | 'bell';

  let { name, size = 20 }: { name: IconName; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.7"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === 'folder'}
    <path
      d="M3.5 7.5A1.5 1.5 0 0 1 5 6h4l2 2h7a1.5 1.5 0 0 1 1.5 1.5v7A1.5 1.5 0 0 1 18 18.5H5A1.5 1.5 0 0 1 3.5 17z"
    />
  {:else if name === 'search'}
    <circle cx="11" cy="11" r="6" />
    <path d="m16 16 4 4" />
  {:else if name === 'upload'}
    <path d="M12 16V4" />
    <path d="m8 8 4-4 4 4" />
    <path d="M4 16v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" />
  {:else if name === 'list'}
    <path d="M8 6h12M8 12h12M8 18h12" />
    <path d="M4 6h.01M4 12h.01M4 18h.01" />
  {:else if name === 'grid'}
    <rect x="4" y="4" width="6" height="6" rx="1" />
    <rect x="14" y="4" width="6" height="6" rx="1" />
    <rect x="4" y="14" width="6" height="6" rx="1" />
    <rect x="14" y="14" width="6" height="6" rx="1" />
  {:else if name === 'tile'}
    <rect x="6" y="3.5" width="12" height="7" rx="1.5" />
    <rect x="6" y="13.5" width="12" height="7" rx="1.5" />
  {:else if name === 'eye'}
    <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z" />
    <circle cx="12" cy="12" r="3" />
  {:else if name === 'play'}
    <path d="M8 5v14l11-7z" fill="currentColor" stroke="none" />
  {:else if name === 'pause'}
    <path d="M8 5h3v14H8zM13 5h3v14h-3z" fill="currentColor" stroke="none" />
  {:else if name === 'stop'}
    <rect x="6" y="6" width="12" height="12" rx="1" fill="currentColor" stroke="none" />
  {:else if name === 'previous'}
    <path d="M6 5v14" />
    <path d="M19 5 9 12l10 7z" fill="currentColor" stroke="none" />
  {:else if name === 'next'}
    <path d="M18 5v14" />
    <path d="M5 5l10 7-10 7z" fill="currentColor" stroke="none" />
  {:else if name === 'image'}
    <rect x="3" y="5" width="18" height="14" rx="2" />
    <circle cx="8.5" cy="10" r="1.5" />
    <path d="m21 15-5-5L5 21" />
  {:else if name === 'folder-plus'}
    <path
      d="M3.5 7.5A1.5 1.5 0 0 1 5 6h4l2 2h7a1.5 1.5 0 0 1 1.5 1.5v7A1.5 1.5 0 0 1 18 18.5H5A1.5 1.5 0 0 1 3.5 17z"
    />
    <path d="M12 10v5M9.5 12.5h5" />
  {:else if name === 'terminal'}
    <rect x="3" y="4" width="18" height="16" rx="2" />
    <path d="m7 9 3 3-3 3M12.5 15H17" />
  {:else if name === 'shield'}
    <path d="M12 3 5 6v5c0 4.5 2.9 7.8 7 9 4.1-1.2 7-4.5 7-9V6z" />
  {:else if name === 'move'}
    <path d="M5 9V5h4M19 15v4h-4M5 5l5 5M19 19l-5-5" />
  {:else if name === 'pencil'}
    <path d="m4 20 4.5-1 10-10a2.1 2.1 0 0 0-3-3l-10 10z" />
    <path d="m14.5 7.5 2 2" />
  {:else if name === 'moon'}
    <path d="M20 14.5A8 8 0 0 1 9.5 4 8.5 8.5 0 1 0 20 14.5Z" />
  {:else if name === 'sun'}
    <circle cx="12" cy="12" r="4" />
    <path
      d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"
    />
  {:else if name === 'menu'}
    <path d="M4 7h16M4 12h16M4 17h16" />
  {:else if name === 'close'}
    <path d="m6 6 12 12M18 6 6 18" />
  {:else if name === 'copy'}
    <rect x="9" y="9" width="11" height="11" rx="2" />
    <path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1" />
  {:else if name === 'share'}
    <circle cx="18" cy="5" r="3" />
    <circle cx="6" cy="12" r="3" />
    <circle cx="18" cy="19" r="3" />
    <path d="m8.6 10.7 6.8-3.4M8.6 13.3l6.8 3.4" />
  {:else if name === 'settings'}
    <path d="M4 7h10M18 7h2M4 17h2M10 17h10" />
    <circle cx="16" cy="7" r="2" />
    <circle cx="8" cy="17" r="2" />
  {:else if name === 'logout'}
    <path d="M10 17l5-5-5-5" />
    <path d="M15 12H3" />
    <path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4" />
  {:else if name === 'file'}
    <path d="M6 3h7l5 5v13H6z" />
    <path d="M13 3v5h5" />
  {:else if name === 'trash'}
    <path d="M4 7h16M9 7V4h6v3M6 7l1 14h10l1-14" />
    <path d="M10 11v6M14 11v6" />
  {:else if name === 'download'}
    <path d="M12 4v12m0 0 4-4m-4 4-4-4" />
    <path d="M4 20h16" />
  {:else if name === 'link'}
    <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
    <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
  {:else if name === 'unlink'}
    <path d="M15 7h2a5 5 0 0 1 0 10h-2" />
    <path d="M9 17H7A5 5 0 0 1 7 7h2" />
  {:else if name === 'volume'}
    <path d="M4 9h4l5-4v14l-5-4H4z" />
    <path d="M16 8a5 5 0 0 1 0 8" />
    <path d="M18.5 5.5a9 9 0 0 1 0 13" />
  {:else if name === 'volume-muted'}
    <path d="M4 9h4l5-4v14l-5-4H4z" />
    <path d="m16 9 5 6m0-6-5 6" />
  {:else if name === 'bell'}
    <path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9" />
    <path d="M10 21h4" />
  {/if}
</svg>
