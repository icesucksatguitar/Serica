import React from 'react';

/**
 * Keeps a fault in an optional layer (the tour, a guide) from taking the
 * whole window down: the layer disappears and the browser carries on.
 */
export class SafeBoundary extends React.Component<{ children: React.ReactNode; onError?: () => void }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: unknown) {
    console.error('[serica] a layer failed and was closed:', error);
    this.props.onError?.();
  }

  render() {
    return this.state.failed ? null : this.props.children;
  }
}

export default SafeBoundary;
