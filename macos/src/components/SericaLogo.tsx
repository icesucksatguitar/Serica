import React, { useId } from 'react';

/**
 * The Serica mark: two slanted glass panes, the front one lit from its top
 * right corner. Each pane is a rounded square mapped onto its slant, the same
 * geometry the app icon is rendered from.
 */
export const SericaLogo: React.FC<{ size?: number | string; className?: string }> = ({ size = 24, className }) => {
  const id = useId().replace(/:/g, '');
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="22.5 38.5 520 520"
      role="img"
      aria-label="Serica"
    >
      <defs>
        <linearGradient id={`${id}-front`} x1="1" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#FFFCF7" stopOpacity="0.97" />
          <stop offset="1" stopColor="#969696" stopOpacity="0.8" />
        </linearGradient>
        <linearGradient id={`${id}-back`} x1="1" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#969696" stopOpacity="0.62" />
          <stop offset="1" stopColor="#5C5C5C" stopOpacity="0.55" />
        </linearGradient>
      </defs>
      <rect width="1" height="1" rx="0.15" fill={`url(#${id}-back)`} transform="matrix(345 -135 -25 270 170 285)" />
      <rect width="1" height="1" rx="0.15" fill={`url(#${id}-front)`} transform="matrix(345 -135 -25 270 70 180)" />
    </svg>
  );
};
