/** Two attachment tags are the same tag when they differ only in case. */
export const sameTag = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'accent' }) === 0;
