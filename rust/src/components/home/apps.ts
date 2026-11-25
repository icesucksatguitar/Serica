// App widgets for the new-tab page. Each one opens the real app in a tab:
// its home, a "new" action, or a prefilled prompt/search where the app
// supports one in its URL. Nothing is fetched from the app itself (no
// accounts or tokens): the app does its own sign-in in the tab it opens.

export interface AppWidgetDef {
  id: string;
  name: string;
  /** Brand color, used for the widget's accent. */
  color: string;
  /** Site whose icon is shown: a domain, or a full address where a product
   *  shares its domain with others (Google's apps). */
  domain: string;
  home: string;
  /** `{q}` is replaced by the typed text (URL-encoded). */
  queryUrl?: string;
  placeholder?: string;
  actions: { label: string; url: string }[];
  description: string;
}

export const APP_WIDGETS: readonly AppWidgetDef[] = [
  {
    id: 'claude',
    name: 'Claude',
    color: '#D97757',
    domain: 'claude.ai',
    home: 'https://claude.ai',
    queryUrl: 'https://claude.ai/new?q={q}',
    placeholder: 'Ask Claude…',
    actions: [
      { label: 'New chat', url: 'https://claude.ai/new' },
      { label: 'Projects', url: 'https://claude.ai/projects' },
    ],
    description: 'Start a chat with your question already typed in',
  },
  {
    id: 'chatgpt',
    name: 'ChatGPT',
    color: '#10A37F',
    domain: 'chatgpt.com',
    home: 'https://chatgpt.com',
    queryUrl: 'https://chatgpt.com/?q={q}',
    placeholder: 'Ask ChatGPT…',
    actions: [{ label: 'New chat', url: 'https://chatgpt.com' }],
    description: 'Ask a question in a new chat',
  },
  {
    id: 'perplexity',
    name: 'Perplexity',
    color: '#20B8CD',
    domain: 'perplexity.ai',
    home: 'https://www.perplexity.ai',
    queryUrl: 'https://www.perplexity.ai/search?q={q}',
    placeholder: 'Ask anything…',
    actions: [],
    description: 'Search with answers and sources',
  },
  {
    id: 'notion',
    name: 'Notion',
    color: '#9B9A97',
    domain: 'notion.so',
    home: 'https://www.notion.so',
    actions: [
      { label: 'New page', url: 'https://notion.new' },
      { label: 'Open Notion', url: 'https://www.notion.so' },
    ],
    description: 'Jump into your workspace or start a page',
  },
  {
    id: 'gmail',
    name: 'Gmail',
    color: '#EA4335',
    domain: 'https://mail.google.com/mail/',
    home: 'https://mail.google.com',
    actions: [
      { label: 'Compose', url: 'https://mail.google.com/mail/?view=cm&fs=1' },
      { label: 'Inbox', url: 'https://mail.google.com' },
    ],
    description: 'Open your inbox or write an email',
  },
  {
    id: 'gcal',
    name: 'Calendar',
    color: '#4285F4',
    domain: 'https://calendar.google.com/calendar/',
    home: 'https://calendar.google.com',
    actions: [
      { label: 'New event', url: 'https://calendar.google.com/calendar/r/eventedit' },
      { label: 'Today', url: 'https://calendar.google.com/calendar/r/day' },
    ],
    description: 'Google Calendar: today, or a new event',
  },
  {
    id: 'docs',
    name: 'Google Docs',
    color: '#4285F4',
    domain: 'https://docs.google.com/document/',
    home: 'https://docs.google.com',
    actions: [
      { label: 'New doc', url: 'https://docs.new' },
      { label: 'New sheet', url: 'https://sheets.new' },
    ],
    description: 'Start a document or spreadsheet',
  },
  {
    id: 'youtube',
    name: 'YouTube',
    color: '#FF0033',
    domain: 'youtube.com',
    home: 'https://www.youtube.com',
    queryUrl: 'https://www.youtube.com/results?search_query={q}',
    placeholder: 'Search YouTube…',
    actions: [{ label: 'Subscriptions', url: 'https://www.youtube.com/feed/subscriptions' }],
    description: 'Search videos or open your subscriptions',
  },
  {
    id: 'spotify',
    name: 'Spotify',
    color: '#1DB954',
    domain: 'open.spotify.com',
    home: 'https://open.spotify.com',
    queryUrl: 'https://open.spotify.com/search/{q}',
    placeholder: 'Search music…',
    actions: [{ label: 'Your library', url: 'https://open.spotify.com/collection' }],
    description: 'Search music and open your library',
  },
  {
    id: 'githubapp',
    name: 'GitHub',
    color: '#8B949E',
    domain: 'github.com',
    home: 'https://github.com',
    queryUrl: 'https://github.com/search?q={q}',
    placeholder: 'Search GitHub…',
    actions: [
      { label: 'New repository', url: 'https://github.com/new' },
      { label: 'Pull requests', url: 'https://github.com/pulls' },
    ],
    description: 'Search code and repositories, or start one',
  },
];

export function appWidget(kind: string): AppWidgetDef | undefined {
  return kind.startsWith('app:') ? APP_WIDGETS.find((a) => `app:${a.id}` === kind) : undefined;
}

export function appIcon(domain: string): string {
  return domain.includes('/')
    ? `https://www.google.com/s2/favicons?domain_url=${encodeURIComponent(domain)}&sz=64`
    : `https://www.google.com/s2/favicons?domain=${encodeURIComponent(domain)}&sz=64`;
}

export function appQueryUrl(def: AppWidgetDef, text: string): string {
  return (def.queryUrl ?? def.home).replace('{q}', encodeURIComponent(text));
}
