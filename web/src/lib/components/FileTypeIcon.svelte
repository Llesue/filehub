<script module lang="ts">
  type Category =
    | 'folder'
    | 'image'
    | 'video'
    | 'audio'
    | 'archive'
    | 'code'
    | 'pdf'
    | 'doc'
    | 'sheet'
    | 'slide'
    | 'text'
    | 'app'
    | 'library'
    | 'installer'
    | 'shortcut'
    | 'config'
    | 'font'
    | 'disk'
    | 'ebook'
    | 'design'
    | 'database'
    | 'file';
</script>

<script lang="ts">
  let {
    name,
    isDir = false,
    size = 20,
  }: { name: string; isDir?: boolean; size?: number } = $props();

  const files = (category: Category, extensions: string[]): Array<[string, Category]> =>
    extensions.map((extension) => [extension, category]);

  const extensions = new Map<string, Category>([
    ...files('image', [
      'jpg',
      'jpeg',
      'png',
      'gif',
      'webp',
      'bmp',
      'tif',
      'tiff',
      'avif',
      'heic',
      'heif',
      'svg',
      'ico',
      'icns',
      'jxl',
      'exr',
      'hdr',
      'tga',
      'cr2',
      'nef',
      'arw',
      'dng',
    ]),
    ...files('video', [
      'mp4',
      'mov',
      'mkv',
      'webm',
      'avi',
      'm4v',
      'mpg',
      'mpeg',
      'm2ts',
      'mts',
      'wmv',
      'flv',
      '3gp',
      'ogv',
      'vob',
      'asf',
      'f4v',
      'rm',
      'rmvb',
    ]),
    ...files('audio', [
      'mp3',
      'wav',
      'flac',
      'aac',
      'ogg',
      'm4a',
      'opus',
      'wma',
      'aiff',
      'amr',
      'ape',
      'm4b',
    ]),
    ...files('archive', [
      'zip',
      'rar',
      '7z',
      'tar',
      'gz',
      'bz2',
      'xz',
      'tgz',
      'tbz2',
      'txz',
      'zst',
      'cab',
    ]),
    ...files('code', [
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
      'xml',
      'sql',
      'vue',
      'svelte',
      'php',
      'rb',
      'swift',
      'kt',
      'dart',
      'lua',
      'mjs',
      'cjs',
      'bat',
      'cmd',
      'ps1',
      'vbs',
      'wsf',
    ]),
    ...files('pdf', ['pdf', 'ofd']),
    ...files('doc', ['doc', 'docx', 'rtf', 'odt', 'docm', 'dotm', 'dotx', 'wps', 'caj']),
    ...files('sheet', ['xls', 'xlsx', 'csv', 'ods', 'xlsm', 'xltm', 'xltx', 'et', 'tsv']),
    ...files('slide', ['ppt', 'pptx', 'odp', 'pptm', 'potm', 'potx', 'ppsx', 'dps', 'key']),
    ...files('text', ['txt', 'md', 'markdown', 'log', 'srt', 'ass', 'vtt', 'ssa']),
    ...files('app', ['exe', 'com', 'scr']),
    ...files('library', ['dll', 'sys', 'drv', 'ocx', 'so', 'dylib']),
    ...files('installer', ['msi', 'msix', 'appx', 'msu', 'pkg', 'deb', 'rpm', 'apk']),
    ...files('shortcut', ['lnk', 'url', 'webloc', 'desktop']),
    ...files('config', ['inf', 'ini', 'cfg', 'conf', 'env', 'reg', 'properties']),
    ...files('font', ['ttf', 'otf', 'fon', 'woff', 'woff2', 'eot']),
    ...files('disk', ['iso', 'img', 'vhd', 'vhdx', 'vmdk', 'wim', 'esd', 'dmg']),
    ...files('ebook', ['epub', 'mobi', 'azw3']),
    ...files('design', ['psd', 'ai', 'xd', 'sketch', 'fig', 'eps']),
    ...files('database', ['sqlite', 'sqlite3', 'db']),
  ]);

  const filenames = new Map<string, Category>([
    ['dockerfile', 'code'],
    ['makefile', 'code'],
    ['cmakelists.txt', 'code'],
    ['readme', 'text'],
    ['license', 'text'],
  ]);

  const category = $derived.by(() => {
    if (isDir) return 'folder';
    const lower = name.toLowerCase();
    const special = filenames.get(lower);
    if (special) return special;
    const ext = lower.split('.').pop() ?? '';
    return extensions.get(ext) ?? 'file';
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
  {:else if category === 'app'}
    <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
    <path d="M3.5 8.5h17M8 6.5h.01M11 6.5h.01" />
  {:else if category === 'library'}
    <path d="M4 8V6a2 2 0 0 1 2-2h3v4H4M20 16v2a2 2 0 0 1-2 2h-3v-4h5M8 4v16M16 4v16" />
  {:else if category === 'installer'}
    <path d="M4 8h16v11H4zM4 8l2-4h12l2 4M12 10v5m0 0 2-2m-2 2-2-2" />
  {:else if category === 'shortcut'}
    <path d="M5 19 19 5M10 5h9v9" />
  {:else if category === 'config'}
    <path d="M4 7h9M17 7h3M4 17h3M11 17h9" />
    <circle cx="15" cy="7" r="2" />
    <circle cx="9" cy="17" r="2" />
  {:else if category === 'font'}
    <path d="M5 20 12 4l7 16M8 14h8M5 20h4M15 20h4" />
  {:else if category === 'disk'}
    <ellipse cx="12" cy="6.5" rx="7" ry="3" />
    <path d="M5 6.5v11c0 1.7 3.1 3 7 3s7-1.3 7-3v-11M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3" />
  {:else if category === 'ebook'}
    <path d="M4 5.5A2.5 2.5 0 0 1 6.5 3H20v18H6.5A2.5 2.5 0 0 1 4 18.5z" />
    <path d="M4 17.5h16M8 7h8M8 10h6" />
  {:else if category === 'design'}
    <path d="M12 3.5 18.5 10 12 20.5 5.5 10Z" />
    <path d="M12 10v6.5" />
    <circle cx="12" cy="9.5" r="1.3" />
    <path d="M3 20c3.5-.5 6-2.5 8-5.5" />
  {:else if category === 'database'}
    <ellipse cx="12" cy="6" rx="7" ry="3" />
    <path d="M5 6v12c0 1.7 3.1 3 7 3s7-1.3 7-3V6M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3" />
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
  .type-app {
    color: var(--file-video);
  }
  .type-library,
  .type-config {
    color: var(--file-code);
  }
  .type-installer,
  .type-disk {
    color: var(--file-archive);
  }
  .type-shortcut,
  .type-font,
  .type-ebook {
    color: var(--file-doc);
  }
  .type-design {
    color: var(--file-image);
  }
  .type-database {
    color: var(--file-sheet);
  }
</style>
