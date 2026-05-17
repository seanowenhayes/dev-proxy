import { describe, it, expect } from 'vitest';
import { cn } from '../utils';

describe('cn()', () => {
    it('joins class strings', () => {
        expect(cn('a', 'b', 'c')).toBe('a b c');
    });

    it('filters out falsy values', () => {
        expect(cn('a', false, undefined, 'b')).toBe('a b');
    });

    it('returns empty string when all values are falsy', () => {
        expect(cn(false, undefined)).toBe('');
    });

    it('handles a single class', () => {
        expect(cn('only')).toBe('only');
    });
});
