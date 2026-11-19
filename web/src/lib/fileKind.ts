export type FileKind =
  | 'folder'
  | 'text'
  | 'image'
  | 'video'
  | 'audio'
  | 'pdf'
  | 'binary'
  | 'unknown';

const image = new Set(['jpg', 'jpeg', 'png', 'gif', 'webp', 'avif', 'bmp', 'ico', 'tif', 'tiff']);
const video = new Set(['mp4', 'mov', 'mkv', 'webm', 'avi', 'm4v']);
const audio = new Set(['mp3', 'wav', 'ogg', 'm4a', 'flac', 'aac', 'opus', 'oga']);
const text = new Set([
  'txt',
  'md',
  'markdown',
  'rst',
  'adoc',
  'json',
  'jsonc',
  'json5',
  'yaml',
  'yml',
  'toml',
  'ini',
  'conf',
  'cfg',
  'properties',
  'env',
  'xml',
  'html',
  'htm',
  'css',
  'scss',
  'sass',
  'less',
  'js',
  'mjs',
  'cjs',
  'jsx',
  'ts',
  'tsx',
  'vue',
  'svelte',
  'rs',
  'go',
  'py',
  'rb',
  'php',
  'java',
  'kt',
  'kts',
  'swift',
  'c',
  'h',
  'cc',
  'cpp',
  'cxx',
  'hpp',
  'cs',
  'sh',
  'bash',
  'zsh',
  'fish',
  'bat',
  'cmd',
  'ps1',
  'sql',
  'graphql',
  'gql',
  'proto',
  'log',
  'csv',
  'tsv',
  'gitignore',
  'dockerfile',
  'makefile',
  'cmake',
  'gradle',
  'srt',
  'ass',
  'vtt',
  'ssa',
]);
const binary = new Set([
  'ai',
  'psd',
  'psb',
  'cdr',
  'sketch',
  'fig',
  'xd',
  'indd',
  'idml',
  'eps',
  'ps',
  'cr2',
  'cr3',
  'nef',
  'arw',
  'dng',
  'raf',
  'rw2',
  'orf',
  'pef',
  'srw',
  'raw',
  'db',
  'sqlite',
  'sqlite3',
  'mdb',
  'accdb',
  'zip',
  'rar',
  '7z',
  'tar',
  'gz',
  'bz2',
  'xz',
  'zst',
  'lz4',
  'cab',
  'iso',
  'dmg',
  'apk',
  'ipa',
  'jar',
  'deb',
  'rpm',
  'msi',
  'doc',
  'docx',
  'xls',
  'xlsx',
  'xlsb',
  'ppt',
  'pptx',
  'pages',
  'numbers',
  'key',
  'exe',
  'dll',
  'so',
  'dylib',
  'bin',
  'class',
  'pyc',
  'o',
  'a',
  'lib',
  'ttf',
  'otf',
  'woff',
  'woff2',
  'eot',
  'wmv',
  'flv',
  'm2ts',
  'mts',
  '3gp',
  'ogv',
  'vob',
  'rm',
  'rmvb',
  'asf',
  'torrent',
  'p12',
  'pfx',
  'jks',
  'keystore',
  'der',
  'heic',
  'heif',
]);
const ambiguous = new Set(['ts', 'obj', 'stl', 'eps', 'dxf']);
const specialNames = new Set([
  'dockerfile',
  'makefile',
  'cmakelists.txt',
  '.gitignore',
  '.dockerignore',
  '.editorconfig',
  '.env',
  '.env.local',
  '.env.example',
]);

export function extensionOf(name: string): string {
  const dot = name.lastIndexOf('.');
  if (dot <= 0 || dot === name.length - 1) return '';
  return name.slice(dot + 1).trim().toLowerCase();
}

function validExtension(extension: string): boolean {
  return extension.length > 0 && extension.length <= 16 && /^[a-z0-9._-]+$/.test(extension);
}

export function classifyFile(
  name: string,
  isDir = false,
  mimeType?: string | null,
): FileKind {
  if (isDir) return 'folder';
  if (specialNames.has(name.toLowerCase())) return 'text';
  const extension = extensionOf(name);
  if (!validExtension(extension)) {
    return mimeKind(mimeType) ?? 'unknown';
  }
  if (ambiguous.has(extension)) {
    return mimeKind(mimeType) ?? 'unknown';
  }
  if (image.has(extension)) return 'image';
  if (video.has(extension)) return 'video';
  if (audio.has(extension)) return 'audio';
  if (extension === 'pdf') return 'pdf';
  if (text.has(extension)) return 'text';
  if (binary.has(extension)) return 'binary';
  return mimeKind(mimeType) ?? 'unknown';
}

function mimeKind(mimeType?: string | null): FileKind | null {
  if (!mimeType) return null;
  const mime = mimeType.toLowerCase();
  if (mime === 'video/mp2t') return 'video';
  if (mime.startsWith('video/')) return 'video';
  if (mime.startsWith('audio/')) return 'audio';
  if (mime.startsWith('image/')) return 'image';
  if (mime === 'application/pdf') return 'pdf';
  if (
    mime.startsWith('text/') ||
    mime === 'application/json' ||
    mime === 'application/xml' ||
    mime === 'application/javascript'
  ) {
    return 'text';
  }
  return null;
}
