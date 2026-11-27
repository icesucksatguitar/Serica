import './adapter/tauriBridge';
import '@fontsource-variable/inter/opsz.css';
import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { DetachedWindow } from './components/DetachedWindow';

// A tab in a window of its own loads this bundle for its toolbar.
const detachedTab = new URLSearchParams(window.location.search).get('detached');

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    {detachedTab ? <DetachedWindow tabId={detachedTab} /> : <App />}
  </React.StrictMode>
);
