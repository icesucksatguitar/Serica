import React, { useEffect, useMemo, useState } from 'react';
import { GuideTour, type GuideStep } from './GuideTour';
import { useViewportOverlay } from '../hooks/useViewportOverlay';

/**
 * The first-run tour, right after setup: the address bar, the sidebar taking
 * in the site controls, widgets, Theme Studio and Focus Mode. Five short
 * steps; Skip or Escape ends it at any point. Replayed from Settings › About.
 */
export const ProductTour: React.FC<{
  topTabs: boolean;
  onNewTab: boolean;
  setSidebarOpen: (open: boolean) => void;
  onDone: () => void;
}> = ({ topTabs, onNewTab, setSidebarOpen, onDone }) => {
  const [running, setRunning] = useState(false);
  useEffect(() => {
    const start = () => setRunning(true);
    window.addEventListener('serica:start-tour', start);
    return () => window.removeEventListener('serica:start-tour', start);
  }, []);
  // The page is shown as a still image while the tour talks over it.
  useViewportOverlay(running);

  const steps: GuideStep[] = useMemo(
    () => [
      {
        target: '.top-floating-url-pill',
        title: 'Everything starts here',
        body: 'Type a place or words to search. Ctrl+T opens a new tab, Ctrl+K finds any open tab, and typing > runs a command.',
        onEnter: () => setSidebarOpen(false),
      },
      topTabs
        ? {
            target: '.top-tab-strip',
            title: 'Your tabs',
            body: 'Tabs sit across the top. Drag one to reorder it; drop a link or a file here to open it. The sidebar is one switch away in Settings › Appearance.',
          }
        : {
            target: '.nsb.visible',
            title: 'The sidebar takes in the controls',
            body: 'Ctrl+S, the panel button or the left edge opens it. Back, forward and reload move into its top row, and go back to where they were when it closes.',
            onEnter: () => setSidebarOpen(true),
          },
      {
        target: onNewTab ? '.home-customize-btn2' : undefined,
        title: 'Widgets on the new tab',
        body: 'Press and hold a widget to move it anywhere on the grid; the others make room. Customize adds widgets and changes the clock and background.',
        onEnter: () => setSidebarOpen(false),
      },
      {
        target: '[data-tour="tools"]',
        title: 'Theme Studio',
        body: 'Each space has its own colours. Pick a palette in Theme Studio, from this menu or Settings › Appearance.',
      },
      {
        target: '[data-tour="focus"]',
        title: 'Focus Mode',
        body: 'Block the sites that pull you away, for 25 minutes or on a schedule. Lock it, and not even a restart ends it early.',
      },
    ],
    [topTabs, onNewTab, setSidebarOpen],
  );

  if (!running) return null;
  return (
    <GuideTour
      id="product-tour"
      steps={steps}
      onClose={() => {
        setRunning(false);
        setSidebarOpen(false);
        onDone();
      }}
    />
  );
};

export default ProductTour;
