import './adapter/tauriBridge';
import '@fontsource-variable/inter/opsz.css';
import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { DetachedWindow } from './components/DetachedWindow';
import { clearStaleViewportHoles } from './hooks/useViewportOverlay';

// A tab in a window of its own loads this bundle for its toolbar.
const detachedTab = new URLSearchParams(window.location.search).get('detached');

const render = () =>
  ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
    <React.StrictMode>
      {detachedTab ? <DetachedWindow tabId={detachedTab} /> : <App />}
    </React.StrictMode>
  );

if (detachedTab) {
  render();
} else {
  // Cut-outs left in the page by the UI before a reload go first, so they
  // are never removed after this UI has made its own (never waits long).
  void Promise.race([clearStaleViewportHoles(), new Promise((r) => setTimeout(r, 400))]).then(render);
}
