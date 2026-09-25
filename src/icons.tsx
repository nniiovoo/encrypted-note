// Line icons drawn in the style of SF Symbols, inline so they need no font or network. Decorative
// only: every icon sits next to a text label (no tooltips: they'd be separate OS windows, ADR-0002).

import type { ReactNode } from "react";
import type { Kind } from "./api";

const PATHS = {
  all: <path d="M3 11.5V15a1.5 1.5 0 0 0 1.5 1.5h11A1.5 1.5 0 0 0 17 15v-3.5M3 11.5 5.2 4.5h9.6l2.2 7M3 11.5h4.2l1 2h3.6l1-2H17" />,
  star: <path d="m10 2.8 2.2 4.6 5 .7-3.6 3.5.9 5L10 14.2l-4.5 2.4.9-5L2.8 8.1l5-.7z" />,
  seed: <path d="M8 5.5h9M8 10h9M8 14.5h9M3.8 5.5h.4M3.8 10h.4M3.8 14.5h.4" />,
  key: (
    <>
      <circle cx="6.5" cy="13.5" r="3.5" />
      <path d="m9 11 7.5-7.5M13.5 6.5l2 2M11.3 8.7l1.5 1.5" />
    </>
  ),
  login: (
    <>
      <circle cx="10" cy="7" r="3.2" />
      <path d="M3.8 17c.8-3 3.3-4.6 6.2-4.6s5.4 1.6 6.2 4.6" />
    </>
  ),
  code: <path d="M6.5 5.5 2.5 10l4 4.5M13.5 5.5l4 4.5-4 4.5M11.3 3.8 8.7 16.2" />,
  text: <path d="M5.5 2.5h6l4 4v10a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1zM11.5 2.5v4h4M7.5 10h5M7.5 13h5" />,
  trash: (
    <path d="M3.5 5.5h13M8 5.5V3.8a.8.8 0 0 1 .8-.8h2.4a.8.8 0 0 1 .8.8v1.7M5 5.5l.8 10.2a1.5 1.5 0 0 0 1.5 1.3h5.4a1.5 1.5 0 0 0 1.5-1.3L15 5.5M8.3 8.5V14M11.7 8.5V14" />
  ),
  settings: (
    <>
      <path d="M3 6h7.5M15.5 6H17M3 14h1.5M9.5 14H17" />
      <circle cx="13" cy="6" r="2.2" />
      <circle cx="7" cy="14" r="2.2" />
    </>
  ),
  lock: (
    <>
      <rect x="4.5" y="9" width="11" height="8.5" rx="1.8" />
      <path d="M7 9V6.5a3 3 0 0 1 6 0V9" />
    </>
  ),
  plus: <path d="M10 4.5v11M4.5 10h11" />,
  search: (
    <>
      <circle cx="8.5" cy="8.5" r="5" />
      <path d="m12.3 12.3 4.2 4.2" />
    </>
  ),
  eye: (
    <>
      <path d="M1.8 10S4.8 4.5 10 4.5s8.2 5.5 8.2 5.5-3 5.5-8.2 5.5S1.8 10 1.8 10z" />
      <circle cx="10" cy="10" r="2.5" />
    </>
  ),
  eyeSlash: (
    <>
      <path d="M1.8 10S4.8 4.5 10 4.5s8.2 5.5 8.2 5.5-3 5.5-8.2 5.5S1.8 10 1.8 10z" />
      <circle cx="10" cy="10" r="2.5" />
      <path d="m3.5 3.5 13 13" />
    </>
  ),
  copy: (
    <>
      <rect x="7" y="7" width="9.5" height="9.5" rx="1.5" />
      <path d="M13 7V4.5A1.5 1.5 0 0 0 11.5 3h-7A1.5 1.5 0 0 0 3 4.5v7A1.5 1.5 0 0 0 4.5 13H7" />
    </>
  ),
  pencil: <path d="m13.2 3.8 3 3L7 16H4v-3zM11.2 5.8l3 3" />,
  shield: <path d="M10 2.5 16 5v4.6c0 3.9-2.6 6.7-6 7.9-3.4-1.2-6-4-6-7.9V5z" />,
  warning: <path d="M10 3.2 17.8 16.5H2.2zM10 8.2v4M10 14.4v.1" />,
  computer: <path d="M4 3.5h12A1.5 1.5 0 0 1 17.5 5v7a1.5 1.5 0 0 1-1.5 1.5H4A1.5 1.5 0 0 1 2.5 12V5A1.5 1.5 0 0 1 4 3.5zM7 17h6M10 13.5V17" />,
  back: <path d="M12.5 4.5 7 10l5.5 5.5" />,
} satisfies Record<string, ReactNode>;

export type IconName = keyof typeof PATHS;

export const Icon = ({ name, className = "icon" }: { name: IconName; className?: string }) => (
  <svg
    className={className}
    viewBox="0 0 20 20"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.6"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    {PATHS[name]}
  </svg>
);

export const KIND_ICONS: Record<Kind, IconName> = {
  seed_phrase: "seed",
  private_key: "key",
  login: "login",
  api_key: "code",
  text: "text",
};

/** A Kind's glyph on a colored rounded square, like the icons in System Settings. */
export const KindBadge = ({ kind, size = "small" }: { kind: Kind; size?: "small" | "large" }) => (
  <span className={`badge-icon ${size} ${kind}`} aria-hidden="true">
    <Icon name={KIND_ICONS[kind]} />
  </span>
);
