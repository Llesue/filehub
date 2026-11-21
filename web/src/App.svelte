<script lang="ts">
  /* global __APP_VERSION__ */
  import DOMPurify from 'dompurify';
  import { marked } from 'marked';
  import { onMount, tick } from 'svelte';
  import { SvelteMap, SvelteSet } from 'svelte/reactivity';
  import AccountDialog from './lib/components/AccountDialog.svelte';
  import NoticeDialog from './lib/components/NoticeDialog.svelte';
  import ProgressPanel from './lib/components/ProgressPanel.svelte';
  import UploadPanel from './lib/components/UploadPanel.svelte';
  import ToastPanel from './lib/components/ToastPanel.svelte';
  import AdminPage from './lib/components/AdminPage.svelte';
  import AuditPage from './lib/components/AuditPage.svelte';
  import SharesPage from './lib/components/SharesPage.svelte';
  import ConfirmDialog from './lib/components/ConfirmDialog.svelte';
  import DirStatsDialog from './lib/components/DirStatsDialog.svelte';
  import FileTypeIcon from './lib/components/FileTypeIcon.svelte';
  import NameDialog from './lib/components/NameDialog.svelte';
  import NotificationCenter from './lib/components/NotificationCenter.svelte';
  import PreviewModal from './lib/components/PreviewModal.svelte';
  import PropertiesDialog from './lib/components/PropertiesDialog.svelte';
  import PreferencesPage from './lib/components/PreferencesPage.svelte';
  import RemoteMountDialog from './lib/components/RemoteMountDialog.svelte';
  import ShareDialog from './lib/components/ShareDialog.svelte';
  import TerminalPanel from './lib/components/TerminalPanel.svelte';
  import CodeEditor from './lib/components/CodeEditor.svelte';
  import TransferDialog from './lib/components/TransferDialog.svelte';
  import AuthScreen from './lib/components/AuthScreen.svelte';
  import SharePage from './lib/components/SharePage.svelte';
  import Icon from './lib/components/Icon.svelte';
  import AudioPlayer from './lib/components/AudioPlayer.svelte';
  import { initLanguage, language, localeOf, rootAlias, setLanguage, t } from './lib/i18n';
  import { showNotice, showNoticeAction } from './lib/notices';
  import { showToast } from './lib/toast';
  import { branding, brandMark, loadBranding } from './lib/branding';
  import { playAudio, type AudioTrack } from './lib/audio';
  import { uploadManager } from './lib/uploads.svelte';
  import { resolveViewPreference } from './lib/viewPref';
  import { connectEvents, disconnectEvents, realtimeActive, realtimeEvent } from './lib/events';
  import {
    addNotification,
    clearNotifications,
    ensureNotificationOwner,
  } from './lib/notifications';
  import { absoluteTime, relativeTimeParts } from './lib/time';
  import {
    failProgress,
    finishProgress,
    setProgressCancel,
    startProgress,
    updateProgress,
  } from './lib/progress';
  import {
    api,
    ApiError,
    type AuthResponse,
    type Entry,
    type RemoteMount,
    type Root,
    type Capabilities,
    type JobInfo,
    type SystemFeatures,
    type TransferJobResult,
    type UserPreferences,
    type SearchEntry,
    type Usage,
    type User,
    type VersionInfo,
  } from './lib/api';
  import { applyTheme, resolveTheme, type Theme, type ThemePreference } from './lib/theme';
  import { splitFileName } from './lib/filename';
  import { classifyFile } from './lib/fileKind';

  let theme = $state<Theme>('light');
  function storedView(): 'list' | 'thumb' | 'tile' {
    try {
      return resolveViewPreference(localStorage.getItem('filehub.view'), null) ?? 'list';
    } catch {
      return 'list';
    }
  }

  let view = $state<'list' | 'thumb' | 'tile'>(storedView());
  let sortField = $state<'name' | 'size' | 'modified'>('name');
  let sortOrder = $state<'asc' | 'desc'>('asc');
  let sidebarOpen = $state(false);
  let accountOpen = $state(false);
  let loading = $state(true);
  let initialized = $state(false);
  let user = $state<User | null>(null);
  let versionInfo = $state<VersionInfo | null>(null);
  let mountDialogOpen = $state(false);
  let userRemote = $state<{ mount: boolean; share: boolean } | null>(null);
  const canMountRemote = $derived(
    userRemote?.mount ?? user?.features.federation_mount ?? false,
  );
  const canShareRemote = $derived(
    userRemote?.share ?? user?.features.federation_share ?? false,
  );
  let preferences = $state<UserPreferences | null>(null);
  let systemFeatures = $state<SystemFeatures>({ thumbnails: true, editor: true, terminal: true });
  let capabilities = $state<Capabilities>({ terminal: true, zip: true, unzip: true });
  let csrf = $state('');
  let loggingOut = $state(false);
  let roots = $state<Root[]>([]);
  let activeRootId = $state<number | null>(null);
  let currentPath = $state('');
  let currentReadOnly = $state(false);
  let currentReadOnlyReason = $state<'policy' | 'filesystem' | null>(null);
  let viewRootId = $state<number | null>(null);
  let viewPath = $state('');
  let pathBarEl = $state<HTMLElement | null>(null);
  let collapsedCount = $state(0);
  let pathMenuOpen = $state(false);
  let clock = $state(Date.now());
  let entries = $state<Entry[]>([]);
  let selected = $state<string[]>([]);
  let dirSizes = $state<Record<string, { size: number; stale: boolean } | null>>({});
  let dirSizeErrors = $state<Record<string, string | null>>({});
  let selectionAnchor = $state<string | null>(null);
  let filesLoading = $state(false);
  let filesRefreshing = $state(false);
  let filesLoadingMore = $state(false);
  let listGeneration = 0;
  let listAbort: AbortController | null = null;
  let loadMoreAbort: AbortController | null = null;
  let usageAbort: AbortController | null = null;
  let usagePollTimer: ReturnType<typeof setTimeout> | null = null;
  let usagePollAttempts = 0;
  let bootSlow = $state(false);
  let bootSlowTimer: ReturnType<typeof setTimeout> | null = null;
  let filesSlow = $state(false);
  let filesSlowTimer: ReturnType<typeof setTimeout> | null = null;
  let searchAbort: AbortController | null = null;
  let dirSizeGeneration = 0;
  let contentElement = $state<HTMLElement | null>(null);
  let fileTable = $state<HTMLTableElement | null>(null);
  let gridElement = $state<HTMLElement | null>(null);
  let virtualStart = $state(0);
  let virtualEnd = $state(0);
  let virtualPadTop = $state(0);
  let virtualPadBottom = $state(0);
  let nextCursor = $state<string | null>(null);
  let focusPage = $state<{ offset: number; total: number } | null>(null);
  let filesError = $state('');

  let fileInput = $state<HTMLInputElement | null>(null);
  let folderInput = $state<HTMLInputElement | null>(null);
  const shareToken = (() => {
    const match = window.location.pathname.match(/^\/s\/([^/]+)/);
    const token = match?.[1];
    return token ? decodeURIComponent(token) : null;
  })();
  let searchQuery = $state('');
  let searchResults = $state<SearchEntry[]>([]);
  let searchActive = $state(false);
  let searchLoading = $state(false);
  let searchLoadingMore = $state(false);
  let searchNextCursor = $state<string | null>(null);
  let searchTooShort = $state(false);
  let searchFailed = $state(false);
  let searchView = $state<'list' | 'grid'>('list');
  let searchSort = $state<'name' | 'size' | 'modified'>('name');
  let searchSortOrder = $state<'asc' | 'desc'>('asc');
  let searchSelected = $state<string[]>([]);
  let searchShare = $state<{
    rootId: number;
    paths: string[];
    name: string;
    isDir: boolean;
  } | null>(null);
  let searchMoreAbort: AbortController | null = null;
  let searchInput = $state<HTMLInputElement | null>(null);
  let searchFiltersOpen = $state(false);
  let searchKind = $state('');
  let searchScope = $state('');
  let searchMinMb = $state('');
  let searchMaxMb = $state('');
  let searchModified = $state('');
  let searchToken = 0;
  let editorOpen = $state(false);
  let editorName = $state('');
  let editorPath = $state('');
  let editorContent = $state('');
  let editorOriginalContent = $state('');
  let editorEtag = $state('');
  let editorError = $state('');
  let editorSaving = $state(false);
  let editorPreview = $state(false);
  let editorReadOnly = $state(false);
  let editorReadOnlyReason = $state<'file' | 'directory' | 'policy' | 'lossy' | null>(null);
  let editorLineEnding = $state<'lf' | 'crlf' | 'mixed'>('lf');
  let editorEncoding = $state('utf-8');
  let editorBom = $state(false);
  let editorLossy = $state(false);
  // Live Markdown preview and the line-number gutter are debounced: parsing a
  // multi-megabyte document on every keystroke stalls the main thread.
  const LIVE_PREVIEW_LIMIT = 300_000;
  let editorPreviewSource = $state('');
  let editorPreviewStale = $state(false);
  let previewTimer: ReturnType<typeof setTimeout> | null = null;
  let previewElement = $state<HTMLDivElement | null>(null);
  let thumbVersions = $state<Record<string, number>>({});
  let thumbFailures = $state<Record<string, boolean>>({});
  let remoteEditors = $state<Record<string, Record<string, number>>>({});
  let presenceRoot: number | null = null;
  let presencePath = '';
  let presenceTimer: ReturnType<typeof setInterval> | null = null;
  const presenceClient = (() => {
    try {
      const existing = sessionStorage.getItem('filehub.edit.client');
      if (existing) return existing;
      const next =
        typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
          ? crypto.randomUUID()
          : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
      sessionStorage.setItem('filehub.edit.client', next);
      return next;
    } catch {
      return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
    }
  })();
  let usage = $state<Usage | null>(null);
  let marquee = $state<{ x1: number; y1: number; x2: number; y2: number } | null>(null);
  let marqueePreview = $state<string[]>([]);
  let suppressSelectionClear = false;
  let appView = $state<'files' | 'settings' | 'admin' | 'audit' | 'shares'>('files');
  let settingsMounted = $state(false);
  let sharesMounted = $state(false);
  let adminMounted = $state(false);
  let contextMenu = $state<{ x: number; y: number; entry: Entry } | null>(null);
  let searchMenu = $state<{ x: number; y: number; result: SearchEntry } | null>(null);
  let searchShareTarget = $state<SearchEntry | null>(null);
  let contextMenuElement = $state<HTMLElement | null>(null);
  let contextMenuLeft = $state(false);
  let contextMenuUp = $state(false);
  let sortMenuOpen = $state(false);
  let uploadBatchStarted = 0;
  let uploadBatchBytes = 0;
  let uploadBatchCount = 0;
  let uploadBatchRoot: number | null = null;
  let uploadBatchPath = '';
  const shareNotifySeen = new SvelteMap<string, number>();
  let blankMenu = $state<{ x: number; y: number } | null>(null);
  let blankMenuElement = $state<HTMLElement | null>(null);
  let blankMenuLeft = $state(false);
  let blankMenuUp = $state(false);
  let terminalOpen = $state(false);
  let terminalMinimized = $state(false);
  let rootsRefreshTimer: number | null = null;
  let dirVersionTimer: ReturnType<typeof setInterval> | null = null;
  let lastDirVersion = '';
  let jobProgressRestored = $state(false);
  let terminalAvailable = $derived(
    capabilities.terminal &&
      systemFeatures.terminal &&
      preferences?.terminal_enabled !== false,
  );

  $effect(() => {
    if (!terminalAvailable) {
      terminalOpen = false;
      terminalMinimized = false;
    }
  });
  let transfer = $state<{ mode: 'copy' | 'move'; entries: Entry[] } | null>(null);
  let dragPayload = $state<{
    rootId: number;
    paths: string[];
    names: string[];
    dirs: string[];
  } | null>(null);
  let dragOverKey = $state<string | null>(null);
  let dragOverValid = $state(true);
  let nameDialog = $state<{
    mode: 'mkdir' | 'touch' | 'rename' | 'archive';
    entry: Entry | null;
    title: string;
    label: string;
    initialValue: string;
  } | null>(null);
  let archiveTargets = $state<Entry[]>([]);
  let dirStatsOpen = $state(false);
  let preview = $state<{ items: Entry[]; index: number } | null>(null);
  let properties = $state<{
    data: Awaited<ReturnType<typeof api.properties>> | null;
    loading: boolean;
    error: string;
  } | null>(null);
  let shareTargets = $state<Entry[] | null>(null);
  let confirmState = $state<{
    title: string;
    message: string;
    items?: { name: string; isDir: boolean }[];
    requireText: string | null;
    action: (value: string) => void;
  } | null>(null);
  let editorDirty = $derived(editorContent !== editorOriginalContent);
  let editorPresenceKey = $derived(activeRootId === null ? '' : `${activeRootId}:${editorPath}`);
  let editorPeerEditing = $derived(
    editorOpen &&
      editorPresenceKey !== '' &&
      Object.keys(remoteEditors[editorPresenceKey] ?? {}).length > 0,
  );
  let previewHtml = $derived(
    editorPreview ? DOMPurify.sanitize(String(marked.parse(editorPreviewSource))) : '',
  );

  $effect(() => {
    const html = previewHtml;
    const element = previewElement;
    if (!editorPreview || !html || !element) return;
    let cancelled = false;
    void Promise.all([
      import('prismjs'),
      import('prismjs/components/prism-javascript'),
      import('prismjs/components/prism-typescript'),
      import('prismjs/components/prism-json'),
      import('prismjs/components/prism-python'),
      import('prismjs/components/prism-rust'),
      import('prismjs/components/prism-bash'),
      import('prismjs/components/prism-markup'),
      import('prismjs/components/prism-css'),
      import('prismjs/components/prism-markdown'),
      import('prismjs/components/prism-yaml'),
      import('prismjs/components/prism-sql'),
    ])
      .then(([module]) => {
        if (cancelled) return;
        const Prism = module.default;
        for (const code of element.querySelectorAll('pre code')) {
          Prism.highlightElement(code);
        }
      })
      .catch((error) => {
        console.warn('Prism failed to load', error);
      });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    const root = activeRootId;
    const path = currentPath;
    const view = appView;
    if (dirVersionTimer) {
      clearInterval(dirVersionTimer);
      dirVersionTimer = null;
    }
    lastDirVersion = '';
    if (root === null || view !== 'files') return;
    let disposed = false;
    const check = async (): Promise<void> => {
      if (document.visibilityState === 'hidden') return;
      try {
        const result = await api.dirVersion(root, path);
        if (disposed) return;
        const next = String(result.mtime);
        if (lastDirVersion === '') {
          lastDirVersion = next;
          return;
        }
        if (lastDirVersion !== next) {
          lastDirVersion = next;
          void loadList(root, path);
        }
      } catch (error) {
        if (!(error instanceof ApiError) || error.status !== 404) {
          console.warn('directory change probe failed', error);
        }
      }
    };
    void check();
    dirVersionTimer = setInterval(() => void check(), 5_000);
    return () => {
      disposed = true;
      if (dirVersionTimer) {
        clearInterval(dirVersionTimer);
        dirVersionTimer = null;
      }
    };
  });

  $effect(() => {
    const content = editorContent;
    const active = editorPreview;
    if (!editorOpen || !active) {
      editorPreviewSource = '';
      editorPreviewStale = false;
      return;
    }
    if (content.length > LIVE_PREVIEW_LIMIT) {
      editorPreviewStale = true;
      return;
    }
    if (previewTimer) clearTimeout(previewTimer);
    previewTimer = setTimeout(() => {
      previewTimer = null;
      editorPreviewSource = content;
      editorPreviewStale = false;
    }, 120);
    return () => {
      if (previewTimer) {
        clearTimeout(previewTimer);
        previewTimer = null;
      }
    };
  });

  function toggleEditorPreview(): void {
    editorPreview = !editorPreview;
    if (!editorPreview) return;
    if (editorContent.length > LIVE_PREVIEW_LIMIT) {
      editorPreviewSource = '';
      editorPreviewStale = true;
    } else {
      editorPreviewSource = editorContent;
      editorPreviewStale = false;
    }
  }

  function refreshEditorPreview(): void {
    editorPreviewSource = editorContent;
    editorPreviewStale = false;
  }

  let menuTargets = $derived(
    contextMenu
      ? selected.includes(contextMenu.entry.name) && selected.length > 1
        ? entries.filter((entry) => selected.includes(entry.name))
        : [contextMenu.entry]
      : [],
  );
  let menuMedia = $derived(
    menuTargets.filter(
      (entry) => !entry.is_dir && (isImageName(entry.name) || isVideoName(entry.name)),
    ),
  );

  let selectionSummary = $derived.by(() => {
    const picked = entries.filter((entry) => selected.includes(entry.name));
    const counts: Record<string, number> = {};
    let bytes = 0;
    for (const entry of picked) {
      const kind = classifyFile(entry.name, entry.is_dir, entry.mime_type);
      const label =
        kind === 'folder'
          ? t($language, '目录')
          : kind === 'image'
            ? t($language, '图片')
            : kind === 'video'
              ? t($language, '视频')
              : kind === 'audio'
                ? t($language, '音频')
                : kind === 'pdf' || kind === 'text'
                  ? t($language, '文档')
                  : t($language, '其他');
      counts[label] = (counts[label] ?? 0) + 1;
      if (!entry.is_dir) bytes += entry.size;
    }
    return {
      bytes,
      parts: Object.entries(counts).map(([label, count]) => `${label} ${count}`),
    };
  });

  let activeRoot = $derived(roots.find((root) => root.id === activeRootId) ?? null);
  const localRoots = $derived(roots.filter((root) => root.kind !== 'remote'));
  const remoteRoots = $derived(roots.filter((root) => root.kind === 'remote'));
  let remoteCaps = $state<Record<number, string[]>>({});
  let remoteMounts = $state<Record<number, RemoteMount>>({});
  let currentStale = $state(false);
  const activeRemoteCaps = $derived(
    activeRoot?.kind === 'remote' ? (remoteCaps[activeRoot.id] ?? []) : null,
  );
  function remoteCan(capability: string): boolean {
    return activeRemoteCaps === null || activeRemoteCaps.includes(capability);
  }

  function remoteStateBadge(rootId: number): string | null {
    const mount = remoteMounts[rootId];
    if (!mount || mount.state === 'online') return null;
    return remoteStateLabel(mount.state);
  }

  function remoteTitle(rootId: number, fallback: string): string {
    const mount = remoteMounts[rootId];
    if (!mount) return fallback;
    const mode = mount.mode === 'rw' ? t($language, '读写') : t($language, '只读');
    const state = remoteStateLabel(mount.state);
    return `${mount.alias} · ${mount.base_url} · ${mount.grant_label} · ${mode} · ${state}`;
  }

  function remoteStateLabel(state: RemoteMount['state']): string {
    switch (state) {
      case 'online':
        return t($language, '在线');
      case 'offline':
        return t($language, '离线');
      case 'token_invalid':
        return t($language, '令牌无效');
      case 'grant_revoked':
        return t($language, '授权已撤销');
      case 'unsupported':
        return t($language, '对端不支持');
    }
  }
  let pathSegments = $derived(currentPath.split('/').filter(Boolean));
  let quotaPercent = $derived(
    usage?.quota_bytes && usage.used_bytes !== null
      ? Math.min(100, (usage.used_bytes / usage.quota_bytes) * 100)
      : 0,
  );
  let quotaLabel = $derived(
    usage
      ? usage.used_bytes === null
        ? t($language, '统计中…')
        : `${formatSize(usage.used_bytes)} / ${usage.quota_bytes ? formatSize(usage.quota_bytes) : '∞'}`
      : '—',
  );
  let pageTitle = $derived.by(() => {
    const brand = $branding.name;
    if (appView === 'shares') return `${brand} - ${t($language, '分享管理')}`;
    if (appView === 'settings') return `${brand} - ${t($language, '设置')}`;
    if (appView === 'admin') return `${brand} - ${t($language, '系统管理')}`;
    if (appView === 'audit') return `${brand} - ${t($language, '审计日志')}`;
    const current =
      pathSegments[pathSegments.length - 1] ||
      (activeRoot ? rootLabel(activeRoot.alias) : undefined);
    return current ? `${brand} - ${current}` : brand;
  });

  const VIRTUAL_THRESHOLD = 300;
  const VIRTUAL_ROW_HEIGHT = 49;
  let visibleEntries = $derived(
    entries.length > VIRTUAL_THRESHOLD ? entries.slice(virtualStart, virtualEnd) : entries,
  );

  $effect(() => {
    void entries.length;
    void view;
    void contentElement;
    void fileTable;
    void gridElement;
    updateVirtualRange();
  });

  $effect(() => {
    if (appView === 'settings') settingsMounted = true;
    if (appView === 'shares') sharesMounted = true;
    if (appView === 'admin') adminMounted = true;
  });

  onMount(() => {
    initLanguage();
    void loadBranding();
    try {
      theme = resolveTheme(
        localStorage.getItem('filehub.theme'),
        window.matchMedia('(prefers-color-scheme: dark)').matches,
      );
    } catch (error) {
      console.warn('could not read theme preference', error);
    }

    applyTheme(theme);
    if (!shareToken) {
      void bootstrap();
    } else {
      loading = false;
    }

    const onKeyDown = (event: KeyboardEvent) => {
      const typing = isTypingTarget(event.target);

      if (event.key === '/' && !typing) {
        event.preventDefault();
        searchInput?.focus();
        return;
      }
      if (
        (event.key === '`' || event.key === '~') &&
        (terminalOpen || !typing) &&
        activeRoot &&
        terminalAvailable
      ) {
        event.preventDefault();
        terminalOpen = !terminalOpen;
        terminalMinimized = false;
        return;
      }
      if (event.key === 'Escape') {
        if (pathMenuOpen) {
          pathMenuOpen = false;
        } else if (editorOpen) {
          closeEditor();
        } else if (searchActive) {
          exitSearch();
        } else if (sidebarOpen) {
          sidebarOpen = false;
        } else {
          clearSelection();
        }
        return;
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's' && editorOpen) {
        event.preventDefault();
        void saveEditor();
        return;
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'a' && !typing) {
        event.preventDefault();
        selected = entries.map((entry) => entry.name);
        return;
      }
      if (event.key === 'Delete' && selected.length > 0 && !typing) {
        event.preventDefault();
        void deleteSelected();
      }
    };

    const unsubscribeEvents = realtimeEvent.subscribe((event) => {
      if (!event) return;
      if (event.type === 'usage.updated') {
        void loadUsage();
      }
      if (event.type === 'thumbnail.ready' || event.type === 'thumbnail.failed') {
        handleThumbnailEvent(event.type, event.data);
      }
      if (event.type === 'edit.presence') {
        handlePresenceEvent(event.data);
      }
      if (event.type === 'user.features_changed') {
        void loadPreferences();
      }
      if (event.type === 'roots.changed') {
        scheduleRootsRefresh();
      }
      if (event.type === 'settings.changed') {
        void loadPreferences();
        void loadBranding();
      }
      if (event.type === 'resync') {
        scheduleRootsRefresh();
        void loadUsage();
        void loadPreferences();
        if (user) void restoreJobs(user.id);
      }
      if (event.type === 'thumbnail.batch') {
        const items = event.data.events;
        if (Array.isArray(items)) {
          for (const item of items) {
            if (
              item &&
              typeof item === 'object' &&
              (item as { type?: unknown }).type !== undefined
            ) {
              const typed = item as { type: string; data: Record<string, unknown> };
              handleThumbnailEvent(typed.type, typed.data);
            }
          }
        }
      }
      if (event.type === 'notification') {
        const kind = String(event.data.kind ?? 'notice');
        const message =
          kind === 'job_failed' && event.data.job_kind
            ? jobFailureMessage(event.data)
            : String(event.data.message ?? '');
        if (message) {
          addNotification(kind, message);
          showToast(message);
        }
      }
      if (
        event.type === 'share.visited' ||
        event.type === 'share.downloaded' ||
        event.type === 'share.uploaded'
      ) {
        notifyShareActivity(event.type, event.data);
      }
    });
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('resize', updateVirtualRange);
    const autoSaveTimer = window.setInterval(() => {
      if (editorOpen && editorDirty && !editorSaving && !editorReadOnly) {
        void saveEditor(true);
      }
    }, 30_000);
    const clockTimer = window.setInterval(() => (clock = Date.now()), 60_000);
    const presencePruneTimer = window.setInterval(pruneRemotePresence, 5000);
    const onPageHide = () => {
      if (presenceRoot !== null) {
        void sendPresence('close', presenceRoot, presencePath, true);
        stopPresenceHeartbeat();
      }
    };
    const onPageShow = () => {
      if (editorOpen && activeRootId !== null) {
        beginEditPresence(activeRootId, editorPath);
      }
    };
    window.addEventListener('pagehide', onPageHide);
    window.addEventListener('pageshow', onPageShow);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('resize', updateVirtualRange);
      window.removeEventListener('pagehide', onPageHide);
      window.removeEventListener('pageshow', onPageShow);
      window.clearInterval(autoSaveTimer);
      window.clearInterval(clockTimer);
      window.clearInterval(presencePruneTimer);
      stopPresenceHeartbeat();
      unsubscribeEvents();
      if (usagePollTimer) clearTimeout(usagePollTimer);
      if (bootSlowTimer) clearTimeout(bootSlowTimer);
      if (filesSlowTimer) clearTimeout(filesSlowTimer);
    };
  });

  function setTheme(next: ThemePreference): void {
    const prefersDark =
      typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: dark)').matches;
    theme = next === 'system' ? (prefersDark ? 'dark' : 'light') : next;
    applyTheme(theme);
    try {
      localStorage.setItem('filehub.theme', next);
    } catch (error) {
      console.warn('could not persist theme preference', error);
    }
  }

  function setThemePreference(next: ThemePreference): void {
    setTheme(next);
    void savePreference({ theme: next });
  }

  function toggleTheme(): void {
    setThemePreference(theme === 'dark' ? 'light' : 'dark');
  }

  function startBootSlowHint(): void {
    if (bootSlowTimer) clearTimeout(bootSlowTimer);
    bootSlow = false;
    bootSlowTimer = setTimeout(() => {
      bootSlow = true;
      bootSlowTimer = null;
    }, 6000);
  }

  function stopBootSlowHint(): void {
    if (bootSlowTimer) clearTimeout(bootSlowTimer);
    bootSlowTimer = null;
    bootSlow = false;
  }

  function startFilesSlowHint(): void {
    if (filesSlowTimer) clearTimeout(filesSlowTimer);
    filesSlow = false;
    filesSlowTimer = setTimeout(() => {
      filesSlow = true;
      filesSlowTimer = null;
    }, 4000);
  }

  function stopFilesSlowHint(): void {
    if (filesSlowTimer) clearTimeout(filesSlowTimer);
    filesSlowTimer = null;
    filesSlow = false;
  }

  async function bootstrap(): Promise<void> {
    loading = true;
    startBootSlowHint();

    try {
      const status = await api.setupStatus();
      initialized = status.initialized;

      if (!initialized) {
        return;
      }

      try {
        const auth = await api.me();
        user = auth.user;
        userRemote = auth.remote ?? null;
        csrf = auth.csrf_token;
        ensureNotificationOwner(auth.user.id);
        connectEvents(auth.user.id);
        attachUploads(auth.user.id);
        void restoreJobs(auth.user.id);
        await loadPreferences();
        await loadRoots();
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 401)) {
          console.warn('could not restore session', error);
        }
      }
    } catch (error) {
      console.warn('could not read setup status', error);
      initialized = true;
    } finally {
      loading = false;
      stopBootSlowHint();
    }
  }

  async function loadVersionInfo(): Promise<void> {
    try {
      versionInfo = await api.version();
    } catch {
      // Update checks are best-effort; keep the baked version.
    }
  }

  function handleAuth(auth: AuthResponse): void {
    user = auth.user;
    userRemote = auth.remote ?? null;
    csrf = auth.csrf_token;
    ensureNotificationOwner(auth.user.id);
    connectEvents(auth.user.id);
    attachUploads(auth.user.id);
    void restoreJobs(auth.user.id);
    initialized = true;
    loading = false;
    stopBootSlowHint();
    void loadPreferences();
    void loadRoots();
    void loadVersionInfo();
  }

  async function restoreJobs(userId: number): Promise<void> {
    const seenKey = `filehub.jobs.seen.${userId}`;
    const notifiedKey = `filehub.jobs.notified.${userId}`;
    const stored = Number(localStorage.getItem(seenKey));
    const since =
      Number.isFinite(stored) && stored > 0 ? Math.floor(stored) : Math.floor(Date.now() / 1000);
    let notified: string[] = [];
    try {
      const raw = JSON.parse(localStorage.getItem(notifiedKey) ?? '[]') as unknown;
      if (Array.isArray(raw)) {
        notified = raw.filter((value): value is string => typeof value === 'string');
      }
    } catch {
      notified = [];
    }

    let jobs: JobInfo[];
    try {
      jobs = await api.jobs(since);
    } catch (error) {
      if (!(error instanceof ApiError && error.status === 401)) {
        console.warn('could not restore jobs', error);
      }
      return;
    }

    const active = jobs
      .filter((job) => job.status === 'queued' || job.status === 'running')
      .filter((job) => job.kind === 'copy' || job.kind === 'move')
      .sort((left, right) => right.created_at - left.created_at);
    if (active.length > 0 && !jobProgressRestored) {
      jobProgressRestored = true;
      const queue = active.slice(0, 5);
      const runRestored = async (): Promise<void> => {
        const job = queue.shift();
        if (!job) return;
        startProgress(
          t($language, '{kind}任务', { kind: jobKindLabel(job.kind) }),
          100,
          t($language, '正在恢复任务…'),
        );
        try {
          await waitForTransferJob(job.id);
        } catch (error) {
          console.warn('restored job failed', error);
        }
        if (activeRootId !== null) await loadList(activeRootId, currentPath);
        await runRestored();
      };
      void runRestored();
    }

    for (const job of jobs) {
      if (job.kind !== 'copy' && job.kind !== 'move') continue;
      if (!job.finished_at || job.finished_at < since || notified.includes(job.id)) continue;
      if (job.status === 'completed') {
        notified.push(job.id);
        addNotification(
          'job_completed',
          t($language, '{kind}任务已完成', { kind: jobKindLabel(job.kind) }),
        );
        showToast(t($language, '{kind}任务已完成', { kind: jobKindLabel(job.kind) }));
      } else if (job.status === 'failed') {
        notified.push(job.id);
        addNotification(
          'job_failed',
          t($language, '{kind}任务失败：{message}', {
            kind: jobKindLabel(job.kind),
            message: job.error ?? t($language, '操作失败'),
          }),
        );
      }
    }
    try {
      localStorage.setItem(seenKey, String(Math.floor(Date.now() / 1000)));
      localStorage.setItem(notifiedKey, JSON.stringify(notified.slice(-100)));
    } catch {
      // Storage may be unavailable; restoring jobs still works.
    }
  }

  function jobFailureMessage(data: Record<string, unknown>): string {
    const kind = jobKindLabel(String(data.job_kind ?? ''));
    const label = String(data.label ?? data.path ?? '');
    const code = String(data.code ?? '');
    const reason =
      code === 'NOT_FOUND'
        ? t($language, '不存在或已被删除')
        : code === 'PERMISSION_DENIED'
          ? t($language, '无写入权限')
          : code === 'FORBIDDEN'
            ? t($language, '目标只读')
            : code === 'CONFLICT'
              ? t($language, '目标已存在')
              : code === 'TOO_LARGE'
                ? t($language, '文件过大')
                : String(data.message ?? t($language, '操作失败'));
    return label
      ? t($language, '{kind}失败：{label}（{reason}）', { kind, label, reason })
      : t($language, '{kind}失败：{reason}', { kind, reason });
  }

  function shareNotifyEnabled(): boolean {
    try {
      return localStorage.getItem('filehub.notify.share_activity') !== '0';
    } catch {
      return true;
    }
  }

  function notifyShareActivity(type: string, data: Record<string, unknown>): void {
    if (!shareNotifyEnabled()) return;
    const id = String(data.id ?? '');
    const name = String(data.name || t($language, '文件合集'));
    const item = String(data.path ?? data.name ?? '');
    const windowMs = type === 'share.visited' ? 10 * 60_000 : 5 * 60_000;
    const key = type === 'share.visited' ? `visit:${id}` : `${type}:${id}:${item}`;
    const now = Date.now();
    const last = shareNotifySeen.get(key);
    if (last !== undefined && now - last < windowMs) return;
    shareNotifySeen.set(key, now);
    if (shareNotifySeen.size > 500) {
      for (const [entryKey, at] of shareNotifySeen) {
        if (now - at > 60 * 60_000) shareNotifySeen.delete(entryKey);
      }
    }
    const message =
      type === 'share.visited'
        ? t($language, '分享「{name}」被访问', { name })
        : type === 'share.downloaded'
          ? t($language, '分享「{name}」被下载：{item}', { name, item })
          : t($language, '分享「{name}」收到上传：{item}', { name, item });
    addNotification('share_activity', message);
  }

  function shouldNotifyLong(started: number, bytes: number, count: number, left: boolean): boolean {
    return (
      left ||
      (typeof document !== 'undefined' && document.hidden) ||
      Date.now() - started >= 3000 ||
      bytes >= 100 * 1024 * 1024 ||
      count >= 50
    );
  }

  function notifyLongCompletion(message: string, left: boolean): void {
    addNotification('job_completed', message);
    if (left || (typeof document !== 'undefined' && document.hidden)) showToast(message);
  }

  function jobKindLabel(kind: string): string {
    if (kind === 'copy') return t($language, '复制');
    if (kind === 'move') return t($language, '移动');
    if (kind === 'index') return t($language, '索引');
    if (kind === 'dir_size') return t($language, '目录大小');
    if (kind === 'thumbnail') return t($language, '缩略图');
    return kind;
  }

  function attachUploads(userId: number): void {
    uploadManager.attach(userId, () => csrf, {
      onUploaded: (rootId, directory) => {
        clearDirSizeCache();
        if (rootId === activeRootId && directory === currentPath) {
          void loadList(rootId, currentPath);
        }
      },
      onNotice: (message, kind) => showNotice(message, kind),
      onAllDone: () => {
        showToast(t($language, '上传完成'));
        if (
          shouldNotifyLong(
            uploadBatchStarted,
            uploadBatchBytes,
            uploadBatchCount,
            activeRootId !== uploadBatchRoot || currentPath !== uploadBatchPath,
          )
        ) {
          notifyLongCompletion(
            t($language, '上传完成：{count} 项 · {size}', {
              count: uploadBatchCount,
              size: formatSize(uploadBatchBytes),
            }),
            activeRootId !== uploadBatchRoot || currentPath !== uploadBatchPath,
          );
        }
        uploadBatchStarted = 0;
        uploadBatchBytes = 0;
        uploadBatchCount = 0;
        uploadBatchRoot = null;
        uploadBatchPath = '';
      },
    });
    if (uploadManager.pendingCount > 0) {
      showNoticeAction(
        t($language, '有 {count} 个上传未完成，重新选择相同文件即可续传', {
          count: uploadManager.pendingCount,
        }),
        t($language, '继续上传'),
        uploadManager.pendingHasDirectories ? t($language, '选择目录') : t($language, '选择文件'),
        () => (uploadManager.pendingHasDirectories ? folderInput?.click() : fileInput?.click()),
        'info',
      );
    }
  }

  async function loadPreferences(): Promise<void> {
    try {
      const result = await api.getPreferences();
      systemFeatures = result.system;
      capabilities = result.capabilities;
      applyPreferences(result.preferences, false);
    } catch (error) {
      if (!(error instanceof ApiError && error.status === 401)) {
        console.warn('could not load preferences', error);
      }
    }
  }

  function applyPreferences(next: UserPreferences, reload = true): void {
    const hiddenChanged = preferences?.show_hidden !== next.show_hidden;
    preferences = next;
    setTheme(next.theme);
    setLanguage(next.language);
    if (!next.terminal_enabled) terminalOpen = false;
    if (reload && hiddenChanged && activeRootId !== null) {
      exitSearch();
      void loadList(activeRootId, currentPath);
    }
  }

  async function savePreference(patch: Parameters<typeof api.updatePreferences>[0]): Promise<void> {
    if (!user) return;
    try {
      const result = await api.updatePreferences(patch, csrf);
      systemFeatures = result.system;
      capabilities = result.capabilities;
      applyPreferences(result.preferences);
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '偏好保存失败'));
    }
  }

  async function logout(): Promise<void> {
    if (loggingOut) return;

    loggingOut = true;
    endEditPresence();
    try {
      await api.logout(csrf);
    } catch (error) {
      console.warn('logout request failed', error);
    } finally {
      clearNotifications();
      uploadManager.detach();
      jobProgressRestored = false;
      user = null;
      preferences = null;
      csrf = '';
      disconnectEvents();
      try {
        for (let index = localStorage.length - 1; index >= 0; index -= 1) {
          const key = localStorage.key(index);
          if (key?.startsWith('filehub.terminal.history')) localStorage.removeItem(key);
        }
      } catch (error) {
        console.warn('failed to clear terminal history', error);
      }
      roots = [];
      entries = [];
      currentPath = '';
      viewRootId = null;
      viewPath = '';
      currentReadOnly = false;
      selected = [];
      activeRootId = null;
      usagePollAttempts = 0;
      if (usagePollTimer) clearTimeout(usagePollTimer);
      usagePollTimer = null;
      loggingOut = false;
    }
  }

  async function loadRoots(): Promise<void> {
    try {
      const result = await api.roots();
      roots = result.roots;
      const next = roots.find((root) => root.id === activeRootId) ?? roots[0] ?? null;
      if (!next) {
        activeRootId = null;
        currentPath = '';
        viewRootId = null;
        viewPath = '';
        currentReadOnly = false;
        currentReadOnlyReason = null;
        entries = [];
        filesError = t($language, '当前账号还没有可访问的目录');
        return;
      }
      await loadList(next.id, '');
    } catch (error) {
      handleFilesError(error);
    }
  }

  /// Refresh the sidebar roots after an admin change without resetting the
  /// current browse position unless the root disappeared.
  function scheduleRootsRefresh(): void {
    if (rootsRefreshTimer !== null) window.clearTimeout(rootsRefreshTimer);
    rootsRefreshTimer = window.setTimeout(() => {
      rootsRefreshTimer = null;
      void refreshRoots();
    }, 120);
  }

  async function refreshRemoteCaps(): Promise<void> {
    try {
      const result = await api.remoteMounts();
      const next: Record<number, string[]> = {};
      const mounts: Record<number, RemoteMount> = {};
      for (const mount of result.mounts) {
        next[mount.root_id] = mount.capabilities;
        mounts[mount.root_id] = mount;
      }
      remoteCaps = next;
      remoteMounts = mounts;
    } catch (error) {
      console.debug('remote mount capabilities unavailable', error);
    }
  }

  async function refreshRoots(): Promise<void> {
    try {
      const result = await api.roots();
      roots = result.roots;
      const next = roots.find((root) => root.id === activeRootId) ?? roots[0] ?? null;
      if (!next) {
        activeRootId = null;
        currentPath = '';
        currentReadOnly = false;
        currentReadOnlyReason = null;
        entries = [];
        filesError = t($language, '当前账号还没有可访问的目录');
        return;
      }
      await loadList(next.id, next.id === activeRootId ? currentPath : '');
      void refreshRemoteCaps();
    } catch (error) {
      handleFilesError(error);
    }
  }

  function cancelThumbScope(
    rootId: number,
    path: string,
    scope: 'dir' | 'file',
  ): void {
    void api.cancelThumbScope(rootId, path, scope, csrf).catch((error) => {
      console.warn('cancel thumbnail scope failed', error);
    });
  }

  let thumbWarmTimer = 0;
  const thumbWarmSeen: Record<string, number> = {};

  function scheduleRemoteThumbWarm(
    root: Root,
    path: string,
    items: Entry[],
  ): void {
    if (root.kind !== 'remote') return;
    const cacheKey = `${root.id}:${path}`;
    const now = Date.now();
    if (now - (thumbWarmSeen[cacheKey] ?? 0) < 300_000) return;
    if (thumbWarmTimer) window.clearTimeout(thumbWarmTimer);
    thumbWarmTimer = window.setTimeout(() => {
      thumbWarmSeen[cacheKey] = Date.now();
      const paths = items
        .filter((entry) => !entry.is_dir && entry.has_preview)
        .slice(0, 100)
        .map((entry) => entry.name);
      if (paths.length === 0) return;
      void api.remoteThumbWarm(root.id, path, paths, csrf).catch((error) => {
        console.debug('remote thumbnail warm skipped', error);
      });
    }, 800);
  }

  async function loadList(rootId: number, path: string, focus?: string): Promise<boolean> {
    const generation = ++listGeneration;
    listAbort?.abort();
    loadMoreAbort?.abort();
    abortDirSizeRequests();
    dirSizeGeneration += 1;
    const controller = new AbortController();
    listAbort = controller;
    focusPage = null;
    if (
      viewRootId !== null &&
      (viewRootId !== rootId || viewPath !== path)
    ) {
      cancelThumbScope(viewRootId, viewPath, 'dir');
    }
    const refreshing =
      viewRootId === rootId && viewPath === path && entries.length > 0;
    filesLoading = !refreshing;
    filesRefreshing = refreshing;
    filesError = '';
    thumbFailures = {};
    nextCursor = null;
    clearDrag();
    startFilesSlowHint();
    try {
      const storedView = resolveViewPreference(
        localStorage.getItem('filehub.view'),
        localStorage.getItem(`view:${rootId}:${path}`),
      );
      if (storedView) view = storedView;
      const savedSort =
        localStorage.getItem(`sort:${rootId}:${path}`) ?? localStorage.getItem('filehub.sort');
      if (savedSort) {
        const [field, order] = savedSort.split(':');
        if (field === 'name' || field === 'size' || field === 'modified') {
          sortField = field;
          sortOrder = order === 'desc' ? 'desc' : 'asc';
        }
      } else {
        sortField = 'name';
        sortOrder = 'asc';
      }
      const result = await api.list(
        rootId,
        path,
        sortField,
        sortOrder,
        false,
        undefined,
        200,
        controller.signal,
        focus,
      );
      if (generation !== listGeneration || controller.signal.aborted) return false;
      activeRootId = result.root.id;
      currentPath = result.path;
      viewRootId = result.root.id;
      viewPath = result.path;
      currentReadOnly = result.read_only;
      currentReadOnlyReason = result.read_only
        ? (result.read_only_reason ?? 'policy')
        : null;
      currentStale = result.stale === true;
      entries = result.entries;
      nextCursor = result.next_cursor;
      scheduleRemoteThumbWarm(result.root, result.path, result.entries);
      loadDirectorySizes(entries, result.root.id, result.path, dirSizeGeneration, true);
      if (focus && typeof result.focus_index === 'number') {
        const offset = result.focus_offset ?? 0;
        const pageStart = offset - result.focus_index;
        focusPage =
          pageStart > 0
            ? {
                offset,
                total: result.total ?? result.entries.length,
              }
            : null;
        selected = [focus];
        selectionAnchor = focus;
      } else {
        selected = [];
        selectionAnchor = null;
      }
      const viewKey = `view:${result.root.id}:${result.path}`;
      const sortKey = `sort:${result.root.id}:${result.path}`;
      try {
        const remembered = resolveViewPreference(
          localStorage.getItem('filehub.view'),
          localStorage.getItem(viewKey),
        );
        if (remembered) {
          view = remembered;
          // Pin the first resolved view to this directory so a later change
          // in another directory cannot leak back through the global default.
          if (!localStorage.getItem(viewKey)) {
            localStorage.setItem(viewKey, remembered);
          }
          if (!localStorage.getItem('filehub.view')) {
            localStorage.setItem('filehub.view', remembered);
          }
        }

        const savedSort = localStorage.getItem(sortKey) ?? localStorage.getItem('filehub.sort');
        if (savedSort) {
          const [field, order] = savedSort.split(':');
          if (field === 'name' || field === 'size' || field === 'modified') {
            sortField = field;
            sortOrder = order === 'desc' ? 'desc' : 'asc';
          }
        }
        if (!localStorage.getItem(sortKey)) {
          localStorage.setItem(sortKey, `${sortField}:${sortOrder}`);
        }
      } catch (error) {
        console.warn('could not persist view preference', error);
      }
      void loadUsage();
      await tick();
      return true;
    } catch (error) {
      if (isAbortError(error)) return false;
      if (focus && error instanceof ApiError && error.status === 404) return false;
      handleFilesError(error);
      return false;
    } finally {
      if (generation === listGeneration) {
        filesLoading = false;
        filesRefreshing = false;
        stopFilesSlowHint();
      }
    }
  }

  async function loadMore(): Promise<void> {
    if (
      appView !== 'files' ||
      searchActive ||
      activeRootId === null ||
      nextCursor === null ||
      filesLoadingMore
    ) {
      return;
    }
    filesLoadingMore = true;
    const rootId = activeRootId;
    const path = currentPath;
    const generation = listGeneration;
    loadMoreAbort?.abort();
    const controller = new AbortController();
    loadMoreAbort = controller;
    try {
      const result = await api.list(
        rootId,
        path,
        sortField,
        sortOrder,
        false,
        nextCursor,
        200,
        controller.signal,
      );
      if (generation !== listGeneration || controller.signal.aborted) return;
      if (rootId !== activeRootId || path !== currentPath) return;
      entries = [...entries, ...result.entries];
      nextCursor = result.next_cursor;
      loadDirectorySizes(result.entries, result.root.id, result.path, dirSizeGeneration, false);
    } catch (error) {
      if (!isAbortError(error)) handleFilesError(error);
    } finally {
      if (generation === listGeneration) filesLoadingMore = false;
    }
  }

  const dirSizeTimers = new SvelteMap<string, ReturnType<typeof setTimeout>>();
  const dirSizeAttempts = new SvelteMap<string, number>();
  const dirSizeControllers = new SvelteSet<AbortController>();
  const dirSizeQueue: Array<() => void> = [];
  const DIR_SIZE_CONCURRENCY = 6;
  let dirSizeActive = 0;

  function abortDirSizeRequests(): void {
    for (const controller of dirSizeControllers) controller.abort();
    dirSizeControllers.clear();
    dirSizeQueue.length = 0;
    for (const timer of dirSizeTimers.values()) clearTimeout(timer);
    dirSizeTimers.clear();
    dirSizeAttempts.clear();
  }

  function pumpDirSizeQueue(): void {
    while (dirSizeActive < DIR_SIZE_CONCURRENCY && dirSizeQueue.length > 0) {
      const run = dirSizeQueue.shift();
      run?.();
    }
  }

  function enqueueDirectorySize(
    key: string,
    rootId: number,
    path: string,
    generation: number,
  ): void {
    const run = (): void => {
      if (generation !== dirSizeGeneration) {
        pumpDirSizeQueue();
        return;
      }
      dirSizeActive += 1;
      void requestDirectorySize(key, rootId, path, generation).finally(() => {
        dirSizeActive = Math.max(0, dirSizeActive - 1);
        pumpDirSizeQueue();
      });
    };
    if (dirSizeActive < DIR_SIZE_CONCURRENCY) run();
    else dirSizeQueue.push(run);
  }

  function loadDirectorySizes(
    items: Entry[],
    rootId: number,
    path: string,
    generation: number,
    revalidate: boolean,
  ): void {
    for (const entry of items) {
      if (!entry.is_dir) continue;
      const key = `${rootId}:${joinPath(path, entry.name)}`;
      if (dirSizeTimers.has(key)) continue;
      if (!revalidate && dirSizes[key] !== undefined) continue;
      enqueueDirectorySize(key, rootId, joinPath(path, entry.name), generation);
    }
  }

  function keepDirSizeValue(key: string): void {
    const current = dirSizes[key];
    if (current) {
      dirSizes = { ...dirSizes, [key]: { size: current.size, stale: true } };
    } else {
      dirSizes = { ...dirSizes, [key]: null };
    }
  }

  async function requestDirectorySize(
    key: string,
    rootId: number,
    path: string,
    generation: number,
  ): Promise<void> {
    if (generation !== dirSizeGeneration) return;
    const controller = new AbortController();
    dirSizeControllers.add(controller);
    try {
      const result = await api.dirSize(rootId, path, controller.signal);
      if (generation !== dirSizeGeneration) return;
      if (result.status === 'ready' && result.size !== null) {
        const stale = Boolean(result.stale);
        dirSizes = { ...dirSizes, [key]: { size: result.size, stale } };
        dirSizeErrors = { ...dirSizeErrors, [key]: result.error ?? null };
        resortEntriesBySize();
        if (!stale || !result.refreshing) {
          dirSizeAttempts.delete(key);
          return;
        }
      } else if (result.status === 'error') {
        keepDirSizeValue(key);
        dirSizeErrors = {
          ...dirSizeErrors,
          [key]: result.error ?? t($language, '无法统计目录大小'),
        };
        dirSizeAttempts.delete(key);
        return;
      }
      const attempts = (dirSizeAttempts.get(key) ?? 0) + 1;
      if (attempts > 150) {
        keepDirSizeValue(key);
        dirSizeErrors = { ...dirSizeErrors, [key]: t($language, '统计超时') };
        dirSizeAttempts.delete(key);
        return;
      }
      dirSizeAttempts.set(key, attempts);
      const timer = setTimeout(() => {
        dirSizeTimers.delete(key);
        void requestDirectorySize(key, rootId, path, generation);
      }, 2000);
      dirSizeTimers.set(key, timer);
    } catch (error) {
      if (isAbortError(error) || generation !== dirSizeGeneration) return;
      keepDirSizeValue(key);
      dirSizeErrors = {
        ...dirSizeErrors,
        [key]: error instanceof Error ? error.message : t($language, '无法统计目录大小'),
      };
      dirSizeAttempts.delete(key);
    } finally {
      dirSizeControllers.delete(controller);
    }
  }

  function clearDirSizeCache(): void {
    dirSizes = {};
    dirSizeErrors = {};
  }

  function entrySize(entry: Entry): string {
    if (!entry.is_dir) return formatSize(entry.size);
    if (activeRootId === null) return t($language, '计算中…');
    const size = dirSizes[`${activeRootId}:${joinPath(currentPath, entry.name)}`];
    if (size === undefined) {
      return entry.size > 0 ? formatSize(entry.size) : t($language, '计算中…');
    }
    return size === null ? '—' : formatSize(size.size);
  }

  function entrySizeStale(entry: Entry): boolean {
    if (!entry.is_dir || activeRootId === null) return false;
    return dirSizes[`${activeRootId}:${joinPath(currentPath, entry.name)}`]?.stale === true;
  }

  function resortEntriesBySize(): void {
    if (sortField !== 'size' || activeRootId === null) return;
    const rootId = activeRootId;
    const path = currentPath;
    const sizeOf = (entry: Entry): number => {
      if (!entry.is_dir) return entry.size;
      const value = dirSizes[`${rootId}:${joinPath(path, entry.name)}`];
      return value ? value.size : entry.size;
    };
    const known = (entry: Entry, value: number): boolean => !entry.is_dir || value > 0;
    entries = [...entries].sort((left, right) => {
      const dirs = Number(right.is_dir) - Number(left.is_dir);
      if (dirs !== 0) return dirs;
      const leftSize = sizeOf(left);
      const rightSize = sizeOf(right);
      const leftKnown = known(left, leftSize);
      const rightKnown = known(right, rightSize);
      if (leftKnown !== rightKnown) return leftKnown ? -1 : 1;
      if (leftSize !== rightSize) {
        return sortOrder === 'desc' ? rightSize - leftSize : leftSize - rightSize;
      }
      return left.name.localeCompare(right.name);
    });
  }

  function entrySizeError(entry: Entry): string {
    if (!entry.is_dir || activeRootId === null) return '';
    const error = dirSizeErrors[`${activeRootId}:${joinPath(currentPath, entry.name)}`];
    if (error) return error;
    return entrySizeStale(entry) ? t($language, '显示的是上次统计结果，正在后台更新…') : '';
  }

  function handleContentScroll(event: Event): void {
    const element = event.currentTarget as HTMLElement;
    updateVirtualRange();
    if (element.scrollHeight - element.scrollTop - element.clientHeight < 240) {
      void loadMore();
    }
  }

  function galleryMetrics(element: HTMLElement): {
    columns: number;
    rowHeight: number;
    gap: number;
  } {
    const style = getComputedStyle(element);
    const columns = Math.max(
      1,
      style.gridTemplateColumns.split(' ').filter((value) => value.length > 0).length,
    );
    const gap = view === 'thumb' ? 16 : 10;
    const card = element.querySelector<HTMLElement>('.file-card, .file-tile');
    const cardHeight = card?.getBoundingClientRect().height ?? (view === 'thumb' ? 190 : 84);
    return { columns, rowHeight: cardHeight + gap, gap };
  }

  function updateVirtualRange(): void {
    if (!contentElement || entries.length <= VIRTUAL_THRESHOLD) {
      virtualStart = 0;
      virtualEnd = entries.length;
      virtualPadTop = 0;
      virtualPadBottom = 0;
      return;
    }
    if (view === 'list') {
      const table = fileTable;
      if (!table) {
        virtualStart = 0;
        virtualEnd = Math.min(entries.length, 80);
        virtualPadTop = 0;
        virtualPadBottom = 0;
        return;
      }
      const contentRect = contentElement.getBoundingClientRect();
      const tableRect = table.getBoundingClientRect();
      const headerHeight = table.tHead?.getBoundingClientRect().height ?? 0;
      const tableTop = tableRect.top - contentRect.top + contentElement.scrollTop + headerHeight;
      const relativeTop = contentElement.scrollTop - tableTop;
      const overscan = 12;
      const first = Math.floor(relativeTop / VIRTUAL_ROW_HEIGHT) - overscan;
      const count = Math.ceil(contentElement.clientHeight / VIRTUAL_ROW_HEIGHT) + overscan * 2;
      const start = Math.max(0, Math.min(first, entries.length));
      virtualStart = start;
      virtualEnd = Math.min(entries.length, start + count);
      virtualPadTop = 0;
      virtualPadBottom = 0;
      return;
    }

    const grid = gridElement;
    if (!grid) {
      virtualStart = 0;
      virtualEnd = Math.min(entries.length, 60);
      virtualPadTop = 0;
      virtualPadBottom = 0;
      return;
    }
    const metrics = galleryMetrics(grid);
    if (!grid.querySelector<HTMLElement>('.file-card, .file-tile')) {
      virtualStart = 0;
      virtualEnd = Math.min(entries.length, 60);
      virtualPadTop = 0;
      virtualPadBottom = 0;
      requestAnimationFrame(updateVirtualRange);
      return;
    }
    const contentRect = contentElement.getBoundingClientRect();
    const gridRect = grid.getBoundingClientRect();
    const gridTop = gridRect.top - contentRect.top + contentElement.scrollTop;
    const relativeTop = contentElement.scrollTop - gridTop;
    const totalRows = Math.ceil(entries.length / metrics.columns);
    const startRow = Math.max(0, Math.floor(relativeTop / metrics.rowHeight) - 1);
    const visibleRows = Math.ceil(contentElement.clientHeight / metrics.rowHeight) + 3;
    const endRow = Math.min(totalRows, startRow + visibleRows);
    virtualStart = Math.min(entries.length, startRow * metrics.columns);
    virtualEnd = Math.min(entries.length, endRow * metrics.columns);
    virtualPadTop = startRow * metrics.rowHeight;
    virtualPadBottom = Math.max(0, (totalRows - endRow) * metrics.rowHeight);
  }

  function isAbortError(error: unknown): boolean {
    return error instanceof DOMException && error.name === 'AbortError';
  }

  function readOnlyMessage(): string {
    return currentReadOnlyReason === 'filesystem'
      ? t($language, '当前目录只读（文件系统权限不足）')
      : t($language, '当前目录只读');
  }

  function handleFilesError(error: unknown): void {
    if (error instanceof ApiError && error.status === 401) {
      uploadManager.detach();
      user = null;
      csrf = '';
      return;
    }
    if (error instanceof ApiError && error.status === 403) {
      showNotice(
        error.code === 'PERMISSION_DENIED'
          ? t($language, '服务进程没有访问权限（文件系统权限不足）')
          : error.code === 'REMOTE_ROOT_UNSUPPORTED'
            ? t($language, '该操作暂不支持远程目录')
            : t($language, '当前目录只读'),
        'error',
        t($language, '操作被拒绝'),
      );
      return;
    }
    if (error instanceof ApiError && error.status === 404 && appView === 'files') {
      const parent = currentPath.includes('/')
        ? currentPath.slice(0, currentPath.lastIndexOf('/'))
        : '';
      showNoticeAction(
        t($language, '目录不存在或已被删除：{path}', { path: currentPath || '/' }),
        t($language, '目录不可用'),
        t($language, '返回上级'),
        () => navigate(parent),
        'warning',
      );
      return;
    }
    showNotice(error instanceof Error ? error.message : t($language, '文件列表加载失败'));
  }

  async function loadUsage(): Promise<void> {
    if (usagePollTimer) {
      clearTimeout(usagePollTimer);
      usagePollTimer = null;
    }
    usageAbort?.abort();
    const controller = new AbortController();
    usageAbort = controller;
    try {
      usage = await api.usage(controller.signal);
      if (usage.computing || usage.used_bytes === null) {
        usagePollAttempts += 1;
        if (usagePollAttempts <= 150) {
          if (usagePollTimer) clearTimeout(usagePollTimer);
          usagePollTimer = setTimeout(() => {
            usagePollTimer = null;
            void loadUsage();
          }, 2000);
        }
      } else {
        usagePollAttempts = 0;
      }
    } catch (error) {
      if (!isAbortError(error) && !(error instanceof ApiError && error.status === 401)) {
        console.warn('could not load usage', error);
      }
    }
  }

  function navigate(path: string): void {
    pathMenuOpen = false;
    searchActive = false;
    searchQuery = '';
    searchResults = [];
    if (activeRootId !== null) {
      void loadList(activeRootId, path);
    }
  }

  function handleSearchInput(): void {
    const query = searchQuery.trim();
    if (!query) {
      exitSearch();
    }
  }

  function submitSearch(): void {
    const query = searchQuery.trim();
    if (!query || activeRootId === null) {
      exitSearch();
      return;
    }
    const characters = Array.from(query);
    const length = characters.length;
    const hasCjk = characters.some((character) => {
      const code = character.codePointAt(0) ?? 0;
      return (
        (code >= 0x3400 && code <= 0x4dbf) ||
        (code >= 0x4e00 && code <= 0x9fff) ||
        (code >= 0xf900 && code <= 0xfaff)
      );
    });
    if (length < 3 && !(length === 2 && hasCjk)) {
      searchAbort?.abort();
      searchActive = true;
      searchLoading = false;
      searchLoadingMore = false;
      searchNextCursor = null;
      searchTooShort = true;
      searchFailed = false;
      searchResults = [];
      return;
    }
    searchTooShort = false;
    searchFailed = false;
    searchSelected = [];
    searchActive = true;
    searchLoading = true;
    searchLoadingMore = false;
    searchNextCursor = null;
    searchResults = [];
    void runSearch(query);
  }

  function searchFilterParams(): {
    kind?: string;
    minSize?: number;
    maxSize?: number;
    modifiedAfter?: number;
  } {
    const filters: {
      kind?: string;
      minSize?: number;
      maxSize?: number;
      modifiedAfter?: number;
    } = {};
    if (searchKind) filters.kind = searchKind;
    const minMb = Number(searchMinMb);
    if (searchMinMb.trim() && Number.isFinite(minMb) && minMb >= 0) {
      filters.minSize = Math.floor(minMb * 1024 * 1024);
    }
    const maxMb = Number(searchMaxMb);
    if (searchMaxMb.trim() && Number.isFinite(maxMb) && maxMb >= 0) {
      filters.maxSize = Math.floor(maxMb * 1024 * 1024);
    }
    if (searchModified) {
      const seconds = Number(searchModified);
      if (Number.isFinite(seconds) && seconds > 0) {
        filters.modifiedAfter = Math.floor(Date.now() / 1000) - seconds;
      }
    }
    return filters;
  }

  function applySearchFilters(): void {
    submitSearch();
  }

  function resetSearchFilters(): void {
    searchKind = '';
    searchMinMb = '';
    searchMaxMb = '';
    searchModified = '';
  }

  let searchSortedResults = $derived.by(() => {
    const direction = searchSortOrder === 'asc' ? 1 : -1;
    const sorted = [...searchResults];
    sorted.sort((left, right) => {
      if (left.is_dir !== right.is_dir) return left.is_dir ? -1 : 1;
      let compared: number;
      if (searchSort === 'name') {
        compared = left.name.localeCompare(right.name, undefined, {
          numeric: true,
          sensitivity: 'base',
        });
      } else if (searchSort === 'size') {
        compared = left.size - right.size;
      } else {
        compared = left.modified - right.modified;
      }
      if (compared === 0) {
        compared = left.name.localeCompare(right.name, undefined, {
          numeric: true,
          sensitivity: 'base',
        });
      } else {
        compared *= direction;
      }
      return compared;
    });
    return sorted;
  });

  let searchSelectedRoot = $derived.by(() => {
    if (searchSelected.length === 0) return null;
    const selected = new Set(searchSelected);
    const roots = new Set(
      searchResults
        .filter((entry) => selected.has(searchEntryKey(entry)))
        .map((entry) => entry.rootId),
    );
    return roots.size === 1 ? Array.from(roots)[0] ?? null : undefined;
  });

  let searchSelectedReadOnly = $derived.by(() => {
    if (searchSelectedRoot === null || searchSelectedRoot === undefined) return true;
    return roots.find((root) => root.id === searchSelectedRoot)?.read_only ?? true;
  });

  function searchSelectedEntries(): SearchEntry[] {
    const selected = new Set(searchSelected);
    return searchSortedResults.filter((entry) => selected.has(searchEntryKey(entry)));
  }

  function toggleSearchSelection(entry: SearchEntry): void {
    const key = searchEntryKey(entry);
    searchSelected = searchSelected.includes(key)
      ? searchSelected.filter((value) => value !== key)
      : [...searchSelected, key];
  }

  function toggleSearchSelectAll(): void {
    if (
      searchSelected.length === searchSortedResults.length &&
      searchSortedResults.length > 0
    ) {
      searchSelected = [];
      return;
    }
    searchSelected = searchSortedResults.map(searchEntryKey);
  }

  async function deleteSearchSelected(): Promise<void> {
    const items = searchSelectedEntries();
    const rootId = searchSelectedRoot;
    if (items.length === 0 || rootId === null || rootId === undefined) return;
    const keys = new Set(searchSelected);
    try {
      const preview = await api.deletePreview(
        rootId,
        items.map((item) => item.path),
        csrf,
      );
      const totalSize = items.reduce((sum, item) => sum + (item.is_dir ? 0 : item.size), 0);
      const needsCode =
        preview.requires_code ??
        (items.some((item) => item.is_dir) ||
          items.length > 5 ||
          totalSize > 10 * 1024 * 1024);
      confirmState = {
        title: t($language, '永久删除'),
        message: t($language, '以下文件将被永久删除：'),
        items: items.map((item) => ({ name: item.name, isDir: item.is_dir })),
        requireText: needsCode ? preview.confirm_code : null,
        action: (value) => {
          void (async () => {
            startProgress(t($language, '删除文件'), items.length, t($language, '正在删除…'));
            try {
              await api.deletePaths(
                preview.token,
                undefined,
                csrf,
                needsCode ? value : undefined,
              );
              searchResults = searchResults.filter(
                (entry) => !keys.has(searchEntryKey(entry)),
              );
              searchSelected = [];
              updateProgress(items.length, t($language, '完成'));
              finishProgress();
              confirmState = null;
            } catch (error) {
              finishProgress();
              showNotice(error instanceof Error ? error.message : t($language, '删除失败'));
            }
          })();
        },
      };
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '删除失败'));
    }
  }

  function shareSearchSelected(): void {
    const items = searchSelectedEntries();
    const rootId = searchSelectedRoot;
    if (items.length === 0 || rootId === null || rootId === undefined) return;
    searchShare = {
      rootId,
      paths: items.map((item) => item.path),
      name:
        items.length === 1
          ? items[0]?.name ?? ''
          : t($language, '分享 {count} 个文件', { count: items.length }),
      isDir: items.length === 1 ? (items[0]?.is_dir ?? false) : false,
    };
  }

  function setSearchSort(field: 'name' | 'size' | 'modified'): void {
    if (searchSort === field) {
      searchSortOrder = searchSortOrder === 'asc' ? 'desc' : 'asc';
    } else {
      searchSort = field;
      searchSortOrder = field === 'name' ? 'asc' : 'desc';
    }
  }

  function searchEntryKey(entry: SearchEntry): string {
    return `${entry.rootId ?? 0}:${entry.path}`;
  }

  function mapSearchEntries(entries: SearchEntry[]): SearchEntry[] {
    return entries.map((entry) => ({
      ...entry,
      rootId: entry.root_id ?? entry.rootId ?? 0,
    }));
  }

  async function runSearch(query: string): Promise<void> {
    if (roots.length === 0) return;
    const token = ++searchToken;
    searchAbort?.abort();
    searchMoreAbort?.abort();
    searchMoreAbort = null;
    const controller = new AbortController();
    searchAbort = controller;
    searchLoading = true;
    searchLoadingMore = false;

    try {
      const scope = searchScope ? Number(searchScope) : null;
      const result =
        scope === null
          ? await api.searchAll(
              query,
              100,
              null,
              controller.signal,
              searchFilterParams(),
            )
          : await api.search(scope, query, 100, controller.signal);
      if (
        token !== searchToken ||
        controller.signal.aborted ||
        searchQuery.trim() !== query
      ) {
        return;
      }
      searchResults = mapSearchEntries(result.entries);
      searchNextCursor =
        'next_cursor' in result && typeof result.next_cursor === 'string'
          ? result.next_cursor
          : null;
      searchActive = true;
      searchLoading = false;
    } catch (error) {
      if (token === searchToken && !isAbortError(error)) {
        searchFailed = true;
        handleFilesError(error);
      }
    } finally {
      if (token === searchToken) searchLoading = false;
    }
  }

  async function loadMoreSearch(): Promise<void> {
    if (!searchActive || !searchNextCursor || searchLoadingMore || searchLoading) return;
    const query = searchQuery.trim();
    const cursor = searchNextCursor;
    const token = searchToken;
    const controller = new AbortController();
    searchMoreAbort?.abort();
    searchMoreAbort = controller;
    searchLoadingMore = true;
    try {
      const scope = searchScope ? Number(searchScope) : null;
      const result =
        scope === null
          ? await api.searchAll(
              query,
              100,
              cursor,
              controller.signal,
              searchFilterParams(),
            )
          : await api.search(scope, query, 100, controller.signal);
      if (token !== searchToken || searchQuery.trim() !== query) return;
      const seen = new SvelteSet(searchResults.map(searchEntryKey));
      const appended = mapSearchEntries(result.entries).filter((entry) => {
        const key = searchEntryKey(entry);
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
      });
      searchResults = [...searchResults, ...appended];
      searchNextCursor =
        'next_cursor' in result && typeof result.next_cursor === 'string'
          ? result.next_cursor
          : null;
    } catch (error) {
      if (!isAbortError(error)) handleFilesError(error);
    } finally {
      if (token === searchToken) searchLoadingMore = false;
    }
  }

  function openSearchMenu(event: MouseEvent, result: SearchEntry): void {
    event.preventDefault();
    event.stopPropagation();
    const button = event.currentTarget as HTMLElement | null;
    const rect = button?.getBoundingClientRect();
    searchMenu = {
      x: rect ? Math.max(8, rect.right - 180) : event.clientX,
      y: rect ? rect.bottom + 4 : event.clientY,
      result,
    };
  }

  function downloadSearchResult(result: SearchEntry): void {
    window.location.href = api.downloadUrl(result.rootId, result.path);
    searchMenu = null;
  }

  function openSearchResult(entry: SearchEntry): void {
    const root = entry.rootId ?? activeRootId;
    if (root === null) return;

    searchActive = false;
    searchLoading = false;
    searchLoadingMore = false;
    searchNextCursor = null;
    searchQuery = '';
    searchResults = [];

    activeRootId = root;
    if (entry.is_dir) {
      void loadList(root, entry.path);
    } else {
      const parent = entry.path.includes('/')
        ? entry.path.slice(0, entry.path.lastIndexOf('/'))
        : '';
      void (async () => {
        const located = await loadList(root, parent, entry.name);
        if (!located) {
          showToast(t($language, '文件可能已被移动或删除，已刷新目录'));
          void api.index(root, parent, false, csrf).catch((error) => {
            console.warn('failed to refresh directory index', error);
          });
          await loadList(root, parent);
          return;
        }
        selected = [entry.name];
        selectionAnchor = entry.name;
        await tick();
        await scrollEntryIntoView(entry.name);
      })();
    }
  }

  async function scrollEntryIntoView(name: string): Promise<void> {
    const index = entries.findIndex((entry) => entry.name === name);
    if (index < 0 || !contentElement) return;
    const find = (): HTMLElement | null =>
      contentElement?.querySelector<HTMLElement>(`[data-entry-name="${CSS.escape(name)}"]`) ?? null;
    let target = find();
    if (!target) {
      if (view === 'list') {
        contentElement.scrollTop = Math.max(
          0,
          index * VIRTUAL_ROW_HEIGHT - contentElement.clientHeight / 2 + VIRTUAL_ROW_HEIGHT / 2,
        );
      } else if (gridElement) {
        const metrics = galleryMetrics(gridElement);
        const row = Math.floor(index / metrics.columns);
        contentElement.scrollTop = Math.max(
          0,
          row * metrics.rowHeight - contentElement.clientHeight / 2,
        );
      }
      updateVirtualRange();
      await tick();
      target = find();
    }
    target?.scrollIntoView({ block: 'center', behavior: 'smooth' });
  }

  function exitSearch(): void {
    searchAbort?.abort();
    searchMoreAbort?.abort();
    searchMoreAbort = null;
    searchActive = false;
    searchLoading = false;
    searchLoadingMore = false;
    searchNextCursor = null;
    searchQuery = '';
    searchResults = [];
    searchFiltersOpen = false;
    resetSearchFilters();
  }

  function navigateToSegment(index: number): void {
    navigate(pathSegments.slice(0, index + 1).join('/'));
  }

  function joinPath(base: string, name: string): string {
    return base ? `${base}/${name}` : name;
  }

  let pathMeasureCanvas: HTMLCanvasElement | null = null;

  function measurePathText(text: string, element: HTMLElement): number {
    pathMeasureCanvas ??= document.createElement('canvas');
    const context = pathMeasureCanvas.getContext('2d');
    if (!context) return text.length * 8;
    const style = getComputedStyle(element);
    context.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
    return context.measureText(text).width;
  }

  function recomputePathCollapse(): void {
    const element = pathBarEl;
    if (!element || searchActive || pathSegments.length === 0) {
      collapsedCount = 0;
      return;
    }
    const available = element.clientWidth;
    if (available <= 16) return;
    const gap = 8;
    const separator = measurePathText('/', element) + gap * 2;
    const rootText = activeRoot ? rootLabel(activeRoot.alias) : t($language, '文件');
    const rootWidth = Math.min(measurePathText(rootText, element), 200);
    const widths = pathSegments.map((segment) =>
      Math.min(measurePathText(segment, element), 220),
    );
    let total = rootWidth;
    for (const width of widths) total += separator + width;
    if (total <= available) {
      collapsedCount = 0;
      return;
    }
    const moreWidth = measurePathText('…', element) + 24;
    const last = widths.length - 1;
    let used = rootWidth + separator + moreWidth + separator + (widths[last] ?? 0);
    let keepFrom = last;
    for (let index = last - 1; index >= 0; index -= 1) {
      const extra = separator + (widths[index] ?? 0);
      if (used + extra > available) break;
      used += extra;
      keepFrom = index;
    }
    collapsedCount = keepFrom;
  }

  function searchKindLabel(kind: string): string {
    const keys: Record<string, string> = {
      image: '图片',
      video: '视频',
      audio: '音频',
      text: '文本',
      pdf: 'PDF',
      dir: '目录',
      file: '文件',
    };
    const label = keys[kind];
    return label ? t($language, label) : kind;
  }

  function searchFilterTags(): string[] {
    const tags: string[] = [];
    if (searchKind) tags.push(searchKindLabel(searchKind));
    if (searchMinMb.trim()) tags.push(`> ${searchMinMb.trim()} MB`);
    if (searchMaxMb.trim()) tags.push(`< ${searchMaxMb.trim()} MB`);
    if (searchModified) {
      const keys: Record<string, string> = {
        '86400': '今天',
        '604800': '最近 7 天',
        '2592000': '最近 30 天',
        '31536000': '最近一年',
      };
      const label = keys[searchModified];
      if (label) tags.push(t($language, label));
    }
    return tags;
  }

  function searchStatusText(): string {
    if (searchLoading) return t($language, '搜索中…');
    if (searchTooShort) return t($language, '请输入至少 3 个字符（中文可输入 2 个）');
    if (searchFailed) return t($language, '搜索失败');
    if (searchResults.length === 0) return t($language, '没有结果');
    const loaded = t($language, '已加载 {count} 条', { count: searchResults.length });
    return searchNextCursor ? `${loaded} · ${t($language, '还有更多')}` : loaded;
  }

  $effect(() => {
    const element = pathBarEl;
    if (!element) return;
    const observer = new ResizeObserver(() => recomputePathCollapse());
    observer.observe(element);
    recomputePathCollapse();
    return () => observer.disconnect();
  });

  $effect(() => {
    void pathSegments.join('\u0000');
    void selected.length;
    void $language;
    void searchActive;
    requestAnimationFrame(() => recomputePathCollapse());
  });

  function rootLabel(alias: string): string {
    return rootAlias($language, alias);
  }

  function symlinkBlocked(entry: Entry): boolean {
    return entry.is_symlink && entry.symlink_state !== undefined && entry.symlink_state !== 'ok';
  }

  function symlinkTitle(entry: Entry): string {
    if (entry.symlink_state === 'denied') return t($language, '软链接目标无权限');
    if (entry.symlink_state === 'broken') return t($language, '软链接目标不存在');
    return entry.symlink_target
      ? `${t($language, '链接')} → ${entry.symlink_target}`
      : t($language, '链接');
  }

  function enterEntry(entry: Entry): void {
    void openEntry(entry);
  }

  async function openEntry(entry: Entry): Promise<void> {
    if (symlinkBlocked(entry)) {
      showToast(symlinkTitle(entry));
      return;
    }
    if (entry.is_dir) {
      navigate(joinPath(currentPath, entry.name));
      return;
    }
    const kind = classifyFile(entry.name, false, entry.mime_type);
    if (kind === 'text') {
      if (systemFeatures.editor && preferences?.editor_enabled !== false) await openEditor(entry);
      else showUnsupported(entry);
      return;
    }
    if (kind === 'audio') {
      startAudio(entry);
      return;
    }
    if (kind === 'pdf') {
      openPdf(entry);
      return;
    }
    if (kind === 'image' || kind === 'video') {
      void openPreview(entry);
      return;
    }
    if (kind === 'unknown' && activeRootId !== null) {
      if (!systemFeatures.editor || preferences?.editor_enabled === false) {
        showUnsupported(entry);
        return;
      }
      try {
        const probe = await api.probeFile(
          activeRootId,
          joinPath(currentPath, entry.name),
        );
        if (probe.is_text) await openEditor(entry);
        else showUnsupported(entry);
      } catch {
        showUnsupported(entry);
      }
      return;
    }
    showUnsupported(entry);
  }

  function showUnsupported(entry: Entry): void {
    showNoticeAction(
      t($language, '无法在浏览器中预览「{name}」。', { name: entry.name }),
      t($language, '无法预览'),
      t($language, '下载文件'),
      () => downloadEntry(entry),
      'info',
    );
  }

  function openPdf(entry: Entry): void {
    if (activeRootId === null) return;
    window.open(
      api.previewUrl(activeRootId, joinPath(currentPath, entry.name)),
      '_blank',
      'noopener,noreferrer',
    );
  }

  async function openProperties(rootId: number, path: string): Promise<void> {
    properties = { data: null, loading: true, error: '' };
    try {
      const data = await api.properties(rootId, path);
      properties = { data, loading: false, error: '' };
    } catch (error) {
      properties = {
        data: null,
        loading: false,
        error: error instanceof Error ? error.message : t($language, '操作失败'),
      };
    }
  }

  function startAudio(entry: Entry): void {
    if (activeRootId === null) return;
    const tracks: AudioTrack[] = entries
      .filter((item) => !item.is_dir && isAudioName(item.name))
      .map((item) => ({
        name: item.name,
        url: api.previewUrl(activeRootId ?? 0, joinPath(currentPath, item.name)),
        key: `${activeRootId}:${joinPath(currentPath, item.name)}`,
      }));
    const index = tracks.findIndex((track) => track.name === entry.name);
    if (index >= 0) playAudio(tracks, index);
  }

  function isAudioName(name: string): boolean {
    return classifyFile(name) === 'audio';
  }

  function isVideoName(name: string): boolean {
    return classifyFile(name) === 'video';
  }

  function handleEntryClick(event: MouseEvent, entry: Entry): void {
    if (suppressSelectionClear) {
      suppressSelectionClear = false;
      return;
    }
    if (event.shiftKey && selectionAnchor) {
      const anchor = entries.findIndex((item) => item.name === selectionAnchor);
      const current = entries.findIndex((item) => item.name === entry.name);
      if (anchor >= 0 && current >= 0) {
        const start = Math.min(anchor, current);
        const end = Math.max(anchor, current);
        selected = entries.slice(start, end + 1).map((item) => item.name);
      }
    } else if (event.metaKey || event.ctrlKey) {
      toggleSelection(entry.name);
      selectionAnchor = entry.name;
    } else if (selected.length === 1 && selected[0] === entry.name) {
      selected = [];
      selectionAnchor = null;
    } else {
      selected = [entry.name];
      selectionAnchor = entry.name;
    }
    if (window.matchMedia('(pointer: coarse)').matches) {
      enterEntry(entry);
    }
  }

  function openEntryMenu(event: MouseEvent, entry: Entry): void {
    event.preventDefault();
    event.stopPropagation();
    const button = event.currentTarget as HTMLElement | null;
    const rect = button?.getBoundingClientRect();
    if (!isSelected(entry.name)) selected = [entry.name];
    contextMenu = {
      x: rect ? rect.right : event.clientX,
      y: rect ? rect.bottom + 4 : event.clientY,
      entry,
    };
  }

  function menuPosition(x: number, y: number): { x: number; y: number } {
    return { x, y };
  }

  function clampMenu(
    element: HTMLElement | null,
    state: { x: number; y: number },
  ): { x: number; y: number; left: boolean; up: boolean } | null {
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    const x = Math.max(8, Math.min(state.x, window.innerWidth - rect.width - 8));
    const y = Math.max(8, Math.min(state.y, window.innerHeight - rect.height - 8));
    return {
      x,
      y,
      left: x + rect.width + 180 > window.innerWidth,
      up: y + rect.height > window.innerHeight - 80,
    };
  }

  $effect(() => {
    const state = contextMenu;
    if (!state) return;
    const clamped = clampMenu(contextMenuElement, state);
    if (!clamped) return;
    contextMenuLeft = clamped.left;
    contextMenuUp = clamped.up;
    if (clamped.x !== state.x || clamped.y !== state.y) {
      contextMenu = { ...state, x: clamped.x, y: clamped.y };
    }
  });

  $effect(() => {
    if (!contextMenu || !contextMenuElement) return;
    const first = contextMenuElement.querySelector<HTMLElement>(
      '[role="menuitem"]:not([disabled])',
    );
    first?.focus();
  });

  function menuKeydown(event: KeyboardEvent): void {
    const menu = event.currentTarget as HTMLElement;
    const items = Array.from(
      menu.querySelectorAll<HTMLElement>('[role="menuitem"]:not([disabled])'),
    );
    if (items.length === 0) return;
    const index = items.findIndex((item) => item === document.activeElement);
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      items[(index + 1 + items.length) % items.length]?.focus();
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      items[(index - 1 + items.length) % items.length]?.focus();
    } else if (event.key === 'Home') {
      event.preventDefault();
      items.at(0)?.focus();
    } else if (event.key === 'End') {
      event.preventDefault();
      items.at(-1)?.focus();
    } else if (event.key === 'ArrowRight') {
      const submenu = items.at(index)?.parentElement?.querySelector('.submenu');
      const target = submenu?.querySelector<HTMLElement>('button');
      if (target) {
        event.preventDefault();
        target.focus();
      }
    } else if (event.key === 'ArrowLeft') {
      const current = document.activeElement as HTMLElement | null;
      const submenu = current?.closest('.submenu');
      if (submenu) {
        event.preventDefault();
        (submenu.parentElement?.querySelector(':scope > button') as HTMLElement | null)?.focus();
      }
    } else if (event.key === 'Escape') {
      event.preventDefault();
      contextMenu = null;
      blankMenu = null;
    }
  }

  $effect(() => {
    const state = blankMenu;
    if (!state) return;
    const clamped = clampMenu(blankMenuElement, state);
    if (!clamped) return;
    blankMenuLeft = clamped.left;
    blankMenuUp = clamped.up;
    if (clamped.x !== state.x || clamped.y !== state.y) {
      blankMenu = { ...state, x: clamped.x, y: clamped.y };
    }
  });

  function openContextMenu(event: MouseEvent, entry: Entry): void {
    event.preventDefault();
    event.stopPropagation();
    blankMenu = null;
    if (!isSelected(entry.name)) selected = [entry.name];
    contextMenu = { ...menuPosition(event.clientX, event.clientY), entry };
  }

  async function openPreview(entry: Entry): Promise<void> {
    if (activeRootId === null) return;
    if (isRasterImageName(entry.name)) {
      try {
        const info = await api.imageInfo(activeRootId, joinPath(currentPath, entry.name));
        if (!info.previewable) {
          const size =
            info.width && info.height
              ? `${info.width}×${info.height}`
              : t($language, '未知尺寸');
          const message =
            info.reason === 'max_source'
              ? t($language, '文件过大，无法预览')
              : info.reason === 'unsupported'
                ? t($language, '无法识别图片格式')
                : t($language, '图片过大 {size}，无法预览', { size });
          showNoticeAction(
            message,
            t($language, '无法预览'),
            t($language, '下载文件'),
            () => downloadEntry(entry),
            'info',
          );
          return;
        }
      } catch (error) {
        handleFilesError(error);
        return;
      }
    }
    const items = entries.filter(
      (item) => !item.is_dir && (isImageName(item.name) || isVideoName(item.name)),
    );
    const index = items.findIndex((item) => item.name === entry.name);
    if (index >= 0) {
      preview = { items, index };
    }
  }

  async function openPreviewMany(items: Entry[]): Promise<void> {
    if (activeRootId === null) return;
    const allowed: Entry[] = [];
    let skipped = 0;
    for (const item of items) {
      if (isRasterImageName(item.name)) {
        try {
          const info = await api.imageInfo(activeRootId, joinPath(currentPath, item.name));
          if (!info.previewable) {
            skipped += 1;
            continue;
          }
        } catch {
          skipped += 1;
          continue;
        }
      }
      allowed.push(item);
    }
    if (skipped > 0) {
      showToast(t($language, '已跳过 {count} 张过大的图片', { count: skipped }));
    }
    if (allowed.length > 0) preview = { items: allowed, index: 0 };
  }

  function previewEntry(entry: Entry): void {
    void openEntry(entry);
  }

  function runContextAction(
    action:
      | 'preview'
      | 'download'
      | 'share'
      | 'rename'
      | 'copy-to'
      | 'move-to'
      | 'archive-zip'
      | 'extract'
      | 'delete'
      | 'play'
      | 'properties'
      | 'edit-source',
  ): void {
    const entry = contextMenu?.entry;
    contextMenu = null;
    if (!entry) return;
    if ((action === 'preview' || action === 'download') && symlinkBlocked(entry)) {
      showToast(symlinkTitle(entry));
      return;
    }
    const targets =
      selected.includes(entry.name) && selected.length > 1
        ? entries.filter((item) => selected.includes(item.name))
        : [entry];
    if (action === 'preview') {
      if (targets.length > 1) {
        const media = targets.filter(
          (item) => !item.is_dir && (isImageName(item.name) || isVideoName(item.name)),
        );
        if (media.length > 0) void openPreviewMany(media);
      } else {
        previewEntry(entry);
      }
    }
    if (action === 'download') {
      if (targets.length > 1) void downloadZipEntries(targets);
      else downloadEntry(entry);
    }
    if (action === 'edit-source' && isSvgName(entry.name)) {
      void openEditor(entry);
    }
    if (action === 'share') {
      if (targets.length > 1 && targets.some((item) => item.is_dir)) {
        showToast(t($language, '目录请单独分享'));
        return;
      }
      shareTargets = targets;
    }
    if (action === 'rename' && targets.length === 1) {
      nameDialog = {
        mode: 'rename',
        entry,
        title: t($language, '重命名'),
        label: t($language, '新名称'),
        initialValue: entry.name,
      };
    }
    if (action === 'archive-zip') {
      archiveTargets = targets;
      nameDialog = {
        mode: 'archive',
        entry: null,
        title: t($language, '打包为ZIP'),
        label: t($language, '压缩包名称'),
        initialValue: 'archive.zip',
      };
    }
    if (action === 'extract' && targets.length === 1) {
      void extractEntry(entry);
    }
    if (action === 'copy-to') transfer = { mode: 'copy', entries: targets };
    if (action === 'move-to') transfer = { mode: 'move', entries: targets };
    if (action === 'delete') void requestDelete(targets);
    if (action === 'play' && activeRootId !== null) {
      startAudio(entry);
    }
    if (action === 'properties' && activeRootId !== null) {
      void openProperties(activeRootId, joinPath(currentPath, entry.name));
    }
  }

  function openMkdir(): void {
    nameDialog = {
      mode: 'mkdir',
      entry: null,
      title: t($language, '新建目录'),
      label: t($language, '目录名称'),
      initialValue: '',
    };
  }

  async function submitName(value: string): Promise<void> {
    const dialog = nameDialog;
    nameDialog = null;
    if (!dialog || activeRootId === null) return;
    const rootId = activeRootId;
    try {
      if (dialog.mode === 'mkdir') {
        await api.mkdir(rootId, currentPath, value, csrf);
      } else if (dialog.mode === 'touch') {
        await api.touch(rootId, currentPath, value, csrf);
      } else if (dialog.mode === 'archive') {
        const paths = archiveTargets.map((entry) => entry.name);
        startProgress(t($language, '打包为ZIP'), paths.length, t($language, '正在打包…'));
        try {
          const job = await api.archiveZip(rootId, currentPath, paths, value, csrf);
          const result = await waitForTransferJob(job.job_id);
          const name =
            result && typeof (result as { name?: unknown }).name === 'string'
              ? String((result as { name?: unknown }).name)
              : job.name;
          await loadList(rootId, currentPath);
          selected = name ? [name] : [];
          finishProgress();
          showNotice(t($language, '已创建 {name}', { name }), 'info', t($language, '打包完成'));
        } catch (error) {
          finishProgress();
          throw error;
        }
        return;
      } else if (dialog.entry) {
        await api.rename(rootId, joinPath(currentPath, dialog.entry.name), value, csrf);
      }
      clearDirSizeCache();
      await loadList(rootId, currentPath);
    } catch (error) {
      handleFilesError(error);
    }
  }

  function isDragSource(entry: Entry): boolean {
    return (
      dragPayload?.rootId === activeRootId &&
      dragPayload.paths.includes(joinPath(currentPath, entry.name))
    );
  }

  function startDrag(event: DragEvent, entry: Entry): void {
    if (activeRootId === null) return;
    const chosen =
      selected.includes(entry.name) && selected.length > 1
        ? entries.filter((item) => selected.includes(item.name))
        : [entry];
    const current = chosen.length > 0 ? chosen : [entry];
    const paths = current.map((item) => joinPath(currentPath, item.name));
    dragPayload = {
      rootId: activeRootId,
      paths,
      names: current.map((item) => item.name),
      dirs: current
        .filter((item) => item.is_dir)
        .map((item) => joinPath(currentPath, item.name)),
    };
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('text/plain', paths.join('\n'));
    }
  }

  function entryDragKey(entry: Entry): string {
    return activeRootId === null ? '' : `${activeRootId}:${joinPath(currentPath, entry.name)}`;
  }

  function handleDragOver(event: DragEvent, entry: Entry): void {
    if (event.dataTransfer?.types.includes('Files')) {
      handleFileDragOver(event, entry);
      return;
    }
    if (!entry.is_dir || !dragPayload || activeRootId === null) return;
    event.preventDefault();
    event.stopPropagation();
    const destPath = joinPath(currentPath, entry.name);
    const key = `${activeRootId}:${destPath}`;
    const sameRoot = dragPayload.rootId === activeRootId;
    const invalid =
      currentReadOnly ||
      (sameRoot &&
        dragPayload.paths.some(
          (path) => destPath === path || destPath.startsWith(`${path}/`),
        ));
    dragOverKey = key;
    dragOverValid = !invalid;
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = invalid ? 'none' : 'move';
    }
  }

  function handleDragLeave(event: DragEvent, entry: Entry): void {
    const target = event.currentTarget as HTMLElement;
    const related = event.relatedTarget as Node | null;
    if (related && target.contains(related)) return;
    if (dragOverKey === entryDragKey(entry)) {
      dragOverKey = null;
      dragOverValid = true;
    }
  }

  function clearDrag(): void {
    dragPayload = null;
    dragOverKey = null;
    dragOverValid = true;
  }

  async function waitForTransferJob(jobId: string): Promise<TransferJobResult | null> {
    setProgressCancel(() => {
      void api.cancelJob(jobId, csrf).catch((error) => {
        console.warn('cancel job failed', error);
      });
    });
    try {
      for (;;) {
        const job = await api.job(jobId);
        const detail =
          job.bytes_total > 0
            ? `${formatSize(job.bytes_done)} / ${formatSize(job.bytes_total)}`
            : job.status === 'running'
              ? t($language, '处理中…')
              : t($language, '排队中…');
        const percent =
          job.bytes_total > 0
            ? Math.min(100, Math.round((job.bytes_done / job.bytes_total) * 100))
            : job.total > 0
              ? Math.min(100, Math.round((job.progress / job.total) * 100))
              : 0;
        updateProgress(percent, detail, 100);
        if (job.status === 'completed') {
          const result = (job.result as TransferJobResult | null) ?? null;
          if (result?.failed) {
            showNotice(
              t($language, '有 {count} 项操作失败。', { count: result.failed }),
              'warning',
              t($language, '部分失败'),
            );
          }
          return result;
        }
        if (job.status === 'failed') {
          throw new Error(job.error ?? t($language, '操作失败'));
        }
        if (job.status === 'cancelled') {
          throw new Error(t($language, '操作已取消'));
        }
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
    } finally {
      setProgressCancel(null);
    }
  }

  async function dropInto(event: DragEvent, destRoot: number, destPath: string): Promise<void> {
    event.preventDefault();
    event.stopPropagation();
    dragOverKey = null;
    dragOverValid = true;
    if (destRoot === activeRootId && destPath === currentPath && currentReadOnly) {
      showToast(readOnlyMessage());
      return;
    }
    if (event.dataTransfer?.types.includes('Files')) {
      const candidates = await collectDroppedFiles(event.dataTransfer);
      if (candidates.length > 0) enqueueUploads(candidates, destPath);
      return;
    }
    const payload = dragPayload;
    clearDrag();
    if (!payload) return;
    if (payload.rootId === destRoot && payload.paths.includes(destPath)) return;
    if (
      payload.rootId === destRoot &&
      payload.dirs.some((path) => destPath.startsWith(`${path}/`))
    ) {
      showNotice(t($language, '不能把目录移动到它自己里面。'), 'warning', t($language, '移动失败'));
      return;
    }
    startProgress(
      t($language, '移动文件'),
      payload.paths.length,
      payload.paths.length === 1
        ? (payload.names[0] ?? '')
        : t($language, '已选 {count} 项', { count: payload.paths.length }),
    );
    try {
      const job = await api.transferJob(
        'move',
        payload.rootId,
        payload.paths,
        destRoot,
        destPath,
        csrf,
      );
      await waitForTransferJob(job.job_id);
      updateProgress(100, t($language, '完成'), 100);
      finishProgress();
      showToast(
        payload.paths.length === 1
          ? t($language, '已移动到 {name}', { name: destPath.split('/').pop() ?? destPath })
          : t($language, '已移动 {count} 项', { count: payload.paths.length }),
      );
      clearDirSizeCache();
      if (activeRootId !== null) await loadList(activeRootId, currentPath);
    } catch (error) {
      failProgress(error instanceof Error ? error.message : t($language, '移动失败'));
      handleFilesError(error);
    }
  }

  async function extractEntry(entry: Entry): Promise<void> {
    if (activeRootId === null) return;
    const rootId = activeRootId;
    const path = joinPath(currentPath, entry.name);
    startProgress(t($language, '解压'), 1, entry.name);
    try {
      const job = await api.archiveUnzip(rootId, path, undefined, csrf);
      const result = job.job_id ? await waitForTransferJob(job.job_id) : null;
      finishProgress();
      await loadList(rootId, currentPath);
      const name =
        result && typeof (result as { name?: unknown }).name === 'string'
          ? String((result as { name?: unknown }).name)
          : (job.name.split('/').pop() ?? job.name);
      selected = name ? [name] : [];
      showToast(t($language, '解压完成'));
    } catch (error) {
      failProgress(error instanceof Error ? error.message : t($language, '解压失败'));
      handleFilesError(error);
    }
  }

  async function submitTransfer(destRoot: number, destPath: string): Promise<void> {
    const pending = transfer;
    transfer = null;
    if (!pending || activeRootId === null) return;
    const rootId = activeRootId;
    const sourcePath = currentPath;
    const startedAt = Date.now();
    const totalBytes = pending.entries.reduce((sum, entry) => sum + entry.size, 0);
    startProgress(
      pending.mode === 'copy' ? t($language, '复制文件') : t($language, '移动文件'),
      pending.entries.length,
    );
    try {
      const job = await api.transferJob(
        pending.mode,
        rootId,
        pending.entries.map((entry) => joinPath(sourcePath, entry.name)),
        destRoot,
        destPath,
        csrf,
      );
      const result = await waitForTransferJob(job.job_id);
      updateProgress(100, t($language, '完成'), 100);
      const viewingSource = activeRootId === rootId && currentPath === sourcePath;
      const viewingDestination = activeRootId === destRoot && currentPath === destPath;
      const left = !viewingSource && !viewingDestination;
      if (shouldNotifyLong(startedAt, totalBytes, pending.entries.length, left)) {
        const kind = pending.mode === 'copy' ? t($language, '复制') : t($language, '移动');
        const failed = result?.failed ?? 0;
        const message =
          failed > 0
            ? t($language, '{kind}完成：{success} 项成功，{failed} 项失败', {
                kind,
                success: pending.entries.length - failed,
                failed,
              })
            : t($language, '{kind}完成：{count} 项 · {size} → {dest}', {
                kind,
                count: pending.entries.length,
                size: formatSize(totalBytes),
                dest: destPath || t($language, '根目录'),
              });
        notifyLongCompletion(message, left);
      }
      if (viewingSource) {
        clearDirSizeCache();
        await loadList(rootId, currentPath);
      } else if (viewingDestination) {
        clearDirSizeCache();
        await loadList(destRoot, currentPath);
      }
      finishProgress();
    } catch (error) {
      failProgress(error instanceof Error ? error.message : t($language, '操作失败'));
      handleFilesError(error);
    }
  }

  function isTypingTarget(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLElement &&
      (target.tagName === 'INPUT' ||
        target.tagName === 'TEXTAREA' ||
        target.tagName === 'SELECT' ||
        target.isContentEditable)
    );
  }

  function isMarkdown(name: string): boolean {
    return /\.(md|markdown)$/i.test(name);
  }

  function downloadEntry(entry: Entry): void {
    if (activeRootId === null) return;
    window.location.href = api.downloadUrl(activeRootId, joinPath(currentPath, entry.name));
  }

  function isSelected(name: string): boolean {
    return selected.includes(name);
  }

  function toggleSelection(name: string): void {
    selected = isSelected(name) ? selected.filter((item) => item !== name) : [...selected, name];
  }

  function clearSelection(): void {
    selected = [];
    selectionAnchor = null;
  }

  function handleContentClick(event: MouseEvent): void {
    if (appView !== 'files') return;
    if (suppressSelectionClear) {
      suppressSelectionClear = false;
      return;
    }
    const target = event.target as HTMLElement;
    if (target.closest('[data-entry-name]') || target.closest('button,input,a')) return;
    selected = [];
    contextMenu = null;
    blankMenu = null;
  }

  function handleBlankContextMenu(event: MouseEvent): void {
    if (appView !== 'files') return;
    const target = event.target as HTMLElement;
    if (
      target.closest('[data-entry-name]') ||
      target.closest('button,input,a') ||
      target.closest('.context-menu')
    ) {
      return;
    }
    event.preventDefault();
    contextMenu = null;
    blankMenu = menuPosition(event.clientX, event.clientY);
  }

  function startMarquee(event: MouseEvent): void {
    if (appView !== 'files') return;
    suppressSelectionClear = false;
    if (event.button !== 0 || isTypingTarget(event.target)) return;
    const target = event.target as HTMLElement;
    if (target.closest('[data-entry-name]') || target.closest('button,input,a')) return;
    marquee = { x1: event.clientX, y1: event.clientY, x2: event.clientX, y2: event.clientY };
  }

  function marqueeEntryNames(left: number, right: number, top: number, bottom: number): string[] {
    if (view === 'list' && entries.length > VIRTUAL_THRESHOLD) {
      const table = fileTable;
      if (!table) return [];
      const rect = table.getBoundingClientRect();
      if (rect.left >= right || rect.right <= left || rect.bottom <= top || rect.top >= bottom) {
        return [];
      }
      const headerHeight = table.tHead?.getBoundingClientRect().height ?? 0;
      const startY = rect.top + headerHeight;
      const first = Math.max(0, Math.floor((top - startY) / VIRTUAL_ROW_HEIGHT));
      const last = Math.min(entries.length - 1, Math.floor((bottom - startY) / VIRTUAL_ROW_HEIGHT));
      if (last < first) return [];
      return entries.slice(first, last + 1).map((entry) => entry.name);
    }
    if (view !== 'list' && entries.length > VIRTUAL_THRESHOLD && gridElement) {
      const rect = gridElement.getBoundingClientRect();
      if (rect.left >= right || rect.right <= left || rect.bottom <= top || rect.top >= bottom) {
        return [];
      }
      const metrics = galleryMetrics(gridElement);
      const totalRows = Math.ceil(entries.length / metrics.columns);
      const startRow = Math.max(0, Math.floor(virtualStart / metrics.columns));
      const startY = rect.top + virtualPadTop;
      const firstRow = Math.max(0, Math.floor((top - startY) / metrics.rowHeight) + startRow);
      const lastRow = Math.min(
        totalRows - 1,
        Math.floor((bottom - startY) / metrics.rowHeight) + startRow,
      );
      if (lastRow < firstRow) return [];
      const columnWidth = (rect.width - metrics.gap * (metrics.columns - 1)) / metrics.columns;
      const firstColumn = Math.max(0, Math.floor((left - rect.left) / (columnWidth + metrics.gap)));
      const lastColumn = Math.min(
        metrics.columns - 1,
        Math.floor((right - rect.left) / (columnWidth + metrics.gap)),
      );
      const names: string[] = [];
      for (let row = firstRow; row <= lastRow; row += 1) {
        for (let column = firstColumn; column <= lastColumn; column += 1) {
          const entry = entries[row * metrics.columns + column];
          if (entry) names.push(entry.name);
        }
      }
      return names;
    }
    const names: string[] = [];
    document.querySelectorAll<HTMLElement>('[data-entry-name]').forEach((element) => {
      const rect = element.getBoundingClientRect();
      const intersects =
        rect.left < right && rect.right > left && rect.top < bottom && rect.bottom > top;
      const name = element.dataset.entryName;
      if (intersects && name) names.push(name);
    });
    return names;
  }

  function updateMarquee(event: MouseEvent): void {
    if (appView !== 'files' || !marquee) return;
    const next = { ...marquee, x2: event.clientX, y2: event.clientY };
    marquee = next;
    marqueePreview = marqueeEntryNames(
      Math.min(next.x1, next.x2),
      Math.max(next.x1, next.x2),
      Math.min(next.y1, next.y2),
      Math.max(next.y1, next.y2),
    );
  }

  function finishMarquee(): void {
    if (appView !== 'files' || !marquee) return;
    const left = Math.min(marquee.x1, marquee.x2);
    const right = Math.max(marquee.x1, marquee.x2);
    const top = Math.min(marquee.y1, marquee.y2);
    const bottom = Math.max(marquee.y1, marquee.y2);
    const names = marqueeEntryNames(left, right, top, bottom);

    if (names.length > 0) {
      selected = Array.from(new Set([...selected, ...names]));
    }
    if (Math.abs(right - left) > 2 || Math.abs(bottom - top) > 2) {
      suppressSelectionClear = true;
    }
    marquee = null;
    marqueePreview = [];
  }

  function uploadClick(): void {
    if (!activeRoot) return;
    if (currentReadOnly) {
      showToast(readOnlyMessage());
      return;
    }
    if (!remoteCan('uploads')) {
      showToast(t($language, '对方不支持这个操作'));
      return;
    }
    fileInput?.click();
  }

  async function handleFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    const queued = () =>
      enqueueUploads(files.map((file) => ({ file, relativePath: file.name })));
    const existing = new Set(
      entries.filter((entry) => !entry.is_dir).map((entry) => entry.name),
    );
    const conflicts = files.filter((file) => existing.has(file.name));
    if (conflicts.length === 0) {
      queued();
      return;
    }
    confirmState = {
      title: t($language, '文件已存在'),
      message: t($language, '当前目录已有同名文件。上传会覆盖它们，继续吗？'),
      requireText: null,
      action: () => {
        confirmState = null;
        queued();
      },
    };
  }

  async function handleFolderFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    enqueueUploads(
      files.map((file) => ({
        file,
        relativePath: file.webkitRelativePath || file.name,
      })),
    );
  }

  function folderUploadClick(): void {
    if (!activeRoot) return;
    if (currentReadOnly) {
      showToast(readOnlyMessage());
      return;
    }
    folderInput?.click();
  }

  function enqueueUploads(
    candidates: { file: File; relativePath: string }[],
    basePath = currentPath,
  ): void {
    if (activeRootId === null || candidates.length === 0) return;
    if (currentReadOnly) {
      showToast(readOnlyMessage());
      return;
    }
    filesError = '';
    if (uploadBatchCount === 0) {
      uploadBatchStarted = Date.now();
      uploadBatchBytes = 0;
      uploadBatchCount = 0;
      uploadBatchRoot = activeRootId;
      uploadBatchPath = basePath;
    }
    uploadBatchCount += candidates.length;
    uploadBatchBytes += candidates.reduce((sum, candidate) => sum + candidate.file.size, 0);
    uploadManager.enqueue(candidates, activeRootId, basePath);
  }

  async function collectDroppedFiles(
    dataTransfer: DataTransfer,
  ): Promise<{ file: File; relativePath: string }[]> {
    const entries: FileSystemEntry[] = [];
    for (const item of Array.from(dataTransfer.items ?? [])) {
      const entry = item.webkitGetAsEntry?.() ?? null;
      if (entry) entries.push(entry);
    }
    if (entries.length === 0) {
      return Array.from(dataTransfer.files ?? []).map((file) => ({
        file,
        relativePath: file.name,
      }));
    }
    const readDirectory = (reader: FileSystemDirectoryReader): Promise<FileSystemEntry[]> =>
      new Promise((resolve, reject) => reader.readEntries(resolve, reject));
    const walk = async (
      entry: FileSystemEntry,
      prefix: string,
    ): Promise<{ file: File; relativePath: string }[]> => {
      if (entry.isFile) {
        const file = await new Promise<File>((resolve, reject) =>
          (entry as FileSystemFileEntry).file(resolve, reject),
        );
        return [{ file, relativePath: `${prefix}${file.name}` }];
      }
      if (!entry.isDirectory) return [];
      const reader = (entry as FileSystemDirectoryEntry).createReader();
      const children: FileSystemEntry[] = [];
      for (;;) {
        const batch = await readDirectory(reader);
        if (batch.length === 0) break;
        children.push(...batch);
      }
      const nested = await Promise.all(
        children.map((child) => walk(child, `${prefix}${entry.name}/`)),
      );
      return nested.flat();
    };
    const collected = await Promise.all(entries.map((entry) => walk(entry, '')));
    return collected.flat();
  }

  async function handleAppDrop(event: DragEvent): Promise<void> {
    event.preventDefault();
    if (!event.dataTransfer) return;
    if (currentReadOnly) {
      showToast(readOnlyMessage());
      return;
    }
    const candidates = await collectDroppedFiles(event.dataTransfer);
    enqueueUploads(candidates);
  }

  function handleFileDragOver(event: DragEvent, entry: Entry): void {
    if (!entry.is_dir || !event.dataTransfer?.types.includes('Files')) return;
    event.preventDefault();
    event.stopPropagation();
    dragOverKey = entryDragKey(entry);
    dragOverValid = true;
    event.dataTransfer.dropEffect = currentReadOnly ? 'none' : 'copy';
  }

  async function deleteEntry(entry: Entry): Promise<void> {
    await requestDelete([entry]);
  }

  async function requestDelete(items: Entry[]): Promise<void> {
    if (activeRootId === null || !activeRoot || items.length === 0) {
      return;
    }
    if (currentReadOnly) {
      showToast(readOnlyMessage());
      return;
    }

    const rootId = activeRootId;
    const path = currentPath;
    const deletingPaths = items.map((entry) => joinPath(path, entry.name));
    try {
      const paths = deletingPaths;
      const result = await api.deletePreview(rootId, paths, csrf);
      const totalSize = items.reduce((sum, entry) => sum + entry.size, 0);
      const needsCode =
        items.some((entry) => entry.is_dir) || items.length > 5 || totalSize > 10 * 1024 * 1024;
      confirmState = {
        title: t($language, '永久删除'),
        message: t($language, '以下文件将被永久删除：'),
        items: items.map((entry) => ({ name: entry.name, isDir: entry.is_dir })),
        requireText: needsCode ? result.confirm_code : null,
        action: (value) => {
          void (async () => {
            const startedAt = Date.now();
            startProgress(t($language, '删除文件'), items.length, t($language, '正在删除…'));
            try {
              await api.deletePaths(result.token, undefined, csrf, needsCode ? value : undefined);
              updateProgress(items.length, t($language, '完成'));
              finishProgress();
              confirmState = null;
              const stillHere = activeRootId === rootId && currentPath === path;
              const insideDeleted = deletingPaths.some(
                (deleted) =>
                  currentPath === deleted || currentPath.startsWith(`${deleted}/`),
              );
              if (shouldNotifyLong(startedAt, totalSize, items.length, !stillHere)) {
                notifyLongCompletion(
                  t($language, '删除完成：{count} 项 · {size}', {
                    count: items.length,
                    size: formatSize(totalSize),
                  }),
                  !stillHere,
                );
              }
              if (stillHere || insideDeleted) {
                clearDirSizeCache();
                await loadList(rootId, path);
              }
            } catch (error) {
              failProgress(error instanceof Error ? error.message : t($language, '删除失败'));
              handleFilesError(error);
              confirmState = null;
            }
          })();
        },
      };
    } catch (error) {
      handleFilesError(error);
    }
  }

  async function deleteSelected(): Promise<void> {
    await requestDelete(entries.filter((entry) => selected.includes(entry.name)));
  }

  function setSort(field: 'name' | 'size' | 'modified'): void {
    if (sortField === field) {
      sortOrder = sortOrder === 'asc' ? 'desc' : 'asc';
    } else {
      sortField = field;
      sortOrder = field === 'name' ? 'asc' : 'desc';
    }
    if (activeRootId !== null) {
      localStorage.setItem(`sort:${activeRootId}:${currentPath}`, `${sortField}:${sortOrder}`);
      localStorage.setItem('filehub.sort', `${sortField}:${sortOrder}`);
      void loadList(activeRootId, currentPath);
    }
  }

  function setView(next: 'list' | 'thumb' | 'tile'): void {
    view = next;
    try {
      localStorage.setItem('filehub.view', next);
      if (activeRootId !== null) {
        localStorage.setItem(`view:${activeRootId}:${currentPath}`, next);
      }
    } catch (error) {
      console.warn('could not persist view preference', error);
    }
  }

  function thumbKey(root: number, path: string): string {
    return `${root}:${path}`;
  }

  function handleThumbnailEvent(type: string, data: Record<string, unknown>): void {
    const root = Number(data.root);
    const path = typeof data.path === 'string' ? data.path : '';
    if (!Number.isFinite(root) || root <= 0 || !path) return;
    const key = thumbKey(root, path);
    if (type === 'thumbnail.failed') {
      thumbFailures = { ...thumbFailures, [key]: true };
      return;
    }
    thumbVersions = { ...thumbVersions, [key]: (thumbVersions[key] ?? 0) + 1 };
    if (thumbFailures[key]) {
      const next = { ...thumbFailures };
      delete next[key];
      thumbFailures = next;
    }
  }

  function handlePresenceEvent(data: Record<string, unknown>): void {
    const client = typeof data.client === 'string' ? data.client : '';
    if (client === presenceClient) return;
    const root = Number(data.root);
    const path = typeof data.path === 'string' ? data.path : '';
    if (!Number.isFinite(root) || root <= 0 || !path) return;
    const key = thumbKey(root, path);
    const state = typeof data.state === 'string' ? data.state : '';
    if (state === 'open') {
      const seconds = Number(data.expires_at);
      const expiresAt = Number.isFinite(seconds) ? seconds * 1000 : Date.now() + 45_000;
      const peers = remoteEditors[key] ?? {};
      remoteEditors = { ...remoteEditors, [key]: { ...peers, [client]: expiresAt } };
      return;
    }
    if (state === 'close' && remoteEditors[key]) {
      const peers = { ...remoteEditors[key] };
      delete peers[client];
      const next = { ...remoteEditors };
      if (Object.keys(peers).length === 0) delete next[key];
      else next[key] = peers;
      remoteEditors = next;
    }
  }

  function seedPresencePeers(
    key: string,
    peers: Array<{ client: string; expires_at: number }>,
  ): void {
    if (peers.length === 0) return;
    const existing = remoteEditors[key] ?? {};
    const merged = { ...existing };
    for (const peer of peers) {
      if (peer.client !== presenceClient) merged[peer.client] = peer.expires_at * 1000;
    }
    if (Object.keys(merged).length > 0) remoteEditors = { ...remoteEditors, [key]: merged };
  }

  function pruneRemotePresence(): void {
    const now = Date.now();
    const next: Record<string, Record<string, number>> = {};
    for (const [key, peers] of Object.entries(remoteEditors)) {
      const alive = Object.fromEntries(
        Object.entries(peers).filter(([, expiresAt]) => expiresAt > now),
      );
      if (Object.keys(alive).length > 0) next[key] = alive;
    }
    if (JSON.stringify(next) !== JSON.stringify(remoteEditors)) {
      remoteEditors = next;
    }
  }

  async function sendPresence(
    state: 'open' | 'close',
    root: number,
    path: string,
    keepalive = false,
  ): Promise<Awaited<ReturnType<typeof api.editPresence>> | null> {
    if (!csrf) return null;
    try {
      return await api.editPresence(root, path, state, presenceClient, csrf, keepalive);
    } catch (error) {
      console.warn('edit presence update failed', error);
      return null;
    }
  }

  function stopPresenceHeartbeat(): void {
    if (presenceTimer) clearInterval(presenceTimer);
    presenceTimer = null;
  }

  function beginEditPresence(root: number, path: string): void {
    endEditPresence();
    presenceRoot = root;
    presencePath = path;
    const key = thumbKey(root, path);
    void sendPresence('open', root, path).then((reply) => {
      if (reply) seedPresencePeers(key, reply.peers);
    });
    presenceTimer = setInterval(() => {
      if (presenceRoot !== null) void sendPresence('open', presenceRoot, presencePath);
    }, 20_000);
  }

  function endEditPresence(keepalive = false): void {
    if (presenceRoot === null) {
      stopPresenceHeartbeat();
      return;
    }
    const root = presenceRoot;
    const path = presencePath;
    presenceRoot = null;
    presencePath = '';
    stopPresenceHeartbeat();
    void sendPresence('close', root, path, keepalive);
  }

  function closeEditor(force = false): void {
    if (editorSaving && !force) return;
    if (!force && editorDirty && !editorReadOnly) {
      confirmState = {
        title: t($language, '关闭编辑器'),
        message: t($language, '有未保存修改，确定关闭？'),
        requireText: null,
        action: () => {
          confirmState = null;
          closeEditor(true);
        },
      };
      return;
    }
    endEditPresence();
    if (previewTimer) {
      clearTimeout(previewTimer);
      previewTimer = null;
    }
    editorPreviewSource = '';
    editorPreviewStale = false;
    editorOpen = false;
  }

  async function openEditor(entry: Entry): Promise<void> {
    if (activeRootId === null) return;
    const path = joinPath(currentPath, entry.name);
    try {
      const probe = await api.probeFile(activeRootId, path);
      if (!probe.is_text) {
        const tooLarge = probe.reason === 'too_large';
        showNoticeAction(
          tooLarge
            ? t($language, '文件超过 2MB，无法在编辑器中打开')
            : t($language, '不是 UTF-8 文本，无法编辑'),
          t($language, '无法编辑'),
          t($language, '下载文件'),
          () => downloadEntry(entry),
          'info',
        );
        return;
      }
      const file = await api.getFile(activeRootId, path);
      editorName = entry.name;
      editorPath = path;
      editorContent = file.content;
      editorOriginalContent = file.content;
      editorEtag = file.etag;
      editorError = '';
      editorPreview = false;
      editorPreviewSource = '';
      editorPreviewStale = false;
      editorReadOnly = file.writable === false || currentReadOnly;
      editorReadOnlyReason = file.writable === false
        ? (file.read_only_reason ?? 'file')
        : currentReadOnly
          ? 'policy'
          : null;
      editorLineEnding = file.line_ending ?? 'lf';
      editorEncoding = file.encoding ?? 'utf-8';
      editorBom = file.bom ?? false;
      editorLossy = file.lossy ?? false;
      if (editorLossy) {
        editorReadOnly = true;
        editorReadOnlyReason = 'lossy';
      }
      editorOpen = true;
      if (!editorReadOnly) beginEditPresence(activeRootId, path);
    } catch (error) {
      if (error instanceof ApiError && (error.status === 400 || error.status === 415)) {
        showUnsupported(entry);
      } else {
        handleFilesError(error);
      }
    }
  }

  function applyEditorFile(file: {
    content: string;
    etag: string;
    line_ending?: 'lf' | 'crlf' | 'mixed';
    writable?: boolean;
    read_only_reason?: 'file' | 'directory' | null;
    encoding?: string;
    bom?: boolean;
    lossy?: boolean;
  }): void {
    editorContent = file.content;
    editorOriginalContent = file.content;
    editorEtag = file.etag;
    editorLineEnding = file.line_ending ?? 'lf';
    editorEncoding = file.encoding ?? 'utf-8';
    editorBom = file.bom ?? false;
    editorLossy = file.lossy ?? false;
    editorPreviewSource = '';
    editorPreviewStale = false;
    editorReadOnly = file.writable === false || currentReadOnly || editorLossy;
    editorReadOnlyReason = editorLossy
      ? 'lossy'
      : file.writable === false
        ? (file.read_only_reason ?? 'file')
        : currentReadOnly
          ? 'policy'
          : null;
  }

  async function reloadEditorEncoding(encoding: string): Promise<void> {
    if (activeRootId === null || !editorOpen) return;
    try {
      const file = await api.getFile(
        activeRootId,
        editorPath,
        encoding === 'auto' ? undefined : encoding,
      );
      applyEditorFile(file);
      if (editorReadOnly) endEditPresence();
      else beginEditPresence(activeRootId, editorPath);
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '读取文件失败'));
    }
  }

  function changeEditorEncoding(value: string): void {
    if (value === editorEncoding) return;
    if (editorDirty) {
      confirmState = {
        title: t($language, '切换编码'),
        message: t($language, '有未保存修改，切换编码会丢弃，确定？'),
        requireText: null,
        action: () => {
          confirmState = null;
          void reloadEditorEncoding(value);
        },
      };
      return;
    }
    void reloadEditorEncoding(value);
  }

  function contentForSave(): string {
    if (editorLineEnding !== 'crlf') return editorContent;
    return editorContent.replace(/\r\n/g, '\n').replace(/\n/g, '\r\n');
  }

  async function saveEditor(auto = false, force = false): Promise<void> {
    if (activeRootId === null || !editorOpen || editorReadOnly || currentReadOnly) return;
    editorSaving = true;
    editorError = '';
    try {
      const saved = await api.saveFile(
        activeRootId,
        editorPath,
        contentForSave(),
        editorEtag,
        csrf,
        editorEncoding,
        editorBom,
        force,
      );
      editorEtag = saved.etag;
      editorOriginalContent = editorContent;
      if (!auto) closeEditor(true);
      clearDirSizeCache();
      await loadList(activeRootId, currentPath);
    } catch (error) {
      if (error instanceof ApiError && error.status === 412 && !auto) {
        confirmState = {
          title: t($language, '文件已被修改'),
          message: t($language, '文件已被其他操作修改。要用当前内容覆盖吗？'),
          requireText: null,
          action: () => {
            confirmState = null;
            void saveEditor(true, true);
          },
        };
      } else if (error instanceof ApiError && error.status === 412) {
        editorError = t($language, '文件已被其他操作修改，请关闭后重新打开。');
      } else if (error instanceof ApiError && error.code === 'PERMISSION_DENIED') {
        editorReadOnly = true;
        editorReadOnlyReason = editorReadOnlyReason ?? 'file';
        endEditPresence();
        editorError = t($language, '文件已不可写（系统权限），内容仍可复制');
      } else if (error instanceof ApiError && error.code === 'FORBIDDEN') {
        editorReadOnly = true;
        editorReadOnlyReason = 'policy';
        endEditPresence();
        editorError = t($language, '目录已设为只读，无法保存');
      } else if (error instanceof ApiError && error.code === 'ENCODING_UNREPRESENTABLE') {
        confirmState = {
          title: t($language, '无法用当前编码保存'),
          message: t($language, '当前编码无法表示部分字符，改为 UTF-8 保存？'),
          requireText: null,
          action: () => {
            confirmState = null;
            editorEncoding = 'utf-8';
            editorBom = false;
            void saveEditor();
          },
        };
      } else {
        editorError = error instanceof Error ? error.message : t($language, '保存失败');
      }
    } finally {
      editorSaving = false;
    }
  }

  async function downloadSelectedZip(): Promise<void> {
    await downloadZipEntries(entries.filter((entry) => selected.includes(entry.name)));
  }

  async function downloadZipEntries(items: Entry[]): Promise<void> {
    if (activeRootId === null || items.length === 0) return;

    try {
      const paths = items.map((entry) => joinPath(currentPath, entry.name));
      const blob = await api.downloadZip(activeRootId, paths, csrf);
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = 'filehub.zip';
      anchor.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      handleFilesError(error);
    }
  }

  function formatSize(size: number): string {
    if (size < 1024) return `${size} B`;
    if (size < 1024 ** 2) return `${(size / 1024).toFixed(1)} KB`;
    if (size < 1024 ** 3) return `${(size / 1024 ** 2).toFixed(1)} MB`;
    if (size < 1024 ** 4) return `${(size / 1024 ** 3).toFixed(1)} GB`;
    return `${(size / 1024 ** 4).toFixed(1)} TB`;
  }

  function isImageName(name: string): boolean {
    return /\.(jpe?g|png|gif|webp|bmp|tiff?|svg)$/i.test(name);
  }

  function isRasterImageName(name: string): boolean {
    return /\.(jpe?g|png|gif|webp|bmp|tiff?)$/i.test(name);
  }

  function isSvgName(name: string): boolean {
    return /\.svg$/i.test(name);
  }

  function hideBrokenThumb(event: Event, key: string): void {
    const image = event.currentTarget as HTMLImageElement;
    image.style.display = 'none';
    thumbFailures = { ...thumbFailures, [key]: true };
  }

  function formatRelativeDate(seconds: number): string {
    if (!seconds) return '—';
    const parts = relativeTimeParts(seconds, clock);
    switch (parts.unit) {
      case 'just-now':
        return t($language, '刚刚');
      case 'minute':
        return t($language, '{count} 分钟前', { count: parts.count });
      case 'hour':
        return t($language, '{count} 小时前', { count: parts.count });
      case 'day':
        return t($language, '{count} 天前', { count: parts.count });
      case 'week':
        return t($language, '{count} 周前', { count: parts.count });
      case 'month':
        return t($language, '{count} 个月前', { count: parts.count });
      default:
        return t($language, '{count} 年前', { count: parts.count });
    }
  }

  function absoluteDate(seconds: number): string {
    return absoluteTime(seconds, localeOf($language));
  }
</script>

<svelte:head>
  <title>{pageTitle}</title>
  {#if $branding.customCssVersion > 0}
    <link rel="stylesheet" href={api.customCssUrl($branding.customCssVersion)} />
  {/if}
</svelte:head>

<svelte:window
  onclick={() => {
    contextMenu = null;
    blankMenu = null;
    sortMenuOpen = false;
  }}
  onscroll={() => {
    contextMenu = null;
    blankMenu = null;
  }}
/>

{#if shareToken}
  <SharePage token={shareToken} />
{:else if loading}
  <div class="boot-screen" aria-label={t($language, '正在加载')}>
    <div class="boot-mark" aria-hidden="true">F</div>
    <p class="boot-label">{t($language, '加载中…')}</p>
    {#if bootSlow}
      <p class="boot-hint">{t($language, '疑似网络延迟较高，仍在加载…')}</p>
    {/if}
  </div>
{:else if !initialized}
  <AuthScreen mode="setup" onSuccess={handleAuth} />
{:else if !user}
  <AuthScreen mode="login" onSuccess={handleAuth} />
{:else}
  <div
    class="app"
    role="application"
    ondragover={(event) => event.preventDefault()}
    ondrop={(event) => void handleAppDrop(event)}
  >
    <header class="topbar">
      <button
        type="button"
        class="icon-button mobile-menu"
        aria-label={t($language, '打开菜单')}
        onclick={() => (sidebarOpen = true)}
      >
        <Icon name="menu" size={20} />
      </button>
      <div class="topbar-brand">
        <span class="brand-mark" class:has-icon={$branding.iconUrl} aria-hidden="true">
          {#if $branding.iconUrl}
            <img src={$branding.iconUrl} alt="" />
          {:else}
            {brandMark($branding.name)}
          {/if}
        </span>
        <span class="brand-name">{$branding.name}</span>
      </div>

      {#if appView === 'files'}
        <form
          class="search"
          onsubmit={(event) => {
            event.preventDefault();
            submitSearch();
          }}
        >
          <button type="submit" class="search-submit" aria-label={t($language, '搜索')}>
            <Icon name="search" size={18} />
          </button>
          <input
            bind:this={searchInput}
            bind:value={searchQuery}
            aria-label={t($language, '搜索文件或目录')}
            placeholder={t($language, '搜索文件或目录')}
            disabled={!remoteCan('search')}
            oninput={handleSearchInput}
          />
        </form>
      {:else}
        <div class="page-context"></div>
      {/if}

      <div class="top-actions">
        {#if appView === 'files'}
          <div class="segmented" role="group" aria-label={t($language, '视图')}>
            <button
              type="button"
              class:active={view === 'list'}
              aria-pressed={view === 'list'}
              aria-label={t($language, '列表视图')}
              title={t($language, '列表视图')}
              onclick={() => setView('list')}
            >
              <Icon name="list" size={18} />
            </button>
            <button
              type="button"
              class:active={view === 'thumb'}
              aria-pressed={view === 'thumb'}
              aria-label={t($language, '缩略图视图')}
              title={t($language, '缩略图视图')}
              onclick={() => setView('thumb')}
            >
              <Icon name="grid" size={18} />
            </button>
            <button
              type="button"
              class:active={view === 'tile'}
              aria-pressed={view === 'tile'}
              aria-label={t($language, '平铺视图')}
              title={t($language, '平铺视图')}
              onclick={() => setView('tile')}
            >
              <Icon name="tile" size={18} />
            </button>
          </div>
          {#if view !== 'list'}
            <div class="sort-control">
              <button
                type="button"
                class="sort-trigger"
                aria-label={t($language, '排序方式')}
                onclick={(event) => {
                  event.stopPropagation();
                  sortMenuOpen = !sortMenuOpen;
                }}
              >
                {sortField === 'name'
                  ? t($language, '名称')
                  : sortField === 'size'
                    ? t($language, '大小')
                    : t($language, '修改时间')}{sortOrder === 'asc' ? ' ↑' : ' ↓'}
              </button>
              {#if sortMenuOpen}
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <div
                  class="sort-popup"
                  role="menu"
                  tabindex="-1"
                  onclick={(event) => event.stopPropagation()}
                >
                  {#each [
                    ['name', t($language, '名称')],
                    ['size', t($language, '大小')],
                    ['modified', t($language, '修改时间')],
                  ] as [field, label] (field)}
                    <button
                      type="button"
                      role="menuitem"
                      class:active={sortField === field}
                      onclick={() => {
                        setSort(field as 'name' | 'size' | 'modified');
                        sortMenuOpen = false;
                      }}
                    >
                      {label}{sortField === field ? (sortOrder === 'asc' ? ' ↑' : ' ↓') : ''}
                    </button>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        {/if}

        <NotificationCenter />

        <button
          type="button"
          class="icon-button"
          aria-label={theme === 'dark'
            ? t($language, '切换到浅色模式')
            : t($language, '切换到深色模式')}
          title={theme === 'dark' ? t($language, '切换到浅色模式') : t($language, '切换到深色模式')}
          onclick={toggleTheme}
        >
          <Icon name={theme === 'dark' ? 'sun' : 'moon'} size={19} />
        </button>

        {#if appView === 'files'}
          <button
            type="button"
            class="upload-top"
            class:inactive={currentReadOnly || !remoteCan('uploads')}
            aria-disabled={!activeRoot || currentReadOnly || !remoteCan('uploads')}
            title={
              currentReadOnly || !remoteCan('uploads')
                ? readOnlyMessage()
                : t($language, '上传文件')
            }
            aria-label={t($language, '上传')}
            onclick={uploadClick}
          >
            <Icon name="upload" size={18} />
            <span>{t($language, '上传')}</span>
          </button>
        {/if}

        <button
          type="button"
          class="top-avatar-button"
          aria-label={t($language, '账号菜单')}
          title={t($language, '账号菜单')}
          onclick={() => (accountOpen = true)}
        >
          {#if preferences && preferences.avatar_version > 0}
            <img src={api.avatarUrl(preferences.avatar_version)} alt="" />
          {:else}
            {user.username.slice(0, 1).toUpperCase()}
          {/if}
        </button>
      </div>
    </header>

    <div class="workspace">
      <aside class:open={sidebarOpen} class="sidebar" aria-label={t($language, '主导航')}>
        <nav class="nav" aria-label={t($language, '文件导航')}>
          {#each localRoots as root (root.id)}
            <button
              type="button"
              class:active={appView === 'files' && root.id === activeRootId}
              class="nav-item"
              title={rootLabel(root.alias)}
              onclick={() => {
                appView = 'files';
                activeRootId = root.id;
                void loadList(root.id, '');
              }}
            >
              <Icon name="folder" size={18} />
              <span class="nav-label">{rootLabel(root.alias)}</span>
              {#if root.shared}
                <span class="nav-shared" title={t($language, '已共享给远程实例')}>
                  <Icon name="link" size={13} />
                </span>
              {/if}
              {#if root.read_only}
                <span class="nav-readonly" title={t($language, '目录已设为只读')}
                  >{t($language, '只读')}</span
                >
              {/if}
            </button>
          {/each}
          {#if localRoots.length === 0 && remoteRoots.length === 0}
            <span class="nav-empty">{t($language, '暂无目录')}</span>
          {/if}
          {#if remoteRoots.length > 0}
            <div class="nav-sep"></div>
            {#each remoteRoots as root (root.id)}
              <button
                type="button"
                class:active={appView === 'files' && root.id === activeRootId}
                class="nav-item"
                title={remoteTitle(root.id, rootLabel(root.alias))}
                onclick={() => {
                  appView = 'files';
                  activeRootId = root.id;
                  void loadList(root.id, '');
                }}
              >
                <Icon name="link" size={18} />
                <span class="nav-label">{rootLabel(root.alias)}</span>
                {#if remoteStateBadge(root.id)}
                  <span
                    class="nav-state"
                    role="img"
                    aria-label={remoteStateBadge(root.id)}
                    title={remoteStateBadge(root.id)}
                  ></span>
                {/if}
                {#if root.read_only}
                  <span class="nav-readonly" title={t($language, '目录已设为只读')}
                    >{t($language, '只读')}</span
                  >
                {/if}
              </button>
            {/each}
          {/if}
          {#if canMountRemote}
            {#if remoteRoots.length === 0}
              <div class="nav-sep"></div>
            {/if}
            <button
              type="button"
              class="nav-item nav-add"
              title={t($language, '挂载远程目录')}
              onclick={() => (mountDialogOpen = true)}
            >
              <Icon name="plus" size={18} />
              <span class="nav-label">{t($language, '挂载远程目录')}</span>
            </button>
          {/if}
          <div class="nav-sep"></div>
          {#if terminalAvailable}
            <button
              type="button"
              class="nav-item"
              disabled={!activeRoot}
              title={activeRoot ? t($language, '终端') : t($language, '请先选择存储目录')}
              onclick={() => {
                terminalOpen = true;
                terminalMinimized = false;
              }}
            >
              <Icon name="terminal" size={18} />
              <span class="nav-label">{t($language, '终端')}</span>
            </button>
          {/if}
          <button
            type="button"
            class:active={appView === 'shares'}
            class="nav-item"
            onclick={() => (appView = 'shares')}
          >
            <Icon name="share" size={18} />
            <span class="nav-label">{t($language, '分享管理')}</span>
          </button>
        </nav>

        <div class="sidebar-spacer"></div>

        <div class="quota-card" aria-label={t($language, '存储状态')}>
          <div class="quota-row">
            <div class="quota-label">
              <span>{t($language, '用户配额')}</span>
              <span class="quota-value">{quotaLabel}</span>
            </div>
            <div class="quota-track">
              <div class="quota-fill" style={`width:${quotaPercent}%`}></div>
            </div>
          </div>
          {#each usage?.disks ?? [] as disk (disk.name)}
            <div class="quota-row">
              <div class="quota-label">
                <span class="disk-name" title={disk.name}>{disk.name}</span>
                <span class="quota-value">{formatSize(disk.used)} / {formatSize(disk.total)}</span>
              </div>
              <div class="quota-track">
                <div
                  class="quota-fill disk"
                  style={`width:${disk.total ? Math.min(100, (disk.used / disk.total) * 100) : 0}%`}
                ></div>
              </div>
            </div>
          {/each}
        </div>
        <div class="app-version">
          <a
            class="version-link"
            href={versionInfo?.repository ?? 'https://github.com/Llesue/filehub'}
            target="_blank"
            rel="noopener noreferrer"
            aria-label={t($language, '在GitHub上查看源码')}
            title="GitHub"
          >
            <Icon name="github" size={14} />
          </a>
          <span>v{versionInfo?.version ?? __APP_VERSION__}</span>
          {#if versionInfo?.update_available}
            <a
              class="version-update"
              href={versionInfo?.releases ?? 'https://github.com/Llesue/filehub/releases'}
              target="_blank"
              rel="noopener noreferrer"
            >
              {t($language, '有新版本')}
            </a>
          {/if}
        </div>
      </aside>

      <input
        bind:this={fileInput}
        class="upload-input"
        type="file"
        multiple
        onchange={(event) => void handleFiles(event)}
      />
      <input
        bind:this={folderInput}
        class="upload-input"
        type="file"
        multiple
        webkitdirectory
        onchange={(event) => void handleFolderFiles(event)}
      />

      {#if sidebarOpen}
        <button
          type="button"
          class="overlay"
          aria-label={t($language, '关闭菜单')}
          onclick={() => (sidebarOpen = false)}
        ></button>
      {/if}

      <div class="main">
        {#if appView === 'files'}
          <div class="pathbar">
            {#if !searchActive}
              <div class="path-static">
                <Icon name="folder" size={15} />
                {#if currentStale}
                  <span class="stale-dot" title={t($language, '数据可能已过期')}></span>
                {/if}
                {#if currentReadOnly}
                  <span
                    class="readonly-tag"
                    title={currentReadOnlyReason === 'filesystem'
                      ? t($language, '服务进程没有文件系统写入权限')
                      : t($language, '目录已设为只读')}
                    >{currentReadOnlyReason === 'filesystem'
                      ? t($language, '只读（系统）')
                      : t($language, '只读')}</span
                  >
                {/if}
              </div>
            {/if}
            {#if searchActive}
              <div class="path search-path">
                <Icon name="search" size={15} />
                <span class="search-query" title={searchQuery}>“{searchQuery.trim()}”</span>
                {#each searchFilterTags() as tag, index (index)}
                  <span class="search-filter-tag">{tag}</span>
                {/each}
                <span class="sep">·</span>
                <span class="search-state">{searchStatusText()}</span>
                <button
                  type="button"
                  class="icon-button path-clear"
                  aria-label={t($language, '清除搜索')}
                  title={t($language, '清除搜索')}
                  onclick={exitSearch}
                >
                  <Icon name="close" size={14} />
                </button>
              </div>
            {:else}
              <div class="path" bind:this={pathBarEl}>
                <button
                  type="button"
                  class="path-link path-root"
                  onclick={() => navigate('')}
                >
                  {activeRoot ? rootLabel(activeRoot.alias) : t($language, '文件')}
                </button>
                {#if collapsedCount > 0}
                  <span class="sep">/</span>
                  <div class="path-more">
                    <button
                      type="button"
                      class="path-link path-more-button"
                      aria-label={t($language, '显示隐藏的 {count} 个路径段', {
                        count: collapsedCount,
                      })}
                      title={t($language, '显示隐藏的 {count} 个路径段', {
                        count: collapsedCount,
                      })}
                      aria-expanded={pathMenuOpen}
                      onclick={() => (pathMenuOpen = !pathMenuOpen)}
                    >
                      …
                    </button>
                    {#if pathMenuOpen}
                      <div class="path-menu" role="menu">
                        {#each pathSegments.slice(0, collapsedCount) as segment, index (index)}
                          <button
                            type="button"
                            role="menuitem"
                            onclick={() => {
                              pathMenuOpen = false;
                              navigateToSegment(index);
                            }}
                          >
                            {segment}
                          </button>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/if}
                {#each pathSegments.slice(collapsedCount) as segment, offset (offset)}
                  <span class="sep">/</span>
                  <button
                    type="button"
                    class="path-link"
                    onclick={() => navigateToSegment(collapsedCount + offset)}
                  >
                    {segment}
                  </button>
                {/each}
              </div>
            {/if}
            {#if selected.length > 0}
              <div class="pathbar-actions">
                <span class="selection-count"
                  >{t($language, '已选 {count} 项', { count: selected.length })}</span
                >
                <span class="selection-size">{formatSize(selectionSummary.bytes)}</span>
                {#each selectionSummary.parts as part, index (index)}
                  <span class="selection-kind">{part}</span>
                {/each}
                {#if capabilities.zip && remoteCan('download')}
                  <button
                    type="button"
                    class="icon-button"
                    aria-label={t($language, '下载 ZIP')}
                    title={t($language, '下载 ZIP')}
                    onclick={() => void downloadSelectedZip()}
                  >
                    <Icon name="download" size={16} />
                  </button>
                {/if}
                {#if remoteCan('copy')}
                  <button
                    type="button"
                    class="icon-button"
                    aria-label={t($language, '复制到')}
                    title={t($language, '复制到')}
                    onclick={() =>
                      (transfer = {
                        mode: 'copy',
                        entries: entries.filter((entry) => selected.includes(entry.name)),
                      })}
                  >
                    <Icon name="copy" size={16} />
                  </button>
                {/if}
                {#if !currentReadOnly && remoteCan('move')}
                  <button
                    type="button"
                    class="icon-button"
                    aria-label={t($language, '移动到')}
                    title={t($language, '移动到')}
                    onclick={() =>
                      (transfer = {
                        mode: 'move',
                        entries: entries.filter((entry) => selected.includes(entry.name)),
                      })}
                  >
                    <Icon name="move" size={16} />
                  </button>
                {/if}
                {#if !currentReadOnly && remoteCan('delete')}
                  <button
                    type="button"
                    class="icon-button danger"
                    aria-label={t($language, '删除')}
                    title={t($language, '删除')}
                    onclick={() => void deleteSelected()}
                  >
                    <Icon name="trash" size={16} />
                  </button>
                {/if}
                <button
                  type="button"
                  class="icon-button"
                  aria-label={t($language, '取消选择')}
                  title={t($language, '取消选择')}
                  onclick={clearSelection}
                >
                  <Icon name="close" size={16} />
                </button>
              </div>
            {/if}
          </div>
        {/if}

        <main class="content" bind:this={contentElement} onscroll={handleContentScroll}>
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="content-drag"
            class:file-area={appView === 'files'}
            role="application"
            aria-label={t($language, '文件内容')}
            onmousedown={startMarquee}
            onmousemove={updateMarquee}
            onmouseup={finishMarquee}
            onclick={handleContentClick}
            oncontextmenu={handleBlankContextMenu}
          >
            {#if settingsMounted}
              <div class:view-hidden={appView !== 'settings'}>
                {#if preferences}
                  <PreferencesPage
                    {user}
                    {csrf}
                    {preferences}
                    {capabilities}
                    system={systemFeatures}
                    canShareRemote={canShareRemote}
                    onPreferences={applyPreferences}
                  />
                {/if}
              </div>
            {/if}
            {#if sharesMounted}
              <div class:view-hidden={appView !== 'shares'}>
                <SharesPage {csrf} />
              </div>
            {/if}
            {#if adminMounted}
              <div class:view-hidden={appView !== 'admin'}>
                <AdminPage
                  {csrf}
                  onAudit={() => (appView = 'audit')}
                  onRootsChanged={() => void refreshRoots()}
                />
              </div>
            {/if}
            {#if appView === 'settings'}
              <!-- kept mounted above so returning to settings does not reload -->
            {:else if appView === 'admin'}
              <!-- kept mounted above so returning to admin does not reload -->
            {:else if appView === 'audit'}
              <AuditPage onBack={() => (appView = 'admin')} />
            {:else if appView === 'shares'}
              <!-- kept mounted above so returning to shares does not reload -->
            {:else}
              {#if searchActive}
                <div class="search-header">
                  <span>{t($language, '搜索')} “{searchQuery}”</span>
                  <div class="search-header-actions">
                    {#if searchLoading}
                      <span class="muted">{t($language, '搜索中…')}</span>
                    {/if}
                    <button
                      type="button"
                      class:active={searchFiltersOpen}
                      class="ghost"
                      onclick={() => (searchFiltersOpen = !searchFiltersOpen)}
                      >{t($language, '筛选')}</button
                    >
                    <button type="button" class="ghost" onclick={exitSearch}
                      >{t($language, '清除')}</button
                    >
                  </div>
                </div>
                {#if searchFiltersOpen}
                  <div class="search-filters">
                    <label>
                      <span>{t($language, '范围')}</span>
                      <select bind:value={searchScope} onchange={applySearchFilters}>
                        <option value="">{t($language, '全部（仅本地）')}</option>
                        {#each roots as root (root.id)}
                          <option value={String(root.id)}>
                            {rootLabel(root.alias)}{root.kind === 'remote'
                              ? ` · ${t($language, '远程')}`
                              : ''}
                          </option>
                        {/each}
                      </select>
                    </label>
                    <label>
                      <span>{t($language, '类型')}</span>
                      <select bind:value={searchKind} onchange={applySearchFilters}>
                        <option value="">{t($language, '全部')}</option>
                        <option value="image">{t($language, '图片')}</option>
                        <option value="video">{t($language, '视频')}</option>
                        <option value="audio">{t($language, '音频')}</option>
                        <option value="text">{t($language, '文本')}</option>
                        <option value="pdf">{t($language, 'PDF')}</option>
                        <option value="dir">{t($language, '目录')}</option>
                        <option value="file">{t($language, '文件')}</option>
                      </select>
                    </label>
                    <label>
                      <span>{t($language, '最小大小')}</span>
                      <input
                        type="text"
                        inputmode="decimal"
                        bind:value={searchMinMb}
                        onchange={applySearchFilters}
                        placeholder="MB"
                      />
                    </label>
                    <label>
                      <span>{t($language, '最大大小')}</span>
                      <input
                        type="text"
                        inputmode="decimal"
                        bind:value={searchMaxMb}
                        onchange={applySearchFilters}
                        placeholder="MB"
                      />
                    </label>
                    <label>
                      <span>{t($language, '修改时间')}</span>
                      <select bind:value={searchModified} onchange={applySearchFilters}>
                        <option value="">{t($language, '全部时间')}</option>
                        <option value="86400">{t($language, '今天')}</option>
                        <option value="604800">{t($language, '最近 7 天')}</option>
                        <option value="2592000">{t($language, '最近 30 天')}</option>
                        <option value="31536000">{t($language, '最近一年')}</option>
                      </select>
                    </label>
                  </div>
                {/if}
                <div class="search-controls">
                  {#each [
                    ['name', t($language, '名称')],
                    ['size', t($language, '大小')],
                    ['modified', t($language, '修改时间')],
                  ] as [field, label] (field)}
                    <button
                      type="button"
                      class="sort-button"
                      class:active={searchSort === field}
                      onclick={() => setSearchSort(field as 'name' | 'size' | 'modified')}
                    >
                      {label}{#if searchSort === field}{searchSortOrder === 'asc' ? ' ↑' : ' ↓'}{/if}
                    </button>
                  {/each}
                  <button
                    type="button"
                    class="sort-button"
                    title={searchView === 'list'
                      ? t($language, '网格视图')
                      : t($language, '列表视图')}
                    aria-label={searchView === 'list'
                      ? t($language, '网格视图')
                      : t($language, '列表视图')}
                    onclick={() => (searchView = searchView === 'list' ? 'grid' : 'list')}
                  >
                    {searchView === 'list' ? t($language, '网格视图') : t($language, '列表视图')}
                  </button>
                  <button type="button" class="sort-button" onclick={toggleSearchSelectAll}>
                    {searchSelected.length === searchSortedResults.length &&
                    searchSortedResults.length > 0
                      ? t($language, '清空选择')
                      : t($language, '全选')}
                  </button>
                  {#if searchSelected.length > 0}
                    <span class="selection-count"
                      >{t($language, '已选 {count} 项', { count: searchSelected.length })}</span
                    >
                    {#if searchSelectedRoot === undefined}
                      <span class="danger-text">{t($language, '跨 root 选择不能批量操作')}</span>
                    {:else}
                      <button
                        type="button"
                        class="sort-button danger-text"
                        disabled={searchSelectedReadOnly}
                        onclick={() => void deleteSearchSelected()}
                      >
                        {t($language, '删除')}
                      </button>
                      <button
                        type="button"
                        class="sort-button"
                        onclick={shareSearchSelected}
                      >
                        {t($language, '分享')}
                      </button>
                    {/if}
                    <button type="button" class="sort-button" onclick={() => (searchSelected = [])}>
                      {t($language, '清空选择')}
                    </button>
                  {/if}
                </div>
                {#if searchResults.length === 0}
                  <div class="files-state">
                    {searchLoading
                      ? t($language, '搜索中…')
                      : searchTooShort
                        ? t($language, '请输入至少 3 个字符（中文可输入 2 个）')
                        : searchFailed
                          ? t($language, '搜索失败')
                          : t($language, '没有找到匹配的文件')}
                  </div>
                {:else}
                  {#if searchView === 'grid'}
                    <div class="search-grid">
                      {#each searchSortedResults as result (searchEntryKey(result))}
                        {@const key = searchEntryKey(result)}
                        {@const thumbId = thumbKey(result.rootId, result.path)}
                        <div class="search-card" class:selected={searchSelected.includes(key)}>
                          <input
                            class="search-check"
                            type="checkbox"
                            aria-label={t($language, '选择 {name}', { name: result.name })}
                            checked={searchSelected.includes(key)}
                            onchange={() => toggleSearchSelection(result)}
                          />
                          <button
                            type="button"
                            class="search-more"
                            aria-label={t($language, '更多操作')}
                            title={t($language, '更多操作')}
                            onclick={(event) => openSearchMenu(event, result)}
                          >
                            <Icon name="more" size={16} />
                          </button>
                          <button
                            type="button"
                            class="search-card-open"
                            onclick={() => openSearchResult(result)}
                          >
                            <div class="search-thumb">
                              <FileTypeIcon
                                name={result.name}
                                isDir={result.is_dir}
                                size={44}
                              />
                              {#if systemFeatures.thumbnails && preferences?.show_thumbnails !== false && !result.is_dir && result.has_preview && !thumbFailures[thumbId]}
                                <img
                                  class="thumb-image"
                                  src={api.thumbUrl(
                                    result.rootId,
                                    result.path,
                                    320,
                                    thumbVersions[thumbId],
                                    !$realtimeActive,
                                  )}
                                  alt=""
                                  loading="lazy"
                                  decoding="async"
                                  fetchpriority="low"
                                  onerror={(event) => hideBrokenThumb(event, thumbId)}
                                />
                              {/if}
                            </div>
                            <span class="file-name" title={result.name}>{result.name}</span>
                            <span class="search-path">{result.path}</span>
                            <span
                              class="search-size"
                              title={result.dir_size_stale
                                ? t($language, '可能过期')
                                : undefined}
                            >
                              {#if result.is_dir}
                                {result.dir_size != null ? formatSize(result.dir_size) : '—'}
                                {#if result.entry_count != null}
                                  <span class="search-count">{result.entry_count}</span>
                                {/if}
                              {:else}
                                {formatSize(result.size)}
                              {/if}
                            </span>
                          </button>
                        </div>
                      {/each}
                    </div>
                  {:else}
                    <div class="search-results">
                      {#each searchSortedResults as result (searchEntryKey(result))}
                        {@const key = searchEntryKey(result)}
                        {@const thumbId = thumbKey(result.rootId, result.path)}
                        <div class="search-row" class:selected={searchSelected.includes(key)}>
                          <input
                            type="checkbox"
                            aria-label={t($language, '选择 {name}', { name: result.name })}
                            checked={searchSelected.includes(key)}
                            onchange={() => toggleSearchSelection(result)}
                          />
                          <button
                            type="button"
                            class="search-more"
                            aria-label={t($language, '更多操作')}
                            title={t($language, '更多操作')}
                            onclick={(event) => openSearchMenu(event, result)}
                          >
                            <Icon name="more" size={16} />
                          </button>
                          <button
                            type="button"
                            class="search-result"
                            onclick={() => openSearchResult(result)}
                          >
                            <span class="search-thumb small">
                              <FileTypeIcon
                                name={result.name}
                                isDir={result.is_dir}
                                size={18}
                              />
                              {#if systemFeatures.thumbnails && preferences?.show_thumbnails !== false && !result.is_dir && result.has_preview && !thumbFailures[thumbId]}
                                <img
                                  class="thumb-image"
                                  src={api.thumbUrl(
                                    result.rootId,
                                    result.path,
                                    96,
                                    thumbVersions[thumbId],
                                    !$realtimeActive,
                                  )}
                                  alt=""
                                  loading="lazy"
                                  decoding="async"
                                  fetchpriority="low"
                                  onerror={(event) => hideBrokenThumb(event, thumbId)}
                                />
                              {/if}
                            </span>
                            <span class="file-name" title={result.name}>{result.name}</span>
                            <span class="search-path">{result.path}</span>
                            <span
                              class="search-size"
                              title={result.dir_size_stale
                                ? t($language, '可能过期')
                                : undefined}
                            >
                              {#if result.is_dir}
                                {result.dir_size != null ? formatSize(result.dir_size) : '—'}
                                {#if result.entry_count != null}
                                  <span class="search-count">{result.entry_count}</span>
                                {/if}
                              {:else}
                                {formatSize(result.size)}
                              {/if}
                            </span>
                          </button>
                        </div>
                      {/each}
                    </div>
                  {/if}
                  {#if searchNextCursor}
                    <div class="search-more">
                      <button
                        type="button"
                        class="ghost"
                        disabled={searchLoadingMore}
                        onclick={() => void loadMoreSearch()}
                      >
                        {searchLoadingMore ? t($language, '加载中…') : t($language, '加载更多')}
                      </button>
                    </div>
                  {/if}
                {/if}
              {:else if filesLoading}
                <div class="files-state">
                  <span>{t($language, '加载中…')}</span>
                  {#if filesSlow}
                    <span class="slow-network-hint"
                      >{t($language, '疑似网络延迟较高，仍在加载…')}</span
                    >
                  {/if}
                </div>
              {:else if filesError}
                <div class="files-state">
                  <p>{filesError}</p>
                  <button
                    type="button"
                    class="retry-button"
                    onclick={() => {
                      if (activeRootId !== null) void loadList(activeRootId, currentPath);
                    }}
                  >
                    {t($language, '重试')}
                  </button>
                </div>
              {:else}
                {#if focusPage}
                  <div class="focus-banner" role="status">
                    <span
                      >{t($language, '已定位到第 {index} 项，共 {total} 项', {
                        index: focusPage.offset + 1,
                        total: focusPage.total,
                      })}</span
                    >
                    <button
                      type="button"
                      class="ghost"
                      onclick={() => {
                        if (activeRootId !== null) void loadList(activeRootId, currentPath);
                      }}
                    >
                      {t($language, '回到目录开头')}
                    </button>
                  </div>
                {/if}
                {#if entries.length === 0}
                  <section class="empty-state" aria-labelledby="empty-title">
                    <div class="empty-icon">
                      <Icon name="folder" size={52} />
                    </div>
                    <h1 id="empty-title">{t($language, '这个目录是空的')}</h1>
                    <p>{t($language, '上传文件或创建目录后，它们会显示在这里。')}</p>
                    <button
                      type="button"
                      class="upload-primary"
                      class:inactive={currentReadOnly}
                      aria-disabled={!activeRoot || currentReadOnly}
                      onclick={uploadClick}
                    >
                      <Icon name="upload" size={18} />
                      <span>{t($language, '上传文件')}</span>
                    </button>
                  </section>
                {:else if view === 'list'}
                  <div class="browser" class:refreshing={filesRefreshing}>
                    <table class="file-table" bind:this={fileTable}>
                      <thead>
                        <tr>
                          <th class="check"></th>
                          <th
                            class:active={sortField === 'name'}
                            aria-sort={sortField === 'name'
                              ? sortOrder === 'asc'
                                ? 'ascending'
                                : 'descending'
                              : 'none'}
                          >
                            <button
                              type="button"
                              class="sort-button"
                              onclick={() => setSort('name')}
                            >
                              {t($language, '名称')}{sortField === 'name'
                                ? sortOrder === 'asc'
                                  ? ' ↑'
                                  : ' ↓'
                                : ''}
                            </button>
                          </th>
                          <th
                            class="size"
                            aria-sort={sortField === 'size'
                              ? sortOrder === 'asc'
                                ? 'ascending'
                                : 'descending'
                              : 'none'}
                          >
                            <button
                              type="button"
                              class="sort-button"
                              onclick={() => setSort('size')}
                            >
                              {t($language, '大小')}{sortField === 'size'
                                ? sortOrder === 'asc'
                                  ? ' ↑'
                                  : ' ↓'
                                : ''}
                            </button>
                          </th>
                          <th
                            class="modified"
                            aria-sort={sortField === 'modified'
                              ? sortOrder === 'asc'
                                ? 'ascending'
                                : 'descending'
                              : 'none'}
                          >
                            <button
                              type="button"
                              class="sort-button"
                              onclick={() => setSort('modified')}
                            >
                              {t($language, '修改时间')}{sortField === 'modified'
                                ? sortOrder === 'asc'
                                  ? ' ↑'
                                  : ' ↓'
                                : ''}
                            </button>
                          </th>
                          <th class="actions"></th>
                        </tr>
                      </thead>
                      <tbody>
                        {#if entries.length > VIRTUAL_THRESHOLD && virtualStart > 0}
                          <tr class="virtual-spacer" aria-hidden="true">
                            <td colspan="5" style:height={`${virtualStart * VIRTUAL_ROW_HEIGHT}px`}
                            ></td>
                          </tr>
                        {/if}
                        {#each visibleEntries as entry (entry.name)}
                          {@const nameParts = splitFileName(entry.name, { isDir: entry.is_dir })}
                          <tr
                            data-entry-name={entry.name}
                            class:selected={isSelected(entry.name)}
                            class:drag-source={isDragSource(entry)}
                            class:drop-target={entry.is_dir && dragOverKey === entryDragKey(entry)}
                            class:drop-invalid={entry.is_dir &&
                              dragOverKey === entryDragKey(entry) &&
                              !dragOverValid}
                            class:marquee-selected={marqueePreview.includes(entry.name)}
                            tabindex="0"
                            aria-selected={isSelected(entry.name)}
                            draggable="true"
                            onkeydown={(event) => {
                              if (event.key === 'Enter') {
                                event.preventDefault();
                                enterEntry(entry);
                              } else if (event.key === ' ') {
                                event.preventDefault();
                                toggleSelection(entry.name);
                              }
                            }}
                            onclick={(event) => handleEntryClick(event, entry)}
                            ondblclick={() => enterEntry(entry)}
                            oncontextmenu={(event) => openContextMenu(event, entry)}
                            ondragstart={(event) => startDrag(event, entry)}
                            ondragend={clearDrag}
                            ondragover={(event) => handleDragOver(event, entry)}
                            ondragleave={(event) => handleDragLeave(event, entry)}
                            ondrop={(event) => {
                              if (entry.is_dir) {
                                void dropInto(
                                  event,
                                  activeRootId ?? 0,
                                  joinPath(currentPath, entry.name),
                                );
                              }
                            }}
                          >
                            <td class="check" onclick={(event) => event.stopPropagation()}>
                              <input
                                type="checkbox"
                                aria-label={t($language, '选择 {name}', { name: entry.name })}
                                checked={isSelected(entry.name)}
                                onchange={() => toggleSelection(entry.name)}
                              />
                            </td>
                            <td class="name">
                              <span class="file-icon">
                                <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={22} />
                              </span>
                              <span class="file-name">
                                {#if entry.is_symlink}
                                  <span
                                    class="symlink-mark"
                                    class:bad={entry.symlink_state !== 'ok'}
                                    title={symlinkTitle(entry)}
                                  >
                                    <Icon
                                      name={entry.symlink_state === 'ok' ? 'link' : 'unlink'}
                                      size={14}
                                    />
                                  </span>
                                {/if}
                                <span class="name-stem">{nameParts.stem}</span><span
                                  class="name-ext">{nameParts.ext}</span
                                >{#if entry.shared}<span class="shared-mark" title={t($language, '已共享给远程实例')}><Icon name="link" size={13} /></span>{/if}
                              </span>
                            </td>
                            <td
                              class="size"
                              class:size-stale={entrySizeStale(entry)}
                              title={entrySizeError(entry)}>{entrySize(entry)}</td
                            >
                            <td class="modified" title={absoluteDate(entry.modified)}
                              >{formatRelativeDate(entry.modified)}</td
                            >
                            <td class="actions">
                              {#if !entry.is_dir && isAudioName(entry.name)}
                                <button
                                  type="button"
                                  class="row-action"
                                  aria-label={t($language, '播放')}
                                  title={t($language, '播放')}
                                  onclick={(event) => {
                                    event.stopPropagation();
                                    startAudio(entry);
                                  }}
                                >
                                  <Icon name="play" size={16} />
                                </button>
                              {/if}
                              <button
                                type="button"
                                class="row-action"
                                aria-label={t($language, '更多操作')}
                                title={t($language, '更多操作')}
                                onclick={(event) => openEntryMenu(event, entry)}
                              >
                                <Icon name="more" size={16} />
                              </button>
                              <button
                                type="button"
                                class="row-action"
                                aria-label={t($language, '删除 {name}', { name: entry.name })}
                                title={t($language, '删除')}
                                disabled={!activeRoot || currentReadOnly}
                                onclick={(event) => {
                                  event.stopPropagation();
                                  void deleteEntry(entry);
                                }}
                              >
                                <Icon name="trash" size={16} />
                              </button>
                            </td>
                          </tr>
                        {/each}
                        {#if entries.length > VIRTUAL_THRESHOLD && virtualEnd < entries.length}
                          <tr class="virtual-spacer" aria-hidden="true">
                            <td
                              colspan="5"
                              style:height={`${(entries.length - virtualEnd) * VIRTUAL_ROW_HEIGHT}px`}
                            ></td>
                          </tr>
                        {/if}
                      </tbody>
                    </table>
                  </div>
                {:else if view === 'thumb'}
                  <div class="browser">
                    <div
                      class="file-grid"
                      bind:this={gridElement}
                      style={`--virtual-pad-top:${virtualPadTop}px;--virtual-pad-bottom:${virtualPadBottom}px`}
                    >
                      {#each visibleEntries as entry (entry.name)}
                        {@const nameParts = splitFileName(entry.name, { isDir: entry.is_dir })}
                        {@const thumbPath = joinPath(currentPath, entry.name)}
                        {@const thumbId = thumbKey(activeRootId ?? 0, thumbPath)}
                        <div
                          class="file-card"
                          data-entry-name={entry.name}
                          class:selected={isSelected(entry.name)}
                          class:drag-source={isDragSource(entry)}
                          class:drop-target={entry.is_dir && dragOverKey === entryDragKey(entry)}
                          class:drop-invalid={entry.is_dir &&
                            dragOverKey === entryDragKey(entry) &&
                            !dragOverValid}
                          class:marquee-selected={marqueePreview.includes(entry.name)}
                          role="button"
                          tabindex="0"
                          draggable="true"
                          onclick={(event) => handleEntryClick(event, entry)}
                          ondblclick={() => enterEntry(entry)}
                          oncontextmenu={(event) => openContextMenu(event, entry)}
                          ondragstart={(event) => startDrag(event, entry)}
                          ondragend={clearDrag}
                          ondragover={(event) => handleDragOver(event, entry)}
                          ondragleave={(event) => handleDragLeave(event, entry)}
                          ondrop={(event) => {
                            if (entry.is_dir) {
                              void dropInto(
                                event,
                                activeRootId ?? 0,
                                joinPath(currentPath, entry.name),
                              );
                            }
                          }}
                          onkeydown={(event) => {
                            if (event.key === 'Enter' || event.key === ' ') {
                              event.preventDefault();
                              enterEntry(entry);
                            }
                          }}
                        >
                          <input
                            class="card-check"
                            type="checkbox"
                            aria-label={t($language, '选择 {name}', { name: entry.name })}
                            checked={isSelected(entry.name)}
                            onclick={(event) => event.stopPropagation()}
                            onchange={() => toggleSelection(entry.name)}
                          />
                          <button
                            type="button"
                            class="card-menu"
                            aria-label={t($language, '更多操作')}
                            title={t($language, '更多操作')}
                            onclick={(event) => openEntryMenu(event, entry)}
                          >
                            <Icon name="more" size={16} />
                          </button>
                          <div class="card-icon">
                            <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={40} />
                            {#if systemFeatures.thumbnails && preferences?.show_thumbnails !== false && !entry.is_dir && entry.has_preview && activeRootId !== null && !thumbFailures[thumbId]}
                              <img
                                class="thumb-image"
                                src={api.thumbUrl(
                                  activeRootId,
                                  thumbPath,
                                  320,
                                  thumbVersions[thumbId],
                                  !$realtimeActive,
                                )}
                                alt=""
                                loading="lazy"
                                decoding="async"
                                fetchpriority="low"
                                onerror={(event) => hideBrokenThumb(event, thumbId)}
                              />
                            {/if}
                          </div>
                          <span class="card-name">
                            {#if entry.is_symlink}
                              <span
                                class="symlink-mark"
                                class:bad={entry.symlink_state !== 'ok'}
                                title={symlinkTitle(entry)}
                              >
                                <Icon
                                  name={entry.symlink_state === 'ok' ? 'link' : 'unlink'}
                                  size={13}
                                />
                              </span>
                            {/if}
                            <span class="name-stem">{nameParts.stem}</span><span class="name-ext"
                              >{nameParts.ext}</span
                            >{#if entry.shared}<span class="shared-mark" title={t($language, '已共享给远程实例')}><Icon name="link" size={13} /></span>{/if}
                          </span>
                          <span class="card-meta">
                            <span title={absoluteDate(entry.modified)}
                              ><span class:size-stale={entrySizeStale(entry)}
                                >{entrySize(entry)}</span
                              >
                              · {formatRelativeDate(entry.modified)}</span
                            >
                          </span>
                        </div>
                      {/each}
                    </div>
                  </div>
                {:else}
                  <div class="browser">
                    <div
                      class="tile-grid"
                      bind:this={gridElement}
                      style={`--virtual-pad-top:${virtualPadTop}px;--virtual-pad-bottom:${virtualPadBottom}px`}
                    >
                      {#each visibleEntries as entry (entry.name)}
                        {@const nameParts = splitFileName(entry.name, { isDir: entry.is_dir })}
                        {@const thumbPath = joinPath(currentPath, entry.name)}
                        {@const thumbId = thumbKey(activeRootId ?? 0, thumbPath)}
                        <div
                          class="file-tile"
                          data-entry-name={entry.name}
                          class:selected={isSelected(entry.name)}
                          class:drag-source={isDragSource(entry)}
                          class:drop-target={entry.is_dir && dragOverKey === entryDragKey(entry)}
                          class:drop-invalid={entry.is_dir &&
                            dragOverKey === entryDragKey(entry) &&
                            !dragOverValid}
                          class:marquee-selected={marqueePreview.includes(entry.name)}
                          role="button"
                          tabindex="0"
                          draggable="true"
                          onclick={(event) => handleEntryClick(event, entry)}
                          ondblclick={() => enterEntry(entry)}
                          oncontextmenu={(event) => openContextMenu(event, entry)}
                          ondragstart={(event) => startDrag(event, entry)}
                          ondragend={clearDrag}
                          ondragover={(event) => handleDragOver(event, entry)}
                          ondragleave={(event) => handleDragLeave(event, entry)}
                          ondrop={(event) => {
                            if (entry.is_dir) {
                              void dropInto(
                                event,
                                activeRootId ?? 0,
                                joinPath(currentPath, entry.name),
                              );
                            }
                          }}
                          onkeydown={(event) => {
                            if (event.key === 'Enter' || event.key === ' ') {
                              event.preventDefault();
                              enterEntry(entry);
                            }
                          }}
                        >
                          <input
                            class="card-check"
                            type="checkbox"
                            aria-label={t($language, '选择 {name}', { name: entry.name })}
                            checked={isSelected(entry.name)}
                            onclick={(event) => event.stopPropagation()}
                            onchange={() => toggleSelection(entry.name)}
                          />
                          <button
                            type="button"
                            class="card-menu"
                            aria-label={t($language, '更多操作')}
                            title={t($language, '更多操作')}
                            onclick={(event) => openEntryMenu(event, entry)}
                          >
                            <Icon name="more" size={16} />
                          </button>
                          <div class="tile-icon">
                            <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={34} />
                            {#if systemFeatures.thumbnails && preferences?.show_thumbnails !== false && !entry.is_dir && entry.has_preview && activeRootId !== null && !thumbFailures[thumbId]}
                              <img
                                class="thumb-image"
                                src={api.thumbUrl(
                                  activeRootId,
                                  thumbPath,
                                  320,
                                  thumbVersions[thumbId],
                                  !$realtimeActive,
                                )}
                                alt=""
                                loading="lazy"
                                decoding="async"
                                fetchpriority="low"
                                onerror={(event) => hideBrokenThumb(event, thumbId)}
                              />
                            {/if}
                          </div>
                          <span class="tile-info">
                            <span class="tile-name">
                              {#if entry.is_symlink}
                                <span
                                  class="symlink-mark"
                                  class:bad={entry.symlink_state !== 'ok'}
                                  title={symlinkTitle(entry)}
                                >
                                  <Icon
                                    name={entry.symlink_state === 'ok' ? 'link' : 'unlink'}
                                    size={13}
                                  />
                                </span>
                              {/if}
                              <span class="name-stem">{nameParts.stem}</span><span class="name-ext"
                                >{nameParts.ext}</span
                              >{#if entry.shared}<span class="shared-mark" title={t($language, '已共享给远程实例')}><Icon name="link" size={13} /></span>{/if}
                            </span>
                            <span class="tile-meta">
                              <span title={absoluteDate(entry.modified)}
                                ><span class:size-stale={entrySizeStale(entry)}
                                  >{entrySize(entry)}</span
                                >
                                · {formatRelativeDate(entry.modified)}</span
                              >
                            </span>
                          </span>
                        </div>
                      {/each}
                    </div>
                  </div>
                {/if}
                {#if nextCursor || filesLoadingMore}
                  <div class="load-more">
                    {filesLoadingMore ? t($language, '加载中…') : t($language, '继续滚动加载更多')}
                  </div>
                {/if}
              {/if}
            {/if}
          </div>
        </main>
      </div>
    </div>
  </div>

  <NoticeDialog />
  <ProgressPanel />
  <UploadPanel />
  <ToastPanel />

  {#if accountOpen}
    <AccountDialog
      {user}
      avatarUrl={preferences && preferences.avatar_version > 0
        ? api.avatarUrl(preferences.avatar_version)
        : ''}
      onSettings={() => {
        appView = 'settings';
        accountOpen = false;
      }}
      onAdmin={() => {
        appView = 'admin';
        accountOpen = false;
      }}
      onLogout={() => {
        accountOpen = false;
        void logout();
      }}
      onClose={() => (accountOpen = false)}
    />
  {/if}

  {#if contextMenu}
    <div
      class="context-menu"
      class:open-left={contextMenuLeft}
      class:open-up={contextMenuUp}
      bind:this={contextMenuElement}
      role="menu"
      tabindex="-1"
      style={`left:${contextMenu.x}px;top:${contextMenu.y}px`}
      onclick={(event) => event.stopPropagation()}
      onkeydown={menuKeydown}
    >
      {#if menuMedia.length > 0}
        <button type="button" role="menuitem" onclick={() => runContextAction('preview')}
          >{menuTargets.length > 1
            ? t($language, '预览所选（{count} 项）', { count: menuMedia.length })
            : t($language, '预览')}</button
        >
      {/if}
      {#if menuTargets.length === 1 && isSvgName(contextMenu.entry.name)}
        <button type="button" role="menuitem" onclick={() => runContextAction('edit-source')}
          >{t($language, '编辑源码')}</button
        >
      {/if}
      {#if menuTargets.length === 1 && !contextMenu.entry.is_dir && isAudioName(contextMenu.entry.name)}
        <button type="button" role="menuitem" onclick={() => runContextAction('play')}
          >{t($language, '播放')}</button
        >
      {/if}
      {#if menuTargets.length === 1}
        <button type="button" role="menuitem" onclick={() => runContextAction('properties')}
          >{t($language, '详情')}</button
        >
      {/if}
      {#if menuTargets.length === 1 || capabilities.zip}
        <button type="button" role="menuitem" onclick={() => runContextAction('download')}
          >{menuTargets.length > 1
            ? t($language, '下载（ZIP）')
            : t($language, '下载')}</button
        >
      {/if}
      {#if capabilities.zip &&
      remoteCan('download') &&
      (menuTargets.length > 1 || menuTargets.some((entry) => entry.is_dir))}
        <button type="button" role="menuitem" onclick={() => runContextAction('archive-zip')}
          >{t($language, '打包为ZIP')}</button
        >
      {/if}
      {#if capabilities.unzip &&
      remoteCan('unzip') &&
      menuTargets.length === 1 &&
      !contextMenu.entry.is_dir &&
      /\.zip$/i.test(contextMenu.entry.name)}
        <button type="button" role="menuitem" onclick={() => runContextAction('extract')}
          >{t($language, '解压')}</button
        >
      {/if}
      {#if activeRoot?.kind !== 'remote'}
        <button type="button" role="menuitem" onclick={() => runContextAction('share')}
          >{t($language, '分享')}</button
        >
      {/if}
      {#if !currentReadOnly && remoteCan('rename') && menuTargets.length === 1}
        <button type="button" role="menuitem" onclick={() => runContextAction('rename')}
          >{t($language, '重命名')}</button
        >
      {/if}
      {#if remoteCan('copy')}
        <button type="button" role="menuitem" onclick={() => runContextAction('copy-to')}
          >{t($language, '复制到…')}</button
        >
      {/if}
      {#if !currentReadOnly && remoteCan('move')}
        <button type="button" role="menuitem" onclick={() => runContextAction('move-to')}
          >{t($language, '移动到…')}</button
        >
      {/if}
      {#if !currentReadOnly && remoteCan('delete')}
        <div class="menu-sep"></div>
        <button
          type="button"
          role="menuitem"
          class="danger"
          onclick={() => runContextAction('delete')}>{t($language, '删除')}</button
        >
      {/if}
    </div>
  {/if}

  {#if searchMenu}
    <button
      type="button"
      class="menu-backdrop"
      aria-label={t($language, '关闭')}
      onclick={() => (searchMenu = null)}
    ></button>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="context-menu search-menu"
      role="menu"
      tabindex="-1"
      style={`left:${searchMenu.x}px;top:${searchMenu.y}px`}
      onclick={(event) => event.stopPropagation()}
    >
      <button
        type="button"
        role="menuitem"
        onclick={() => {
          const result = searchMenu?.result;
          searchMenu = null;
          if (result) openSearchResult(result);
        }}>{t($language, '打开所在位置')}</button
      >
      {#if !searchMenu.result.is_dir}
        <button
          type="button"
          role="menuitem"
          onclick={() => {
            const result = searchMenu?.result;
            searchMenu = null;
            if (result) downloadSearchResult(result);
          }}>{t($language, '下载')}</button
        >
      {/if}
      <button
        type="button"
        role="menuitem"
        onclick={() => {
          searchShareTarget = searchMenu?.result ?? null;
          searchMenu = null;
        }}>{t($language, '分享')}</button
      >
    </div>
  {/if}

  {#if blankMenu}
    <div
      class="context-menu has-submenu"
      class:open-left={blankMenuLeft}
      class:open-up={blankMenuUp}
      bind:this={blankMenuElement}
      role="menu"
      tabindex="-1"
      style={`left:${blankMenu.x}px;top:${blankMenu.y}px`}
      onclick={(event) => event.stopPropagation()}
      onkeydown={menuKeydown}
    >
      {#if !currentReadOnly}
        <button
          type="button"
          role="menuitem"
          disabled={!remoteCan('mkdir')}
          onclick={() => {
            blankMenu = null;
            openMkdir();
          }}>{t($language, '新建目录')}</button
        >
        <button
          type="button"
          role="menuitem"
          disabled={!remoteCan('touch')}
          onclick={() => {
            blankMenu = null;
            nameDialog = {
              mode: 'touch',
              entry: null,
              title: t($language, '新建文件'),
              label: t($language, '文件名称'),
              initialValue: '',
            };
          }}>{t($language, '新建文件')}</button
        >
        <button
          type="button"
          role="menuitem"
          disabled={!remoteCan('uploads')}
          onclick={() => {
            blankMenu = null;
            folderUploadClick();
          }}>{t($language, '上传目录')}</button
        >
        <button
          type="button"
          role="menuitem"
          onclick={() => {
            blankMenu = null;
            dirStatsOpen = true;
          }}>{t($language, '目录概览')}</button
        >
      {/if}
      {#if terminalAvailable}
        <button
          type="button"
          role="menuitem"
          onclick={() => {
            blankMenu = null;
            terminalOpen = true;
            terminalMinimized = false;
          }}>{t($language, '打开终端')}</button
        >
        <div class="menu-sep"></div>
      {/if}
      <div class="menu-item">
        <button type="button" role="menuitem">
          {t($language, '排序方式')} <span class="menu-arrow">›</span>
        </button>
        <div class="submenu" role="menu">
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setSort('name');
            }}
          >
            {t($language, '名称')}
            {sortField === 'name' ? (sortOrder === 'asc' ? '↑' : '↓') : ''}
          </button>
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setSort('size');
            }}
          >
            {t($language, '大小')}
            {sortField === 'size' ? (sortOrder === 'asc' ? '↑' : '↓') : ''}
          </button>
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setSort('modified');
            }}
          >
            {t($language, '修改时间')}
            {sortField === 'modified' ? (sortOrder === 'asc' ? '↑' : '↓') : ''}
          </button>
        </div>
      </div>
      <div class="menu-item">
        <button type="button" role="menuitem">
          {t($language, '视图切换')} <span class="menu-arrow">›</span>
        </button>
        <div class="submenu" role="menu">
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setView('list');
            }}>{t($language, '列表')}</button
          >
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setView('thumb');
            }}>{t($language, '缩略图')}</button
          >
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setView('tile');
            }}>{t($language, '平铺')}</button
          >
        </div>
      </div>
    </div>
  {/if}

  {#if confirmState}
    <ConfirmDialog
      open
      title={confirmState.title}
      message={confirmState.message}
      items={confirmState.items}
      confirmLabel={t($language, '删除')}
      danger
      requireText={confirmState.requireText}
      onConfirm={(value) => {
        const action = confirmState?.action;
        confirmState = null;
        action?.(value);
      }}
      onCancel={() => (confirmState = null)}
    />
  {/if}

    <RemoteMountDialog
      open={mountDialogOpen}
      {csrf}
      onClose={() => (mountDialogOpen = false)}
      onMounted={(rootId) => {
        void refreshRoots();
        if (rootId !== null) {
          appView = 'files';
          activeRootId = rootId;
          void loadList(rootId, '');
        }
      }}
    />

  {#if preview && activeRootId !== null}
    <PreviewModal
      root={activeRootId}
      {currentPath}
      items={preview.items}
      index={preview.index}
      {csrf}
      onClose={() => {
        const entry = preview ? preview.items[preview.index] : undefined;
        if (entry && activeRootId !== null) {
          cancelThumbScope(activeRootId, joinPath(currentPath, entry.name), 'file');
        }
        preview = null;
      }}
      onIndexChange={(index) => {
        const previous = preview ? preview.items[preview.index] : undefined;
        if (previous && activeRootId !== null) {
          cancelThumbScope(activeRootId, joinPath(currentPath, previous.name), 'file');
        }
        preview = preview ? { ...preview, index } : null;
      }}
      previewEdge={systemFeatures.preview_edge ?? 2048}
      onEditSource={(entry) => {
        preview = null;
        void openEditor(entry);
      }}
    />
  {/if}

  {#if dirStatsOpen && activeRootId !== null}
    <DirStatsDialog
      rootId={activeRootId}
      currentPath={currentPath}
      {entries}
      {dirSizes}
      onClose={() => (dirStatsOpen = false)}
    />
  {/if}

  {#if nameDialog}
    <NameDialog
      open
      title={nameDialog.title}
      label={nameDialog.label}
      initialValue={nameDialog.initialValue}
      confirmLabel={nameDialog.mode === 'mkdir'
        ? t($language, '创建')
        : nameDialog.mode === 'archive'
          ? t($language, '打包')
          : t($language, '重命名')}
      onConfirm={(value) => void submitName(value)}
      onClose={() => (nameDialog = null)}
    />
  {/if}

  {#if transfer && activeRootId !== null}
    <TransferDialog
      open
      mode={transfer.mode}
      count={transfer.entries.length}
      {roots}
      initialRoot={activeRootId}
      initialPath={currentPath}
      onConfirm={(rootId, path) => void submitTransfer(rootId, path)}
      onClose={() => (transfer = null)}
    />
  {/if}

  {#if (terminalOpen || terminalMinimized) && activeRootId !== null}
    <TerminalPanel
      root={activeRootId}
      userId={user?.id ?? 0}
      cwd={currentPath}
      {csrf}
      minimized={terminalMinimized}
      visible={terminalOpen}
      onMinimize={() => (terminalMinimized = true)}
      onClose={() => {
        terminalOpen = false;
        terminalMinimized = false;
      }}
      onRefresh={() => {
        if (activeRootId !== null) void loadList(activeRootId, currentPath);
      }}
    />
  {/if}

  {#if searchShareTarget}
    <ShareDialog
      root={searchShareTarget.rootId}
      paths={[searchShareTarget.path]}
      name={searchShareTarget.name}
      isDir={searchShareTarget.is_dir}
      {csrf}
      onClose={() => (searchShareTarget = null)}
    />
  {/if}

  {#if shareTargets && shareTargets.length > 0 && activeRootId !== null}
    <ShareDialog
      root={activeRootId}
      paths={shareTargets.map((entry) => joinPath(currentPath, entry.name))}
      name={shareTargets.length === 1
        ? (shareTargets[0]?.name ?? '')
        : t($language, '分享 {count} 个文件', { count: shareTargets.length })}
      isDir={shareTargets.length === 1 ? (shareTargets[0]?.is_dir ?? false) : false}
      {csrf}
      onClose={() => (shareTargets = null)}
    />
  {/if}

  {#if searchShare}
    <ShareDialog
      root={searchShare.rootId}
      paths={searchShare.paths}
      name={searchShare.name}
      isDir={searchShare.isDir}
      {csrf}
      onClose={() => (searchShare = null)}
    />
  {/if}

  {#if marquee}
    <div
      class="marquee"
      style={`left:${Math.min(marquee.x1, marquee.x2)}px;top:${Math.min(
        marquee.y1,
        marquee.y2,
      )}px;width:${Math.abs(marquee.x2 - marquee.x1)}px;height:${Math.abs(
        marquee.y2 - marquee.y1,
      )}px`}
    ></div>
  {/if}

  {#if editorOpen}
    <div class="editor-overlay" role="presentation" onclick={() => closeEditor()}>
      <dialog
        open
        class="editor-panel"
        aria-labelledby="editor-title"
        onclick={(event) => event.stopPropagation()}
      >
        <header class="editor-header">
          <strong id="editor-title">{editorName}</strong>
          {#if editorReadOnly}
            <span
              class="editor-readonly"
              title={editorReadOnlyReason === 'policy'
                ? t($language, '目录已设为只读')
                : editorReadOnlyReason === 'directory'
                  ? t($language, '所在目录不可写（原子保存需要目录写权限）')
                  : editorReadOnlyReason === 'lossy'
                    ? t($language, '编码无法完整解码，只能只读')
                    : t($language, '文件本身不可写')}
              >{t($language, '只读')}</span
            >
          {/if}
          {#if editorPeerEditing}
            <span class="editor-peer" role="status">
              {t($language, '另一个标签页正在编辑')}
            </span>
          {/if}
          <div class="editor-actions">
            {#if isMarkdown(editorName)}
              <button type="button" class="ghost" onclick={() => toggleEditorPreview()}>
                {editorPreview ? t($language, '编辑') : t($language, '预览')}
              </button>
            {/if}
            <button
              type="button"
              class="icon-button"
              aria-label={t($language, '关闭编辑器')}
              onclick={() => closeEditor()}
            >
              <Icon name="close" size={18} />
            </button>
          </div>
        </header>

        {#if editorPreview}
          {#if editorPreviewStale}
            <div class="preview-stale" role="status">
              <span>{t($language, '文档较大，已暂停实时预览')}</span>
              <button type="button" class="ghost" onclick={refreshEditorPreview}>
                {t($language, '刷新预览')}
              </button>
            </div>
          {/if}
          <div
            class="markdown-preview"
            bind:this={previewElement}
          >
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            {@html previewHtml}
          </div>
        {:else}
          <div class="editor-body">
            <CodeEditor
              value={editorContent}
              filename={editorName}
              readonly={editorReadOnly}
              onchange={(next) => (editorContent = next)}
            />
          </div>
        {/if}

        {#if editorError}
          <p class="editor-error" role="alert">{editorError}</p>
        {/if}

        <footer class="editor-footer">
          <span>
            {#if editorReadOnly}
              {t($language, '只读')} ·
            {:else}
              {editorDirty ? t($language, '有未保存修改') : t($language, '已保存')} ·
            {/if}
            <select
              class="editor-encoding"
              value={editorEncoding}
              disabled={editorSaving}
              onchange={(event) => changeEditorEncoding(event.currentTarget.value)}
            >
              <option value="auto">{t($language, '自动检测')}</option>
              <option value="utf-8">UTF-8</option>
              <option value="gbk">GBK / GB18030</option>
              <option value="big5">Big5</option>
              <option value="shift_jis">Shift_JIS</option>
              <option value="euc-kr">EUC-KR</option>
              <option value="windows-1252">Windows-1252</option>
              <option value="utf-16le">UTF-16LE</option>
              <option value="utf-16be">UTF-16BE</option>
            </select>
            · {editorContent.length} chars
            {#if editorLineEnding === 'mixed'}
              · {t($language, '原文件混合换行，保存后统一为 LF')}
            {/if}
          </span>
          <div>
            <button type="button" class="ghost" onclick={() => closeEditor()}
              >{editorReadOnly ? t($language, '关闭') : t($language, '取消')}</button
            >
            {#if !editorReadOnly}
              <button
                type="button"
                class="primary"
                disabled={editorSaving}
                onclick={() => void saveEditor()}
              >
                {editorSaving ? t($language, '保存中…') : t($language, '保存')}
              </button>
            {/if}
          </div>
        </footer>
      </dialog>
    </div>
  {/if}
{/if}

<AudioPlayer hidden={preview !== null} />

{#if properties}
  <PropertiesDialog
    data={properties.data}
    loading={properties.loading}
    error={properties.error}
    onClose={() => (properties = null)}
  />
{/if}

<style>
  .boot-screen {
    display: flex;
    min-height: 100dvh;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    background: var(--bg);
  }

  .boot-label {
    margin: 0;
    color: var(--text-2);
    font-size: 13.5px;
  }

  .boot-hint {
    margin: 0;
    color: var(--text-2);
    font-size: 12.5px;
    opacity: 0.8;
  }

  .boot-mark {
    display: grid;
    width: 36px;
    height: 36px;
    place-items: center;
    border-radius: 10px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 20px;
    font-weight: 600;
  }

  .app {
    display: flex;
    height: 100dvh;
    flex-direction: column;
    background: var(--surface);
  }

  .workspace {
    display: grid;
    min-height: 0;
    flex: 1;
    overflow: hidden;
    grid-template-columns: var(--sidebar-width) minmax(0, 1fr);
  }

  .sidebar {
    position: sticky;
    top: 0;
    display: flex;
    height: 100%;
    min-height: 0;
    flex-direction: column;
    overflow-y: auto;
    padding: 12px;
    background: var(--surface);
  }

  .brand-mark img {
    width: 100%;
    height: 100%;
    border-radius: inherit;
    object-fit: cover;
  }

  .brand-mark {
    display: grid;
    width: 24px;
    height: 24px;
    place-items: center;
    border-radius: 7px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 14.5px;
    font-weight: 600;
  }

  .brand-mark.has-icon {
    background: transparent;
  }

  .brand-name {
    min-width: 0;
  }

  .topbar-brand {
    display: flex;
    width: calc(var(--sidebar-width) - 12px);
    flex: 0 0 auto;
    align-items: center;
    gap: 10px;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.02em;
  }

  .top-avatar-button img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .top-avatar-button {
    display: grid;
    width: 34px;
    height: 34px;
    overflow: hidden;
    flex: 0 0 auto;
    place-items: center;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 50%;
    box-sizing: border-box;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: 13.5px;
    font-weight: 600;
  }

  .upload-top,
  .upload-primary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border: 0;
    border-radius: var(--radius);
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 500;
    white-space: nowrap;
  }

  .upload-top {
    height: 36px;
    padding: 0 14px;
  }

  .upload-primary {
    height: 36px;
    margin-top: 8px;
    padding: 0 16px;
  }

  .upload-top:hover:not(:disabled),
  .upload-primary:hover:not(:disabled) {
    filter: brightness(0.96);
  }

  .upload-top:disabled,
  .upload-primary:disabled {
    opacity: 0.5;
  }

  .upload-top.inactive,
  .upload-primary.inactive {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .upload-top.inactive:hover,
  .upload-primary.inactive:hover {
    filter: none;
  }

  .nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 14px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 36px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-2);
    font-size: 15px;
    font-weight: 400;
    text-align: left;
  }

  .nav-item :global(svg) {
    flex: 0 0 auto;
    stroke-width: 2.1;
  }

  .nav-label {
    overflow: hidden;
    flex: 0 1 auto;
    min-width: 0;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .nav-readonly {
    flex: 0 0 auto;
    padding: 1px 5px;
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--text-2);
    font-size: 11px;
    font-weight: 500;
    line-height: 1.4;
    white-space: nowrap;
  }

  .nav-item:hover:not(:disabled) {
    background: var(--hover);
  }

  .nav-item.active {
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: 700;
  }

  .nav-item.active .nav-label {
    font-weight: 700;
  }

  .nav-item:disabled {
    opacity: 0.45;
  }

  .nav-add {
    color: var(--text-2);
  }

  .nav-shared {
    display: inline-flex;
    flex: 0 0 auto;
    color: var(--text-2);
  }

  .shared-mark {
    display: inline-flex;
    margin-left: 4px;
    color: var(--text-2);
    vertical-align: middle;
  }

  .nav-state {
    flex: 0 0 auto;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--danger);
  }

  .stale-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--warning, #d9a441);
  }

  .nav-add:hover:not(:disabled) {
    color: var(--accent-text);
  }

  .nav-empty {
    padding: 8px 10px;
    color: var(--text-2);
    font-size: 13.5px;
  }

  .nav-sep {
    height: 1px;
    margin: 8px 10px;
    background: var(--border);
  }

  .sidebar-spacer {
    flex: 1;
  }

  .quota-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .quota-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .quota-label {
    display: flex;
    justify-content: space-between;
    color: var(--text-2);
    font-size: 12.5px;
    font-weight: 600;
  }

  .disk-name {
    overflow: hidden;
    min-width: 0;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .quota-value {
    flex: 0 0 auto;
    color: var(--accent-text);
    font-variant-numeric: tabular-nums;
    font-weight: 700;
  }

  .quota-row:last-child .quota-value {
    color: var(--text);
  }

  .quota-track {
    height: 8px;
    overflow: hidden;
    border-radius: 99px;
    background: var(--border);
  }

  .quota-fill {
    width: 0;
    height: 100%;
    background: var(--accent);
  }

  .app-version {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 2px 0;
    color: var(--text-2);
    font-size: 11.5px;
    text-align: left;
  }

  .version-link {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    margin: -6px 0 -6px -4px;
    border-radius: 6px;
    color: var(--text-2);
    text-decoration: none;
  }

  .version-link:hover {
    color: var(--text);
    background: var(--surface-2);
  }

  .version-update {
    color: var(--accent);
    text-decoration: none;
    font-weight: 500;
  }

  .version-update:hover {
    text-decoration: underline;
  }

  .quota-fill.disk {
    background: var(--file-video);
  }

  .main {
    display: flex;
    min-width: 0;
    min-height: 0;
    flex-direction: column;
    overflow: hidden;
    background: var(--surface);
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: 12px;
    height: calc(var(--topbar-height) + env(safe-area-inset-top, 0px));
    padding: env(safe-area-inset-top, 0px) 20px 0;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .search {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 8px;
    max-width: 640px;
    height: 40px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    background: var(--surface-2);
    color: var(--text-2);
  }

  .search-submit {
    display: grid;
    flex: 0 0 auto;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    place-items: center;
  }

  .search:focus-within {
    background: var(--surface);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
  }

  .search input::placeholder {
    color: var(--text-2);
    opacity: 0.8;
  }

  .sort-button {
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
  }

  .sort-button:hover {
    color: var(--text);
  }

  .file-table th.active {
    color: var(--text);
  }

  .file-table th.active .sort-button {
    font-weight: 700;
  }

  .sort-control {
    position: relative;
  }

  .sort-trigger {
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-2);
    font-size: 13px;
    white-space: nowrap;
  }

  .sort-popup {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 40;
    display: grid;
    min-width: 140px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .sort-popup button {
    padding: 7px 10px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    text-align: left;
  }

  .sort-popup button:hover,
  .sort-popup button.active {
    background: var(--hover);
  }

  .top-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
  }

  .segmented {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .segmented button {
    display: grid;
    width: 32px;
    height: 30px;
    place-items: center;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-2);
  }

  .segmented button.active {
    background: var(--surface);
    box-shadow: var(--shadow-1);
    color: var(--text);
  }

  .icon-button {
    display: grid;
    width: 36px;
    height: 36px;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
  }

  .icon-button:hover:not(:disabled) {
    background: var(--hover);
  }

  .mobile-menu {
    display: none;
  }

  .pathbar-actions {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    gap: 2px;
  }

  .selection-count {
    padding: 0 6px;
    color: var(--accent-text);
    font-size: 12.5px;
    white-space: nowrap;
  }

  .pathbar-actions .danger:hover {
    color: var(--danger);
  }

  .pathbar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    padding: 0 20px;
    background: transparent;
  }

  .path-static {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    gap: 8px;
    color: var(--text-2);
  }

  .path {
    display: flex;
    min-width: 0;
    flex: 1;
    align-items: center;
    gap: 8px;
    overflow: hidden;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 14px;
    white-space: nowrap;
  }

  .path-root {
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 0 1 auto;
  }

  .path > .path-link:not(.path-root) {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 0 1 auto;
  }

  .path-more {
    position: relative;
    flex: 0 0 auto;
  }

  .path-more-button {
    padding: 0 4px;
    border-radius: 4px;
  }

  .path-more-button:hover,
  .path-more-button[aria-expanded='true'] {
    background: var(--surface-2);
    color: var(--text);
  }

  .path-menu {
    position: absolute;
    z-index: 20;
    top: calc(100% + 6px);
    left: 0;
    display: flex;
    min-width: 160px;
    max-width: 320px;
    max-height: 280px;
    flex-direction: column;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    box-shadow: var(--shadow);
  }

  .path-menu button {
    padding: 6px 10px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 13px;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .path-menu button:hover {
    background: var(--surface-2);
  }

  .search-path {
    gap: 6px;
    font-family: var(--font-ui);
    font-size: 13px;
  }

  .search-query {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
    font-weight: 600;
  }

  .search-filter-tag {
    flex: 0 0 auto;
    padding: 1px 7px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 12px;
  }

  .search-state {
    color: var(--text-2);
    white-space: nowrap;
  }

  .path-clear {
    flex: 0 0 auto;
    margin-left: auto;
  }

  .sep {
    opacity: 0.5;
  }

  .path-link {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font: inherit;
    white-space: nowrap;
  }

  .path-link:hover {
    color: var(--text);
  }

  .selection-size {
    flex: 0 0 auto;
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 12px;
  }

  .selection-kind {
    flex: 0 0 auto;
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 11px;
  }

  @media (max-width: 760px) {
    .selection-kind {
      display: none;
    }
  }

  .readonly-tag {
    flex: 0 0 auto;
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text-2);
    font-family: var(--font-ui);
    font-size: 12px;
  }

  .content {
    display: block;
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--content-pad);
  }

  .content-drag {
    min-height: 100%;
  }

  .content-drag.file-area {
    user-select: none;
  }

  .focus-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 12px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 13px;
  }

  .browser {
    width: 100%;
  }
  .browser.refreshing {
    pointer-events: none;
    opacity: 0.55;
    transition: opacity 120ms linear;
  }

  .file-table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: 15px;
    font-weight: 500;
  }

  .file-table th {
    height: 36px;
    padding: 0 12px;
    color: var(--text-2);
    font-size: 12.5px;
    font-weight: 500;
    text-align: left;
  }

  .file-table td {
    height: var(--row-height);
    padding: 0 12px;
    border-top: 1px solid var(--border);
    color: var(--text);
  }

  .file-table tbody tr {
    cursor: default;
  }

  .file-table tbody tr:hover {
    background: var(--hover);
  }

  .file-table tbody tr.virtual-spacer:hover {
    background: transparent;
  }

  .file-table tbody tr.virtual-spacer td {
    padding: 0;
    border: 0;
  }

  .file-table tbody tr.selected {
    background: var(--accent-soft);
  }

  .file-table tbody tr.drag-source,
  .file-card.drag-source,
  .file-tile.drag-source {
    opacity: 0.45;
  }

  .file-table tbody tr.drop-target,
  .file-card.drop-target,
  .file-tile.drop-target {
    background: var(--accent-soft);
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .file-table tbody tr.drop-invalid,
  .file-card.drop-invalid,
  .file-tile.drop-invalid {
    outline: 2px dashed var(--danger);
    outline-offset: -2px;
    cursor: not-allowed;
  }

  .file-table tbody tr.marquee-selected,
  .file-card.marquee-selected,
  .file-tile.marquee-selected {
    background: var(--hover);
  }

  .file-table .check {
    width: 36px;
    padding-right: 0;
  }

  .file-table .check input,
  .file-card input {
    accent-color: var(--accent);
  }

  .file-table .name {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .file-icon {
    display: inline-flex;
    flex: 0 0 auto;
    color: var(--text-2);
  }

  .file-name {
    display: flex;
    min-width: 0;
    overflow: hidden;
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
  }

  .name-stem {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .symlink-mark {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    margin-right: 4px;
    color: var(--text-2);
  }

  .symlink-mark.bad {
    color: var(--danger);
  }

  .name-ext {
    flex: 0 0 auto;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .file-icon :global(svg) {
    stroke-width: 2;
  }

  .file-table .size {
    width: 96px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
    font-weight: 500;
    text-align: right;
  }

  .size-stale {
    opacity: 0.55;
    transition: opacity 0.2s ease;
  }

  .file-table .modified {
    width: 160px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }

  .file-table .actions {
    width: 44px;
    text-align: right;
  }

  .row-action {
    display: inline-grid;
    width: 28px;
    height: 28px;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
    opacity: 0;
  }

  .file-table tbody tr:hover .row-action,
  .row-action:focus-visible {
    opacity: 1;
  }

  .row-action:hover:not(:disabled) {
    background: var(--hover);
    color: var(--danger);
  }

  .row-action:disabled {
    opacity: 0;
  }

  .search-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
    color: var(--text-2);
    font-size: 14px;
  }

  .search-header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .search-header .ghost.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }

  .search-filters {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    margin: -2px 0 14px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .search-filters label {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-size: 12.5px;
  }

  .search-filters input,
  .search-filters select {
    width: 96px;
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
  }

  .search-filters select {
    width: auto;
  }

  .search-header .ghost {
    height: 30px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-2);
  }

  .search-results {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .search-more {
    display: flex;
    justify-content: center;
    padding: 12px 0;
  }

  .search-more button {
    height: 32px;
    padding: 0 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-2);
  }

  .search-result {
    display: grid;
    grid-template-columns: 24px minmax(0, 1.2fr) minmax(0, 1fr) 72px;
    align-items: center;
    gap: 10px;
    min-height: 42px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
  }

  .search-result:hover {
    background: var(--hover);
  }

  .search-result > :first-child {
    color: var(--text-2);
  }

  .search-path {
    overflow: hidden;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 12.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .search-size {
    color: var(--text-2);
    font-size: 12.5px;
    text-align: right;
  }

  .search-count {
    margin-left: 4px;
    opacity: 0.7;
  }

  .search-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
  }

  .sort-button {
    height: 28px;
    padding: 0 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-2);
    font-size: 12px;
  }

  .sort-button.active,
  .sort-button:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--text);
  }

  .sort-button:disabled {
    opacity: 0.5;
  }

  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 54;
    border: 0;
    background: transparent;
  }

  .search-menu {
    z-index: 56;
  }

  .search-more {
    display: grid;
    width: 30px;
    height: 30px;
    flex: 0 0 auto;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
  }

  .search-card .search-more {
    position: absolute;
    top: 4px;
    right: 4px;
    z-index: 2;
  }

  .search-row {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 6px;
  }

  .search-row.selected,
  .search-card.selected {
    background: var(--surface-2);
    border-radius: var(--radius-sm);
  }

  .search-row .search-result {
    flex: 1;
  }

  .search-thumb {
    position: relative;
    display: grid;
    width: 100%;
    aspect-ratio: 1;
    place-items: center;
    overflow: hidden;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text-2);
  }

  .search-thumb.small {
    display: grid;
    width: 24px;
    height: 24px;
    aspect-ratio: auto;
    background: transparent;
  }

  .search-thumb .thumb-image {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .search-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 10px;
    padding: 10px;
  }

  .search-card {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .search-card-open {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
  }

  .search-card-open .file-name {
    margin-top: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .search-check {
    position: absolute;
    z-index: 1;
    top: 6px;
    left: 6px;
    width: 16px;
    height: 16px;
  }

  .files-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    min-height: 50vh;
    color: var(--text-2);
    font-size: 14.5px;
    text-align: center;
  }

  .slow-network-hint {
    color: var(--text-2);
    font-size: 12.5px;
    opacity: 0.8;
  }

  .retry-button {
    height: 32px;
    padding: 0 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
  }

  .tile-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 10px;
    padding-top: var(--virtual-pad-top, 0);
    padding-bottom: var(--virtual-pad-bottom, 0);
  }

  .file-tile {
    position: relative;
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    cursor: default;
  }

  .file-tile:hover {
    background: var(--hover);
    box-shadow: var(--shadow-1);
  }

  .file-tile.selected {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .file-tile .card-check {
    top: 6px;
    left: 6px;
  }

  .tile-icon {
    position: relative;
    display: grid;
    width: 64px;
    height: 64px;
    flex: 0 0 64px;
    overflow: hidden;
    place-items: center;
    border-radius: var(--radius);
    background: var(--surface-2);
    color: var(--text-2);
  }

  .tile-info {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    gap: 3px;
    overflow: hidden;
  }

  .tile-name {
    display: flex;
    width: 100%;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
    line-height: 1.35;
    text-align: left;
    white-space: nowrap;
  }

  .tile-meta {
    overflow: hidden;
    color: var(--text-2);
    font-size: 12.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 16px;
    padding-top: var(--virtual-pad-top, 0);
    padding-bottom: var(--virtual-pad-bottom, 0);
  }

  .file-card {
    position: relative;
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 4px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    cursor: default;
  }

  .file-card:hover {
    background: var(--hover);
    box-shadow: var(--shadow-1);
  }

  .file-card:hover .card-icon,
  .file-tile:hover .tile-icon,
  .file-card.marquee-selected .card-icon,
  .file-tile.marquee-selected .tile-icon {
    background: var(--surface);
  }

  .file-card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .card-check {
    position: absolute;
    top: 10px;
    left: 10px;
    z-index: 1;
    opacity: 0;
  }

  .file-card:hover .card-check,
  .file-card.selected .card-check,
  .card-check:focus-visible {
    opacity: 1;
  }

  .card-icon {
    position: relative;
    display: grid;
    height: 96px;
    overflow: hidden;
    place-items: center;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text-2);
  }

  .thumb-image {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .card-name {
    display: flex;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
    line-height: 1.4;
    white-space: nowrap;
  }

  .card-meta {
    color: var(--text-2);
    font-size: 12.5px;
  }

  .upload-input {
    display: none;
  }

  :global(.confirm-panel),
  :global(.notice-dialog),
  :global(.share-panel),
  :global(.assign-dialog),
  :global(.user-dialog),
  :global(.path-dialog),
  :global(.edit-dialog),
  :global(.quota-dialog),
  :global(.account-dialog) {
    max-height: min(85dvh, 720px);
    overflow-y: auto;
  }

  :global(.dialog-overlay),
  :global(.notice-overlay),
  :global(.share-overlay),
  :global(.confirm-overlay) {
    padding-bottom: max(24px, env(safe-area-inset-bottom, 24px));
  }

  @media (pointer: coarse) {
    :global(.confirm-panel button),
    :global(.notice-dialog button) {
      min-height: 44px;
    }
  }

  @media (max-width: 640px) {
    :global(.confirm-overlay),
    :global(.notice-overlay),
    :global(.share-overlay),
    :global(.dialog-overlay),
    :global(.qr-overlay) {
      align-items: end;
      padding: 0;
    }

    :global(.confirm-panel),
    :global(.notice-dialog),
    :global(.share-panel),
    :global(.assign-dialog),
    :global(.user-dialog),
    :global(.path-dialog),
    :global(.edit-dialog),
    :global(.quota-dialog),
    :global(.account-dialog),
    :global(.qr-dialog) {
      width: 100%;
      max-height: 88dvh;
      border-radius: 16px 16px 0 0;
      padding-bottom: max(20px, env(safe-area-inset-bottom, 20px));
    }

    :global(.confirm-panel button),
    :global(.notice-dialog button),
    :global(.share-panel button),
    :global(.qr-dialog button) {
      min-height: 44px;
    }
  }

  .card-menu {
    position: absolute;
    top: 8px;
    right: 8px;
    z-index: 2;
    display: grid;
    width: 32px;
    height: 32px;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: color-mix(in srgb, var(--surface) 82%, transparent);
    color: var(--text-2);
  }

  @media (hover: hover) {
    .file-card .card-menu,
    .file-tile .card-menu {
      opacity: 0;
    }

    .file-card:hover .card-menu,
    .file-tile:hover .card-menu,
    .card-menu:focus-visible {
      opacity: 1;
    }
  }

  :global(textarea),
  :global(.markdown-preview),
  :global(.dialog-overlay),
  :global(.notice-overlay),
  :global(.share-overlay),
  :global(.confirm-overlay) {
    overscroll-behavior: contain;
  }

  @media (pointer: coarse) {
    :global(input),
    :global(textarea),
    :global(select) {
      font-size: 16px;
    }

    :global(button),
    :global([role='button']) {
      touch-action: manipulation;
    }

    .row-action {
      width: 40px;
      height: 40px;
    }
  }

  @media (max-width: 640px) {
    .context-menu {
      left: 12px !important;
      right: 12px;
      top: auto !important;
      bottom: calc(12px + env(safe-area-inset-bottom, 0px)) !important;
      width: auto;
      max-height: 70dvh;
      border-radius: 16px;
    }

    .context-menu button[role='menuitem'] {
      min-height: 44px;
      font-size: 15px;
    }
  }

  .context-menu {
    position: fixed;
    z-index: 55;
    display: flex;
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    min-width: 160px;
    flex-direction: column;
    padding: 5px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .context-menu.has-submenu {
    overflow: visible;
    max-height: none;
  }

  .context-menu button {
    height: 32px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-size: 13.5px;
    text-align: left;
  }

  .context-menu button:hover {
    background: var(--hover);
  }

  .context-menu button.danger {
    color: var(--danger);
  }

  .menu-item {
    position: relative;
  }

  .menu-item > button {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
  }

  .menu-arrow {
    margin-left: auto;
    color: var(--text-2);
  }

  .context-menu.open-left .submenu {
    right: calc(100% - 2px);
    left: auto;
  }

  .context-menu.open-up .submenu {
    top: auto;
    bottom: 0;
  }

  .submenu {
    position: absolute;
    top: 0;
    left: calc(100% - 2px);
    z-index: 56;
    display: none;
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    min-width: 150px;
    flex-direction: column;
    padding: 5px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .menu-item:hover > .submenu,
  .menu-item:focus-within > .submenu {
    display: flex;
  }

  .submenu button {
    height: 32px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-size: 13.5px;
    text-align: left;
  }

  .submenu button:hover {
    background: var(--hover);
  }

  .menu-sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }

  .marquee {
    position: fixed;
    z-index: 40;
    border: 1px solid var(--accent);
    background: var(--accent-soft);
    opacity: 0.55;
    pointer-events: none;
  }

  .view-hidden {
    display: none;
  }

  .editor-overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }

  .editor-panel {
    display: flex;
    width: min(900px, 100%);
    height: min(80vh, 720px);
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .editor-header,
  .editor-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-height: 52px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border);
  }

  .editor-footer {
    border-top: 1px solid var(--border);
    border-bottom: 0;
    color: var(--text-2);
    font-size: 12.5px;
  }

  .editor-header strong {
    overflow: hidden;
    font-size: 16px;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .editor-peer {
    flex: none;
    padding: 2px 8px;
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: 11.5px;
  }

  .editor-actions,
  .editor-footer > div {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .editor-body {
    display: flex;
    min-height: 0;
    flex: 1;
  }

  .editor-readonly {
    flex: 0 0 auto;
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--text-2);
    font-size: 11.5px;
    font-weight: 500;
    white-space: nowrap;
  }

  .preview-stale {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 20px;
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
    font-size: 13px;
  }

  .markdown-preview {
    flex: 1;
    overflow: auto;
    padding: 20px;
    color: var(--text);
    font-size: 16px;
    line-height: 1.7;
  }

  .markdown-preview :global(h1),
  .markdown-preview :global(h2),
  .markdown-preview :global(h3) {
    color: var(--text);
    line-height: 1.4;
  }

  .markdown-preview :global(code),
  .markdown-preview :global(pre) {
    background: var(--surface-2);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
  }

  .markdown-preview :global(pre) {
    overflow: auto;
    padding: 12px;
  }

  .markdown-preview :global(a) {
    color: var(--accent-text);
  }

  .markdown-preview :global(p),
  .markdown-preview :global(ul),
  .markdown-preview :global(ol),
  .markdown-preview :global(blockquote),
  .markdown-preview :global(table),
  .markdown-preview :global(pre) {
    margin: 0 0 14px;
  }

  .markdown-preview :global(h4),
  .markdown-preview :global(h5),
  .markdown-preview :global(h6) {
    color: var(--text);
    line-height: 1.4;
  }

  .markdown-preview :global(ul),
  .markdown-preview :global(ol) {
    padding-left: 22px;
  }

  .markdown-preview :global(li + li) {
    margin-top: 4px;
  }

  .markdown-preview :global(li > input[type='checkbox']) {
    margin-right: 6px;
  }

  .markdown-preview :global(blockquote) {
    margin-left: 0;
    padding: 2px 0 2px 12px;
    border-left: 3px solid var(--border);
    color: var(--text-2);
  }

  .markdown-preview :global(hr) {
    margin: 18px 0;
    border: 0;
    border-top: 1px solid var(--border);
  }

  .markdown-preview :global(table) {
    display: block;
    max-width: 100%;
    overflow-x: auto;
    border-collapse: collapse;
  }

  .markdown-preview :global(th),
  .markdown-preview :global(td) {
    padding: 6px 10px;
    border: 1px solid var(--border);
    text-align: left;
    white-space: nowrap;
  }

  .markdown-preview :global(thead th) {
    background: var(--surface-2);
    font-weight: 600;
  }

  .markdown-preview :global(img) {
    max-width: 100%;
    height: auto;
  }

  .markdown-preview :global(del) {
    color: var(--text-2);
  }

  .markdown-preview :global(code) {
    padding: 1px 5px;
  }

  .markdown-preview :global(pre code) {
    padding: 0;
    background: transparent;
  }

  .markdown-preview :global(.token.comment),
  .markdown-preview :global(.token.prolog),
  .markdown-preview :global(.token.doctype),
  .markdown-preview :global(.token.cdata) {
    color: var(--text-2);
  }

  .markdown-preview :global(.token.keyword),
  .markdown-preview :global(.token.selector),
  .markdown-preview :global(.token.important) {
    color: var(--file-image);
  }

  .markdown-preview :global(.token.string),
  .markdown-preview :global(.token.attr-value) {
    color: var(--file-sheet);
  }

  .markdown-preview :global(.token.number),
  .markdown-preview :global(.token.boolean) {
    color: var(--file-video);
  }

  .markdown-preview :global(.token.function),
  .markdown-preview :global(.token.class-name) {
    color: var(--file-doc);
  }

  .markdown-preview :global(.token.operator),
  .markdown-preview :global(.token.punctuation) {
    color: var(--text-2);
  }

  .editor-encoding {
    height: 26px;
    padding: 0 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-2);
    font-size: 12px;
  }

  .editor-footer .ghost,
  .editor-actions .ghost {
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-2);
  }

  .editor-footer .primary {
    height: 32px;
    padding: 0 14px;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--on-accent);
  }

  .editor-error {
    margin: 0;
    padding: 8px 14px;
    color: var(--danger);
    font-size: 13.5px;
  }

  .load-more {
    padding: 18px;
    color: var(--text-2);
    font-size: 13px;
    text-align: center;
  }

  .empty-state {
    display: flex;
    min-height: calc(
      100dvh - var(--topbar-height) - var(--pathbar-height) - 2 * var(--content-pad)
    );
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    margin: auto;
    color: var(--text-2);
    text-align: center;
  }

  .empty-icon {
    display: grid;
    width: 88px;
    height: 88px;
    margin-bottom: 6px;
    place-items: center;
    border: 1px solid var(--border);
    border-radius: 20px;
    background: var(--surface);
    color: var(--text-2);
  }

  h1 {
    margin: 0;
    color: var(--text);
    font-size: 22px;
    font-weight: 500;
    line-height: 1.4;
  }

  p {
    max-width: 420px;
    margin: 0;
    font-size: 14.5px;
  }

  .overlay {
    display: none;
  }

  @media (max-width: 720px) {
    .file-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 10px;
    }

    .card-icon {
      height: 84px;
    }

    .tile-grid {
      grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    }

    .file-table .size,
    .file-table .modified {
      display: none;
    }

    .editor-overlay {
      padding: 0;
    }

    .editor-panel {
      width: 100%;
      height: 100dvh;
      border-radius: 0;
    }
  }

  @media (max-width: 900px) {
    .workspace {
      grid-template-columns: 1fr;
    }

    .sidebar {
      position: fixed;
      inset: 0 auto 0 0;
      z-index: 30;
      width: 280px;
      transform: translateX(-100%);
      transition: transform var(--motion-base) var(--ease);
    }

    .sidebar.open {
      transform: translateX(0);
    }

    .mobile-menu {
      display: grid;
    }

    .overlay {
      position: fixed;
      inset: 0;
      z-index: 20;
      display: block;
      border: 0;
      background: var(--scrim);
    }

    .topbar {
      gap: 8px;
      padding: 0 12px;
    }

    .topbar-brand {
      width: auto;
    }

    .search {
      max-width: none;
    }

    .upload-top {
      padding: 0 10px;
    }

    .upload-top span {
      display: none;
    }

    .pathbar-actions {
      display: flex;
      flex: 0 0 auto;
      align-items: center;
      gap: 2px;
    }

    .selection-count {
      padding: 0 6px;
      color: var(--accent-text);
      font-size: 12.5px;
      white-space: nowrap;
    }

    .pathbar-actions .danger:hover {
      color: var(--danger);
    }

    .pathbar {
      padding: 0 16px;
    }

    .content {
      padding: 16px;
    }
  }

  @media (max-width: 520px) {
    .segmented {
      display: none;
    }

    .topbar {
      gap: 6px;
      padding: 0 8px;
    }

    .topbar-brand .brand-name {
      display: none;
    }

    .search {
      min-width: 0;
      padding: 0 10px;
    }

    .top-actions {
      gap: 4px;
    }
  }
</style>
