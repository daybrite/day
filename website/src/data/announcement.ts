// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

export interface Announcement {
  label: string;
  headline: string;
  href: `/${string}` | `https://${string}`;
}

// Editorial publication text, like a blog title, rather than reusable UI translations.
// Replace this entry for the next headline, or set it to null to hide the banner.
export const activeAnnouncement: Announcement | null = {
  label: 'Announcement',
  headline: 'A new Day for Rust apps',
  href: '/blog/new-day',
};
