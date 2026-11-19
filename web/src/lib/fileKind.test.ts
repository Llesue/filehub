import { describe, expect, it } from 'vitest';
import { classifyFile, extensionOf } from './fileKind';

describe('extensionOf', () => {
  it('takes the last dot and normalizes', () => {
    expect(extensionOf('photo.JPG')).toBe('jpg');
    expect(extensionOf('archive.tar.gz')).toBe('gz');
    expect(extensionOf('foo. txt')).toBe('txt');
    expect(extensionOf('.env')).toBe('');
    expect(extensionOf('foo.')).toBe('');
  });
});

describe('classifyFile', () => {
  it('never classifies directories by their name', () => {
    expect(classifyFile('video.mp4', true)).toBe('folder');
    expect(classifyFile('photos.2026', true)).toBe('folder');
  });

  it('classifies known text and media', () => {
    expect(classifyFile('notes.md')).toBe('text');
    expect(classifyFile('photo.JPG ')).toBe('image');
    expect(classifyFile('movie.mkv')).toBe('video');
    expect(classifyFile('song.flac')).toBe('audio');
    expect(classifyFile('paper.pdf')).toBe('pdf');
    expect(classifyFile('report.psd')).toBe('binary');
  });

  it('keeps unknown names for probing', () => {
    expect(classifyFile('adfasdasdfasd.2.1.0asadf asdf')).toBe('unknown');
    expect(classifyFile('阿 . adsa 阿斯蒂芬')).toBe('unknown');
    expect(classifyFile('.env')).toBe('text');
    expect(classifyFile('archive.tar.gz')).toBe('binary');
  });

  it('uses MIME for ambiguous types', () => {
    expect(classifyFile('segment.ts', false, 'video/mp2t')).toBe('video');
    expect(classifyFile('app.ts', false, 'text/plain')).toBe('text');
    expect(classifyFile('model.obj', false, 'text/plain')).toBe('text');
    expect(classifyFile('model.obj', false, 'application/octet-stream')).toBe('unknown');
  });
});
