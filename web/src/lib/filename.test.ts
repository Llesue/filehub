import { describe, expect, it } from 'vitest';
import { splitFileName } from './filename';

describe('splitFileName', () => {
  it('splits a normal file at the last dot', () => {
    expect(splitFileName('photo.final.jpg')).toEqual({ stem: 'photo.final', ext: '.jpg' });
  });

  it('keeps files without an extension whole', () => {
    expect(splitFileName('Makefile')).toEqual({ stem: 'Makefile', ext: '' });
  });

  it('treats dotfiles as names without an extension', () => {
    expect(splitFileName('.gitignore')).toEqual({ stem: '.gitignore', ext: '' });
    expect(splitFileName('.env.local')).toEqual({ stem: '.env', ext: '.local' });
  });

  it('keeps trailing dots in the stem', () => {
    expect(splitFileName('report.')).toEqual({ stem: 'report.', ext: '' });
  });

  it('does not split directories', () => {
    expect(splitFileName('photos.2026', { isDir: true })).toEqual({
      stem: 'photos.2026',
      ext: '',
    });
    expect(splitFileName('video.mp4', { isDir: true })).toEqual({ stem: 'video.mp4', ext: '' });
  });

  it('only keeps short ASCII extensions', () => {
    expect(splitFileName('adfasdasdfasd.2.1.0asadf asdf')).toEqual({
      stem: 'adfasdasdfasd.2.1.0asadf asdf',
      ext: '',
    });
    expect(splitFileName('阿 . adsa 阿斯蒂芬')).toEqual({
      stem: '阿 . adsa 阿斯蒂芬',
      ext: '',
    });
    expect(splitFileName('photo.JPG ')).toEqual({ stem: 'photo.JPG ', ext: '' });
    expect(splitFileName('archive.tar.gz')).toEqual({ stem: 'archive.tar', ext: '.gz' });
    expect(splitFileName('report.v2.pdf')).toEqual({ stem: 'report.v2', ext: '.pdf' });
  });
});
