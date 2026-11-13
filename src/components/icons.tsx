// Stroke-based monochrome icons. Pure SVG, currentColor — they pick up
// the parent button's `color`, so the whole UI keeps a consistent
// non-emoji look across platforms (no Win11 color emoji surprises).

import type { CSSProperties } from "react";

interface IconProps {
  size?: number;
  strokeWidth?: number;
  style?: CSSProperties;
  className?: string;
  title?: string;
}

function base(p: IconProps) {
  const size = p.size ?? 16;
  return {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: p.strokeWidth ?? 1.7,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    style: p.style,
    className: p.className,
    "aria-hidden": p.title ? undefined : true,
    role: p.title ? "img" : undefined,
  };
}

export function MicIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5 11a7 7 0 0 0 14 0" />
      <path d="M12 18v3" />
    </svg>
  );
}

export function StopIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor" />
    </svg>
  );
}

export function WandIcon(p: IconProps = {}) {
  // Sparkle / image-gen wand — replaces 🎨.
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <path d="M5 19 17 7" />
      <path d="m14 4 1 2 2 1-2 1-1 2-1-2-2-1 2-1z" />
      <path d="m6 12 .6 1.4L8 14l-1.4.6L6 16l-.6-1.4L4 14l1.4-.6z" />
      <path d="M19 14l.5 1 1 .5-1 .5-.5 1-.5-1-1-.5 1-.5z" />
    </svg>
  );
}

export function EyeIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

export function EarIcon(p: IconProps = {}) {
  // Listening indicator — replaces 👂.
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <path d="M6 8a6 6 0 0 1 12 0c0 3-2 4-2 7a3 3 0 0 1-6 0" />
      <path d="M9 10a3 3 0 0 1 6 0" />
    </svg>
  );
}

export function GearIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1A1.7 1.7 0 0 0 4.6 9a1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" />
    </svg>
  );
}

export function RefreshIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <path d="M3 12a9 9 0 0 1 15.5-6.3L21 8" />
      <path d="M21 3v5h-5" />
      <path d="M21 12a9 9 0 0 1-15.5 6.3L3 16" />
      <path d="M3 21v-5h5" />
    </svg>
  );
}

export function DownloadIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <path d="M12 3v12" />
      <path d="m7 10 5 5 5-5" />
      <path d="M5 21h14" />
    </svg>
  );
}

export function CloseIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <path d="m6 6 12 12" />
      <path d="m18 6-12 12" />
    </svg>
  );
}

export function WarningIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <path d="M10.3 3.7 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.7a2 2 0 0 0-3.4 0Z" />
      <path d="M12 9v4" />
      <path d="M12 17h.01" />
    </svg>
  );
}

/** Stacked squares: window stacking / overlay layer. */
export function LayersIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <rect x="3" y="3" width="13" height="13" rx="2" />
      <path d="M8 21h11a2 2 0 0 0 2-2V8" />
    </svg>
  );
}

/** Drag grip: window move handle. */
export function GripIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)} strokeWidth={p.strokeWidth ?? 2.4}>
      {p.title && <title>{p.title}</title>}
      <path d="M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01" />
    </svg>
  );
}

export function ThumbUpIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <path d="M7 10v11" />
      <path d="M21 12.3a3 3 0 0 0-3-3.3h-4.4l.9-4.5a2 2 0 0 0-2-2.5l-5.5 6V21h11.4a2 2 0 0 0 2-1.7l1.4-7Z" />
    </svg>
  );
}

export function ThumbDownIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <path d="M17 14V3" />
      <path d="M3 11.7a3 3 0 0 0 3 3.3h4.4l-.9 4.5a2 2 0 0 0 2 2.5l5.5-6V3H5.6a2 2 0 0 0-2 1.7L2.2 11.7Z" />
    </svg>
  );
}

export function PaperclipIcon(p: IconProps = {}) {
  return (
    <svg {...base(p)}>
      {p.title && <title>{p.title}</title>}
      <path d="m21 12-9.5 9.5a5 5 0 0 1-7-7L14 5a3.5 3.5 0 0 1 5 5L9.5 19.5a2 2 0 0 1-3-3L15 8" />
    </svg>
  );
}
