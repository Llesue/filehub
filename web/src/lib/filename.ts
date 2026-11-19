const VALID_EXTENSION = /^[A-Za-z0-9._-]{1,16}$/;

export interface SplitNameOptions {
  isDir?: boolean;
}

export function splitFileName(
  name: string,
  options: SplitNameOptions = {},
): { stem: string; ext: string } {
  if (options.isDir) return { stem: name, ext: '' };
  const dot = name.lastIndexOf('.');
  if (dot <= 0 || dot === name.length - 1) return { stem: name, ext: '' };
  const extension = name.slice(dot + 1);
  if (!VALID_EXTENSION.test(extension)) return { stem: name, ext: '' };
  return { stem: name.slice(0, dot), ext: `.${extension}` };
}
