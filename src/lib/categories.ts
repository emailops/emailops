import type { EmailCategory } from '@/types';

/** Every category the inbox knows how to filter by. */
export const VALID_CATEGORIES = new Set<EmailCategory>(['primary', 'social', 'updates', 'forums', 'promotions']);

/**
 * The inbox category filter a user starts with, before they narrow it.
 *
 * Deliberately every valid category rather than a curated subset. Which
 * categories an account actually syncs is an account setting the frontend
 * learns from `getAvailableCategories`, and that is what decides which tabs
 * exist — a filter default cannot know it. The previous default of
 * `['primary', 'social', 'updates']` therefore silently hid Promotions mail
 * from anyone who had opted into syncing it: the mail was downloaded, stored,
 * and then filtered out of the list by a choice the user never made.
 */
export const DEFAULT_CATEGORIES: EmailCategory[] = Array.from(VALID_CATEGORIES);
