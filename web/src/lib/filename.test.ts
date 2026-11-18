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
});
