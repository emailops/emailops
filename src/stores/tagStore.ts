import { create } from 'zustand';
import * as api from '@/lib/api';
import type { EmailTag } from '@/types';

/** Payload of the backend `email-classified` event. */
export interface ClassifiedTags {
  priority: string;
  intent: string;
  topic: string;
  confidence: number | null;
}

const CLASSIFIER_TAG_TYPES = new Set(['priority', 'intent', 'topic']);

/**
 * Pure helper: an email's tags after a classification landed. Only the three
 * tag types the classifier owns are replaced; junk and company tags come from
 * other pipelines and must survive (replacing the whole list dropped them).
 */
export function mergeClassifiedTags(
  existing: EmailTag[],
  emailId: string,
  tags: ClassifiedTags,
  now: number,
): EmailTag[] {
  const { confidence } = tags;
  return [
    ...existing.filter((t) => !CLASSIFIER_TAG_TYPES.has(t.tagType)),
    { emailId, tagType: 'priority', tagValue: tags.priority, confidence, createdAt: now },
    { emailId, tagType: 'intent', tagValue: tags.intent, confidence, createdAt: now },
    { emailId, tagType: 'topic', tagValue: tags.topic, confidence, createdAt: now },
  ];
}

interface TagStore {
  /** Map of emailId -> tags */
  tagsByEmail: Record<string, EmailTag[]>;
  /** Load tags for a batch of email IDs */
  loadTags: (emailIds: string[]) => Promise<void>;
  /** Update tags for a single email (from real-time event) */
  setEmailTags: (emailId: string, tags: EmailTag[]) => void;
  /** Get tags for a single email */
  getTagsForEmail: (emailId: string) => EmailTag[];
}

export const useTagStore = create<TagStore>((set, get) => ({
  tagsByEmail: {},

  loadTags: async (emailIds: string[]) => {
    if (emailIds.length === 0) return;

    // Only fetch for emails we don't have tags for yet
    const existing = get().tagsByEmail;
    const missing = emailIds.filter((id) => !(id in existing));
    if (missing.length === 0) return;

    try {
      const tags = await api.getEmailTagsBatch(missing);
      const grouped: Record<string, EmailTag[]> = {};
      // Initialize all missing IDs with empty arrays
      for (const id of missing) {
        grouped[id] = [];
      }
      for (const tag of tags) {
        if (!grouped[tag.emailId]) {
          grouped[tag.emailId] = [];
        }
        grouped[tag.emailId].push(tag);
      }
      set((state) => ({
        tagsByEmail: { ...state.tagsByEmail, ...grouped },
      }));
    } catch {
      // Non-critical, tags just won't show
    }
  },

  setEmailTags: (emailId: string, tags: EmailTag[]) => {
    set((state) => ({
      tagsByEmail: { ...state.tagsByEmail, [emailId]: tags },
    }));
  },

  getTagsForEmail: (emailId: string) => {
    return get().tagsByEmail[emailId] || [];
  },
}));
