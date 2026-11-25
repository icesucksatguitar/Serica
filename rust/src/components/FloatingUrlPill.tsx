import React, { useState, useEffect, useRef } from 'react';
import { Lock, Globe, Copy, Check, Search, Moon, Sun } from 'lucide-react';
import { prewarmViewport } from '../hooks/useViewportOverlay';

export interface FloatingUrlPillProps {
  url?: string;
  isLoading?: boolean;
  zoomFactor?: number;
  /** Zoom of an untouched page (the text size setting). */
  defaultZoom?: number;
  onClick: () => void;
  onResetZoom?: () => void;
  onOpenFind?: () => void;
  /** Active tab, for the dark-PDF toggle shown on PDF tabs. */
  tabId?: string;
  /** Re-reads the dark-PDF state when the app theme or settings change. */
  pdfDarkKey?: string;
}

const isPdfUrl = (url?: string) => !!url && /\.pdf($|[?#])/i.test(url);

  export const FloatingUrlPill: React.FC<FloatingUrlPillProps> = ({
    url,
    isLoading = false,
    zoomFactor = 1.0,
    defaultZoom = 1.0,
    onClick,
    onResetZoom,
    onOpenFind,
    tabId,
    pdfDarkKey,
  }) => {
    const [copied, setCopied] = useState(false);
    // Dark PDFs: shown when the setting is on and the app is dark; a PDF tab
    // can switch back to the document's own colors (scans, photos).
    const [pdfDark, setPdfDark] = useState<{ available: boolean; dark: boolean } | null>(null);
    const isPdf = isPdfUrl(url);
    useEffect(() => {
      setPdfDark(null);
      if (!isPdf || !tabId) return;
      let alive = true;
      void window.serica?.getPdfDarkTab?.(tabId).then((st) => {
        if (alive && st) setPdfDark(st);
      });
      return () => {
        alive = false;
      };
    }, [isPdf, tabId, url, pdfDarkKey]);
    const togglePdfDark = (e: React.MouseEvent) => {
      e.stopPropagation();
      if (!tabId || !pdfDark) return;
      const next = !pdfDark.dark;
      setPdfDark({ ...pdfDark, dark: next });
      void window.serica?.setPdfDarkTab?.(tabId, next);
    };
    const copyTimerRef = useRef<number | null>(null);

    useEffect(() => {
      return () => {
        if (copyTimerRef.current) window.clearTimeout(copyTimerRef.current);
      };
    }, []);

    const isWebUrl = Boolean(url && (url.startsWith('https://') || url.startsWith('http://')));
    const isHttps = Boolean(url && url.startsWith('https://'));

    let displayDomain = 'new tab';
    if (url && url !== 'serica://newtab' && url !== 'neutrino://newtab' && url !== 'about:blank') {
      try {
        const parsed = new URL(url);
        displayDomain =
          parsed.protocol === 'file:'
            ? decodeURIComponent(parsed.pathname.split('/').pop() || '') || 'local file'
            : parsed.hostname.replace(/^www\./, '');
      } catch {
        displayDomain = url;
      }
    }

    const handleCopy = (e: React.MouseEvent) => {
      e.stopPropagation();
      if (!url || !isWebUrl) return;
      navigator.clipboard?.writeText(url).catch(() => { });
      setCopied(true);
      if (copyTimerRef.current) window.clearTimeout(copyTimerRef.current);
      copyTimerRef.current = window.setTimeout(() => setCopied(false), 1400);
    };

    const handleOpenFind = (e: React.MouseEvent) => {
      e.stopPropagation();
      onOpenFind?.();
    };

    const handleResetZoom = (e: React.MouseEvent) => {
      e.stopPropagation();
      onResetZoom?.();
    };

    const showZoomBadge = isWebUrl && Math.abs(zoomFactor - defaultZoom) > 0.01 && Boolean(onResetZoom);

    return (
      <div
        className={`top-floating-url-pill ${isLoading ? 'loading' : ''} ${isWebUrl ? 'has-actions' : ''}`}
        onPointerEnter={prewarmViewport}
        onClick={onClick}
        role="button"
        tabIndex={0}
        aria-label="Address bar and site controls"
        title={isWebUrl ? `${url}\nClick or press Ctrl+L to search / enter address` : 'Click or press Ctrl+L to search / enter address'}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onClick();
          }
        }}
      >
        <span className="url-pill-main">
          {isWebUrl && !isHttps ? (
            <Globe className="url-lock-icon insecure" size={12} aria-hidden="true" />
          ) : (
            <Lock className="url-lock-icon" size={12} aria-hidden="true" />
          )}
          <span className="url-domain-text">{displayDomain}</span>
        </span>

        {(isWebUrl || showZoomBadge || pdfDark?.available) && (
          <span className="url-pill-actions">
            {showZoomBadge && (
              <button
                type="button"
                className="url-pill-zoom-badge"
                onClick={handleResetZoom}
                aria-label={`Reset zoom from ${Math.round(zoomFactor * 100)}%`}
                title="Reset zoom to 100% (Ctrl+0)"
              >
                {Math.round(zoomFactor * 100)}%
              </button>
            )}
            {pdfDark?.available && (
              <button
                type="button"
                className={`url-pill-action-btn${pdfDark.dark ? ' active' : ''}`}
                onClick={togglePdfDark}
                aria-pressed={pdfDark.dark}
                aria-label={pdfDark.dark ? 'Show the PDF in its own colors' : 'Show the PDF with dark pages'}
                title={pdfDark.dark ? 'Original colors (for scans and photos)' : 'Dark pages'}
              >
                {pdfDark.dark ? <Sun size={11} /> : <Moon size={11} />}
              </button>
            )}
            {isWebUrl && onOpenFind && (
              <button
                type="button"
                className="url-pill-action-btn"
                onClick={handleOpenFind}
                aria-label="Find in page"
                title="Find in page (Ctrl+F)"
              >
                <Search size={11} />
              </button>
            )}
            {isWebUrl && (
              <button
                type="button"
                className={`url-pill-action-btn ${copied ? 'copied' : ''}`}
                onClick={handleCopy}
                aria-label={copied ? 'Copied URL' : 'Copy page URL'}
                title={copied ? 'Copied URL!' : 'Copy current URL'}
              >
                {copied ? <Check size={11} /> : <Copy size={11} />}
              </button>
            )}
          </span>
        )}
      </div>
    );
  };

