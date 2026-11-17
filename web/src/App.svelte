<script lang="ts">
  /* global __APP_VERSION__ */
  import DOMPurify from 'dompurify';
  import { marked } from 'marked';
  import { onMount } from 'svelte';
  import { SvelteMap, SvelteSet } from 'svelte/reactivity';
  import AccountDialog from './lib/components/AccountDialog.svelte';
  import NoticeDialog from './lib/components/NoticeDialog.svelte';
  import ProgressPanel from './lib/components/ProgressPanel.svelte';
  import ToastPanel from './lib/components/ToastPanel.svelte';
  import AdminPage from './lib/components/AdminPage.svelte';
  import AuditPage from './lib/components/AuditPage.svelte';
  import SharesPage from './lib/components/SharesPage.svelte';
  import ConfirmDialog from './lib/components/ConfirmDialog.svelte';
  import FileTypeIcon from './lib/components/FileTypeIcon.svelte';
  import NameDialog from './lib/components/NameDialog.svelte';
  import PreviewModal from './lib/components/PreviewModal.svelte';
  import PropertiesDialog from './lib/components/PropertiesDialog.svelte';
  import PreferencesPage from './lib/components/PreferencesPage.svelte';
  import ShareDialog from './lib/components/ShareDialog.svelte';
  import TerminalPanel from './lib/components/TerminalPanel.svelte';
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
  import { resolveViewPreference } from './lib/viewPref';
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
    type Root,
    type JobInfo,
    type SystemFeatures,
    type TransferJobResult,
    type UserPreferences,
    type SearchEntry,
    type Usage,
    type User,
  } from './lib/api';
  import { applyTheme, resolveTheme, type Theme } from './lib/theme';

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
  let preferences = $state<UserPreferences | null>(null);
  let systemFeatures = $state<SystemFeatures>({ thumbnails: true, editor: true, terminal: true });
  let csrf = $state('');
  let loggingOut = $state(false);
  let roots = $state<Root[]>([]);
  let activeRootId = $state<number | null>(null);
  let currentPath = $state('');
  let currentReadOnly = $state(false);
  let clock = $state(Date.now());
  let entries = $state<Entry[]>([]);
  let selected = $state<string[]>([]);
  let dirSizes = $state<Record<string, { size: number; stale: boolean } | null>>({});
  let dirSizeErrors = $state<Record<string, string | null>>({});
  let selectionAnchor = $state<string | null>(null);
  let filesLoading = $state(false);
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
  let filesError = $state('');
  let uploading = $state(false);
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
  let searchIndexing = $state(false);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  let searchInput = $state<HTMLInputElement | null>(null);
  let searchFiltersOpen = $state(false);
  let searchKind = $state('');
  let searchMinMb = $state('');
  let searchMaxMb = $state('');
  let searchModified = $state('');
  const indexedRoots = new SvelteSet<number>();
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
  let lineNumbersElement = $state<HTMLPreElement | null>(null);
  let usage = $state<Usage | null>(null);
  let marquee = $state<{ x1: number; y1: number; x2: number; y2: number } | null>(null);
  let marqueePreview = $state<string[]>([]);
  let suppressSelectionClear = false;
  let appView = $state<'files' | 'settings' | 'admin' | 'audit' | 'shares'>('files');
  let settingsMounted = $state(false);
  let sharesMounted = $state(false);
  let contextMenu = $state<{ x: number; y: number; entry: Entry } | null>(null);
  let contextMenuElement = $state<HTMLElement | null>(null);
  let contextMenuLeft = $state(false);
  let contextMenuUp = $state(false);
  let blankMenu = $state<{ x: number; y: number } | null>(null);
  let blankMenuElement = $state<HTMLElement | null>(null);
  let blankMenuLeft = $state(false);
  let blankMenuUp = $state(false);
  let terminalOpen = $state(false);
  let transfer = $state<{ mode: 'copy' | 'move'; entries: Entry[] } | null>(null);
  let dragPayload = $state<{ rootId: number; path: string; name: string } | null>(null);
  let nameDialog = $state<{
    mode: 'mkdir' | 'touch' | 'rename';
    entry: Entry | null;
    title: string;
    label: string;
    initialValue: string;
  } | null>(null);
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
    requireText: string | null;
    action: (value: string) => void;
  } | null>(null);
  let editorDirty = $derived(editorContent !== editorOriginalContent);
  let editorLineNumbers = $derived(
    Array.from({ length: editorContent.split('\n').length }, (_, index) => index + 1),
  );
  let previewHtml = $derived(
    editorPreview ? DOMPurify.sanitize(String(marked.parse(editorContent))) : '',
  );

  let activeRoot = $derived(roots.find((root) => root.id === activeRootId) ?? null);
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
        preferences?.terminal_enabled !== false &&
        systemFeatures.terminal
      ) {
        event.preventDefault();
        terminalOpen = !terminalOpen;
        return;
      }
      if (event.key === 'Escape') {
        if (editorOpen) {
          editorOpen = false;
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

    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('resize', updateVirtualRange);
    const autoSaveTimer = window.setInterval(() => {
      if (editorOpen && editorDirty && !editorSaving) {
        void saveEditor(true);
      }
    }, 30_000);
    const clockTimer = window.setInterval(() => (clock = Date.now()), 60_000);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('resize', updateVirtualRange);
      window.clearInterval(autoSaveTimer);
      window.clearInterval(clockTimer);
      if (usagePollTimer) clearTimeout(usagePollTimer);
      if (bootSlowTimer) clearTimeout(bootSlowTimer);
      if (filesSlowTimer) clearTimeout(filesSlowTimer);
    };
  });

  function setTheme(next: Theme): void {
    theme = next;
    applyTheme(theme);
    try {
      localStorage.setItem('filehub.theme', theme);
    } catch (error) {
      console.warn('could not persist theme preference', error);
    }
  }

  function setThemePreference(next: Theme): void {
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
        csrf = auth.csrf_token;
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

  function handleAuth(auth: AuthResponse): void {
    user = auth.user;
    csrf = auth.csrf_token;
    initialized = true;
    loading = false;
    stopBootSlowHint();
    void loadPreferences();
    void loadRoots();
  }

  async function loadPreferences(): Promise<void> {
    try {
      const result = await api.getPreferences();
      systemFeatures = result.system;
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
      applyPreferences(result.preferences);
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '偏好保存失败'));
    }
  }

  async function logout(): Promise<void> {
    if (loggingOut) return;

    loggingOut = true;
    try {
      await api.logout(csrf);
    } catch (error) {
      console.warn('logout request failed', error);
    } finally {
      user = null;
      preferences = null;
      csrf = '';
      roots = [];
      entries = [];
      currentPath = '';
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
        currentReadOnly = false;
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
  async function refreshRoots(): Promise<void> {
    try {
      const result = await api.roots();
      roots = result.roots;
      const next = roots.find((root) => root.id === activeRootId) ?? roots[0] ?? null;
      if (!next) {
        activeRootId = null;
        currentPath = '';
        currentReadOnly = false;
        entries = [];
        filesError = t($language, '当前账号还没有可访问的目录');
        return;
      }
      await loadList(next.id, next.id === activeRootId ? currentPath : '');
    } catch (error) {
      handleFilesError(error);
    }
  }

  async function loadList(rootId: number, path: string): Promise<void> {
    const generation = ++listGeneration;
    listAbort?.abort();
    loadMoreAbort?.abort();
    abortDirSizeRequests();
    dirSizeGeneration += 1;
    const controller = new AbortController();
    listAbort = controller;
    filesLoading = true;
    filesError = '';
    nextCursor = null;
    startFilesSlowHint();
    try {
      const storedView = resolveViewPreference(
        localStorage.getItem('filehub.view'),
        localStorage.getItem(`view:${rootId}:${path}`),
      );
      if (storedView) view = storedView;
      const savedSort = localStorage.getItem(`sort:${rootId}:${path}`);
      if (savedSort) {
        const [field, order] = savedSort.split(':');
        if (field === 'name' || field === 'size' || field === 'modified') {
          sortField = field;
          sortOrder = order === 'desc' ? 'desc' : 'asc';
        }
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
      );
      if (generation !== listGeneration || controller.signal.aborted) return;
      activeRootId = result.root.id;
      currentPath = result.path;
      currentReadOnly = result.read_only;
      entries = result.entries;
      nextCursor = result.next_cursor;
      loadDirectorySizes(entries, result.root.id, result.path, dirSizeGeneration, true);
      selected = [];
      selectionAnchor = null;
      const remembered = resolveViewPreference(
        localStorage.getItem('filehub.view'),
        localStorage.getItem(`view:${result.root.id}:${result.path}`),
      );
      if (remembered) {
        view = remembered;
        try {
          if (!localStorage.getItem('filehub.view')) {
            localStorage.setItem('filehub.view', remembered);
          }
        } catch (error) {
          console.warn('could not persist view preference', error);
        }
      }
      void loadUsage();
    } catch (error) {
      if (isAbortError(error)) return;
      handleFilesError(error);
    } finally {
      if (generation === listGeneration) {
        filesLoading = false;
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

  function abortDirSizeRequests(): void {
    for (const controller of dirSizeControllers) controller.abort();
    dirSizeControllers.clear();
    for (const timer of dirSizeTimers.values()) clearTimeout(timer);
    dirSizeTimers.clear();
    dirSizeAttempts.clear();
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
      void requestDirectorySize(key, rootId, joinPath(path, entry.name), generation);
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

  function handleFilesError(error: unknown): void {
    if (error instanceof ApiError && error.status === 401) {
      user = null;
      csrf = '';
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
    searchActive = false;
    searchQuery = '';
    searchResults = [];
    if (activeRootId !== null) {
      void loadList(activeRootId, path);
    }
  }

  function handleSearchInput(): void {
    if (searchTimer) {
      clearTimeout(searchTimer);
    }
    searchAbort?.abort();
    searchIndexing = false;

    const query = searchQuery.trim();
    if (!query || activeRootId === null) {
      searchActive = false;
      searchResults = [];
      return;
    }

    searchTimer = setTimeout(() => {
      void runSearch(query);
    }, 250);
  }

  async function waitForIndexJob(
    jobId: string,
    token: number,
    query: string,
    signal: AbortSignal,
  ): Promise<void> {
    for (let attempt = 0; attempt < 600; attempt += 1) {
      if (signal.aborted) return;
      try {
        const job: JobInfo = await api.job(jobId, signal);
        if (job.status === 'completed' || job.status === 'failed' || job.status === 'cancelled') {
          return;
        }
      } catch (error) {
        if (isAbortError(error)) return;
        console.warn('background index status failed', error);
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, 1000));
      if (token !== searchToken) return;
    }
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
    const query = searchQuery.trim();
    if (query) void runSearch(query);
  }

  function resetSearchFilters(): void {
    searchKind = '';
    searchMinMb = '';
    searchMaxMb = '';
    searchModified = '';
  }

  async function runSearch(query: string): Promise<void> {
    if (roots.length === 0) return;
    const token = ++searchToken;
    searchAbort?.abort();
    const controller = new AbortController();
    searchAbort = controller;

    const collect = async (): Promise<SearchEntry[]> => {
      const indexing: Promise<void>[] = [];
      const settled = await Promise.allSettled(
        roots.map(async (root) => {
          if (!indexedRoots.has(root.id)) {
            indexedRoots.add(root.id);
            try {
              const job = await api.index(root.id, '', true, csrf);
              indexing.push(waitForIndexJob(job.job_id, token, query, controller.signal));
            } catch (error) {
              console.warn('background index failed', error);
            }
          }
          const result = await api.search(
            root.id,
            query,
            100,
            controller.signal,
            searchFilterParams(),
          );
          return result.entries.map((entry) => ({ ...entry, rootId: root.id }));
        }),
      );
      if (indexing.length > 0) {
        searchIndexing = true;
        void Promise.allSettled(indexing).then(() => {
          if (token !== searchToken) return;
          searchIndexing = false;
          if (searchQuery === query) void runSearch(query);
        });
      }
      return settled.flatMap((result) => (result.status === 'fulfilled' ? result.value : []));
    };

    try {
      const results = await collect();
      if (token !== searchToken || controller.signal.aborted) return;
      searchResults = results.slice(0, 200);
      searchActive = true;
    } catch (error) {
      if (token === searchToken && !isAbortError(error)) handleFilesError(error);
    }
  }

  function openSearchResult(entry: SearchEntry): void {
    const root = entry.rootId ?? activeRootId;
    if (root === null) return;

    searchActive = false;
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
        await loadList(root, parent);
        selected = [entry.name];
        selectionAnchor = entry.name;
      })();
    }
  }

  function exitSearch(): void {
    searchAbort?.abort();
    if (searchTimer) clearTimeout(searchTimer);
    searchActive = false;
    searchQuery = '';
    searchResults = [];
    searchIndexing = false;
    searchFiltersOpen = false;
    resetSearchFilters();
  }

  function navigateToSegment(index: number): void {
    navigate(pathSegments.slice(0, index + 1).join('/'));
  }

  function joinPath(base: string, name: string): string {
    return base ? `${base}/${name}` : name;
  }

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
    if (symlinkBlocked(entry)) {
      showToast(symlinkTitle(entry));
      return;
    }
    if (entry.is_dir) {
      navigate(joinPath(currentPath, entry.name));
    } else if (isTextFile(entry.name)) {
      if (systemFeatures.editor && preferences?.editor_enabled !== false) void openEditor(entry);
      else showUnsupported(entry);
    } else if (isPdfName(entry.name)) {
      openPdf(entry);
    } else if (isLightboxPreview(entry.name)) {
      openPreview(entry);
    } else {
      showUnsupported(entry);
    }
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

  function isLightboxPreview(name: string): boolean {
    return isImageName(name) || isVideoName(name) || isAudioName(name);
  }

  function isPreviewable(name: string): boolean {
    return isLightboxPreview(name) || isPdfName(name);
  }

  function canPreviewEntry(entry: Entry): boolean {
    return !entry.is_dir && (isTextFile(entry.name) || isPreviewable(entry.name));
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
    return /\.(mp3|wav|ogg|m4a|flac|aac)$/i.test(name);
  }

  function isPdfName(name: string): boolean {
    return /\.pdf$/i.test(name);
  }

  function isVideoName(name: string): boolean {
    return /\.(mp4|mov|mkv|webm|avi|m4v)$/i.test(name);
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

  function openPreview(entry: Entry): void {
    const items = entries.filter((item) => !item.is_dir && isLightboxPreview(item.name));
    const index = items.findIndex((item) => item.name === entry.name);
    if (index >= 0) {
      preview = { items, index };
    }
  }

  function previewEntry(entry: Entry): void {
    if (isTextFile(entry.name)) {
      if (systemFeatures.editor && preferences?.editor_enabled !== false) void openEditor(entry);
      else showUnsupported(entry);
    } else if (isPdfName(entry.name)) {
      openPdf(entry);
    } else if (isLightboxPreview(entry.name)) {
      openPreview(entry);
    } else {
      showUnsupported(entry);
    }
  }

  function runContextAction(
    action:
      | 'preview'
      | 'download'
      | 'share'
      | 'rename'
      | 'copy-to'
      | 'move-to'
      | 'delete'
      | 'play'
      | 'properties',
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
    if (action === 'preview') previewEntry(entry);
    if (action === 'download') downloadEntry(entry);
    if (action === 'share') {
      if (targets.length > 1 && targets.some((item) => item.is_dir)) {
        showToast(t($language, '目录请单独分享'));
        return;
      }
      shareTargets = targets;
    }
    if (action === 'rename') {
      nameDialog = {
        mode: 'rename',
        entry,
        title: t($language, '重命名'),
        label: t($language, '新名称'),
        initialValue: entry.name,
      };
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
      } else if (dialog.entry) {
        await api.rename(rootId, joinPath(currentPath, dialog.entry.name), value, csrf);
      }
      clearDirSizeCache();
      await loadList(rootId, currentPath);
    } catch (error) {
      handleFilesError(error);
    }
  }

  function startDrag(event: DragEvent, entry: Entry): void {
    if (activeRootId === null) return;
    const path = joinPath(currentPath, entry.name);
    dragPayload = { rootId: activeRootId, path, name: entry.name };
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('text/plain', path);
    }
  }

  function allowDrop(event: DragEvent): void {
    event.preventDefault();
    event.stopPropagation();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
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
        updateProgress(job.total > 0 ? Math.min(job.progress, job.total) : job.progress, detail);
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
    const payload = dragPayload;
    dragPayload = null;
    if (!payload) return;
    if (payload.rootId === destRoot && payload.path === destPath) return;
    if (payload.rootId === destRoot && destPath.startsWith(`${payload.path}/`)) {
      showNotice(t($language, '不能把目录移动到它自己里面。'), 'warning', t($language, '移动失败'));
      return;
    }
    startProgress(t($language, '移动文件'), 1, payload.name);
    try {
      const job = await api.transferJob(
        'move',
        payload.rootId,
        [payload.path],
        destRoot,
        destPath,
        csrf,
      );
      await waitForTransferJob(job.job_id);
      updateProgress(1, t($language, '完成'));
      finishProgress();
      clearDirSizeCache();
      if (activeRootId !== null) await loadList(activeRootId, currentPath);
    } catch (error) {
      failProgress(error instanceof Error ? error.message : t($language, '移动失败'));
      handleFilesError(error);
    }
  }

  async function submitTransfer(destRoot: number, destPath: string): Promise<void> {
    const pending = transfer;
    transfer = null;
    if (!pending || activeRootId === null) return;
    const rootId = activeRootId;
    startProgress(
      pending.mode === 'copy' ? t($language, '复制文件') : t($language, '移动文件'),
      pending.entries.length,
    );
    try {
      const job = await api.transferJob(
        pending.mode,
        rootId,
        pending.entries.map((entry) => joinPath(currentPath, entry.name)),
        destRoot,
        destPath,
        csrf,
      );
      await waitForTransferJob(job.job_id);
      updateProgress(pending.entries.length, t($language, '完成'));
      clearDirSizeCache();
      await loadList(rootId, currentPath);
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

  function isTextFile(name: string): boolean {
    const lower = name.toLowerCase();
    const dot = lower.lastIndexOf('.');
    const extension = dot > 0 ? lower.slice(dot + 1) : '';
    const binaryExtensions = new Set([
      'jpg',
      'jpeg',
      'png',
      'gif',
      'webp',
      'bmp',
      'tif',
      'tiff',
      'avif',
      'heic',
      'heif',
      'mp4',
      'mov',
      'mkv',
      'webm',
      'avi',
      'm4v',
      'mp3',
      'wav',
      'ogg',
      'flac',
      'aac',
      'm4a',
      'zip',
      'rar',
      '7z',
      'tar',
      'gz',
      'bz2',
      'xz',
      'pdf',
      'doc',
      'docx',
      'xls',
      'xlsx',
      'ppt',
      'pptx',
      'exe',
      'dll',
      'so',
      'dylib',
      'bin',
      'woff',
      'woff2',
      'ttf',
      'otf',
    ]);
    return !binaryExtensions.has(extension);
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
    if (uploading) return;
    if (!activeRoot) return;
    if (currentReadOnly) {
      showToast(t($language, '当前目录只读'));
      return;
    }
    fileInput?.click();
  }

  async function handleFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    await uploadFiles(files);
  }

  async function handleFolderFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    await uploadFiles(files, true);
  }

  function folderUploadClick(): void {
    if (uploading) return;
    if (!activeRoot) return;
    if (currentReadOnly) {
      showToast(t($language, '当前目录只读'));
      return;
    }
    folderInput?.click();
  }

  async function uploadFiles(files: File[], preservePaths = false): Promise<void> {
    if (activeRootId === null || files.length === 0) return;
    if (currentReadOnly) {
      showToast(t($language, '当前目录只读'));
      return;
    }

    const rootId = activeRootId;
    const controller = new AbortController();
    let cancelled = false;
    const failed: File[] = [];
    let completed = 0;

    filesError = '';
    uploading = true;
    startProgress(t($language, '上传文件'), files.length);
    setProgressCancel(() => {
      cancelled = true;
      controller.abort();
    });
    try {
      for (const file of files) {
        if (cancelled) break;
        updateProgress(completed, file.name);
        try {
          await uploadOne(rootId, file, preservePaths, controller.signal);
          completed += 1;
        } catch (error) {
          if (cancelled || isAbortError(error)) break;
          failed.push(file);
          console.warn('upload failed', file.name, error);
        }
      }
      if (cancelled) {
        failProgress(t($language, '上传已取消'));
        return;
      }
      clearDirSizeCache();
      await loadList(rootId, currentPath);
      if (failed.length > 0) {
        failProgress(t($language, '{count} 个文件上传失败', { count: failed.length }));
        showNoticeAction(
          t($language, '{count} 个文件上传失败', { count: failed.length }),
          t($language, '上传失败'),
          t($language, '重试失败文件'),
          () => void uploadFiles(failed, preservePaths),
          'error',
        );
      } else {
        updateProgress(files.length, t($language, '{count} 个文件完成', { count: files.length }));
        finishProgress();
      }
    } finally {
      setProgressCancel(null);
      uploading = false;
    }
  }

  async function uploadOne(
    rootId: number,
    file: File,
    preservePaths: boolean,
    signal: AbortSignal,
  ): Promise<void> {
    if (file.size === 0) {
      throw new Error(t($language, '空文件暂不支持上传：{name}', { name: file.name }));
    }
    const relative = preservePaths ? file.webkitRelativePath || file.name : file.name;
    const parts = relative.split('/').filter(Boolean);
    const filename = parts.pop() ?? file.name;
    const directory = parts.join('/');
    const targetPath = joinPath(currentPath, directory);
    if (directory) {
      await api.mkdir(rootId, currentPath, directory, csrf, true);
    }
    const created = await api.createUpload(rootId, targetPath, filename, file.size, csrf);
    try {
      for (let index = 0; index < created.chunks; index += 1) {
        if (signal.aborted) {
          await api.cancelUpload(created.upload_id, csrf).catch(() => undefined);
          throw new DOMException('aborted', 'AbortError');
        }
        const start = index * created.chunk_size;
        const end = Math.min(start + created.chunk_size, file.size);
        await api.uploadChunk(created.upload_id, index, file.slice(start, end), csrf, signal);
      }
      await api.completeUpload(created.upload_id, csrf);
    } catch (error) {
      await api.cancelUpload(created.upload_id, csrf).catch(() => undefined);
      throw error;
    }
  }

  async function deleteEntry(entry: Entry): Promise<void> {
    await requestDelete([entry]);
  }

  async function requestDelete(items: Entry[]): Promise<void> {
    if (activeRootId === null || !activeRoot || items.length === 0) {
      return;
    }
    if (currentReadOnly) {
      showToast(t($language, '当前目录只读'));
      return;
    }

    const rootId = activeRootId;
    const path = currentPath;
    try {
      const paths = items.map((entry) => joinPath(path, entry.name));
      const result = await api.deletePreview(rootId, paths, csrf);
      const totalSize = items.reduce((sum, entry) => sum + entry.size, 0);
      const needsCode =
        items.some((entry) => entry.is_dir) || items.length > 5 || totalSize > 10 * 1024 * 1024;
      const list = items
        .slice(0, 12)
        .map((entry) => `× ${entry.name}${entry.is_dir ? '/' : ''}`)
        .join('\n');
      const more =
        items.length > 12
          ? '\n' + t($language, '…以及另外 {count} 项', { count: items.length - 12 })
          : '';
      confirmState = {
        title: t($language, '永久删除'),
        message: t($language, '以下文件将被永久删除：\n{list}{more}', { list, more }),
        requireText: needsCode ? result.confirm_code : null,
        action: (value) => {
          void (async () => {
            startProgress(t($language, '删除文件'), items.length, t($language, '正在删除…'));
            try {
              await api.deletePaths(result.token, undefined, csrf, needsCode ? value : undefined);
              updateProgress(items.length, t($language, '完成'));
              finishProgress();
              confirmState = null;
              clearDirSizeCache();
              await loadList(rootId, path);
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
      sortOrder = 'asc';
    }
    if (activeRootId !== null) {
      localStorage.setItem(`sort:${activeRootId}:${currentPath}`, `${sortField}:${sortOrder}`);
      void loadList(activeRootId, currentPath);
    }
  }

  function setView(next: 'list' | 'thumb' | 'tile'): void {
    view = next;
    localStorage.setItem('filehub.view', next);
    if (activeRootId !== null) {
      localStorage.setItem(`view:${activeRootId}:${currentPath}`, next);
    }
  }

  async function openEditor(entry: Entry): Promise<void> {
    if (activeRootId === null) return;
    const path = joinPath(currentPath, entry.name);
    try {
      const probe = await api.probeFile(activeRootId, path);
      if (!probe.is_text) {
        showUnsupported(entry);
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
      editorOpen = true;
    } catch (error) {
      if (error instanceof ApiError && (error.status === 400 || error.status === 415)) {
        showUnsupported(entry);
      } else {
        handleFilesError(error);
      }
    }
  }

  async function saveEditor(auto = false): Promise<void> {
    if (activeRootId === null || !editorOpen) return;
    editorSaving = true;
    editorError = '';
    try {
      const saved = await api.saveFile(activeRootId, editorPath, editorContent, editorEtag, csrf);
      editorEtag = saved.etag;
      editorOriginalContent = editorContent;
      if (!auto) editorOpen = false;
      clearDirSizeCache();
      await loadList(activeRootId, currentPath);
    } catch (error) {
      if (error instanceof ApiError && error.status === 412) {
        editorError = t($language, '文件已被其他操作修改，请关闭后重新打开。');
      } else {
        editorError = error instanceof Error ? error.message : t($language, '保存失败');
      }
    } finally {
      editorSaving = false;
    }
  }

  async function downloadSelectedZip(): Promise<void> {
    if (activeRootId === null || selected.length === 0) return;

    try {
      const paths = selected.map((name) => joinPath(currentPath, name));
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

  function splitFileName(name: string): { stem: string; ext: string } {
    const dot = name.lastIndexOf('.');
    if (dot <= 0) return { stem: name, ext: '' };
    return { stem: name.slice(0, dot), ext: name.slice(dot) };
  }

  function formatSize(size: number): string {
    if (size < 1024) return `${size} B`;
    if (size < 1024 ** 2) return `${(size / 1024).toFixed(1)} KB`;
    if (size < 1024 ** 3) return `${(size / 1024 ** 2).toFixed(1)} MB`;
    if (size < 1024 ** 4) return `${(size / 1024 ** 3).toFixed(1)} GB`;
    return `${(size / 1024 ** 4).toFixed(1)} TB`;
  }

  function isImageName(name: string): boolean {
    return /\.(jpe?g|png|gif|webp|bmp|tiff?)$/i.test(name);
  }

  function hideBrokenThumb(event: Event): void {
    const image = event.currentTarget as HTMLImageElement;
    image.style.display = 'none';
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
  {#if preferences?.custom_css_path && preferences.custom_css_version > 0}
    <link rel="stylesheet" href={api.customCssUrl(preferences.custom_css_version)} />
  {/if}
</svelte:head>

<svelte:window
  onclick={() => {
    contextMenu = null;
    blankMenu = null;
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
    ondrop={(event) => {
      event.preventDefault();
      void uploadFiles(Array.from(event.dataTransfer?.files ?? []));
    }}
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
        <span class="brand-mark" aria-hidden="true">
          {#if $branding.iconUrl}
            <img src={$branding.iconUrl} alt="" />
          {:else}
            {brandMark($branding.name)}
          {/if}
        </span>
        <span class="brand-name">{$branding.name}</span>
      </div>

      {#if appView === 'files'}
        <div class="search">
          <Icon name="search" size={18} />
          <input
            bind:this={searchInput}
            bind:value={searchQuery}
            aria-label={t($language, '搜索文件或目录')}
            placeholder={t($language, '搜索文件或目录')}
            oninput={handleSearchInput}
          />
        </div>
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
              <Icon name="image" size={18} />
            </button>
            <button
              type="button"
              class:active={view === 'tile'}
              aria-pressed={view === 'tile'}
              aria-label={t($language, '平铺视图')}
              title={t($language, '平铺视图')}
              onclick={() => setView('tile')}
            >
              <Icon name="grid" size={18} />
            </button>
          </div>
        {/if}

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
            class:inactive={currentReadOnly}
            aria-disabled={!activeRoot || currentReadOnly || uploading}
            title={currentReadOnly ? t($language, '当前目录只读') : t($language, '上传文件')}
            aria-label={t($language, '上传')}
            onclick={uploadClick}
          >
            <Icon name="upload" size={18} />
            <span>{uploading ? t($language, '上传中…') : t($language, '上传')}</span>
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
          {#each roots as root (root.id)}
            <button
              type="button"
              class:active={appView === 'files' && root.id === activeRootId}
              class="nav-item"
              onclick={() => {
                appView = 'files';
                activeRootId = root.id;
                void loadList(root.id, '');
              }}
            >
              <Icon name="folder" size={18} />
              <span>{rootLabel(root.alias)}</span>
            </button>
          {/each}
          {#if roots.length === 0}
            <span class="nav-empty">{t($language, '暂无目录')}</span>
          {/if}
          <div class="nav-sep"></div>
          <button
            type="button"
            class="nav-item"
            disabled={!activeRoot}
            title={activeRoot ? t($language, '终端') : t($language, '请先选择存储目录')}
            onclick={() => (terminalOpen = true)}
          >
            <Icon name="terminal" size={18} />
            <span>{t($language, '终端')}</span>
          </button>
          <button
            type="button"
            class:active={appView === 'shares'}
            class="nav-item"
            onclick={() => (appView = 'shares')}
          >
            <Icon name="share" size={18} />
            <span>{t($language, '分享管理')}</span>
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
        <div class="app-version">v{__APP_VERSION__}</div>
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
            <div class="path">
              <Icon name="folder" size={15} />
              {#if currentReadOnly}
                <span class="readonly-tag">{t($language, '只读')}</span>
              {/if}
              <button type="button" class="path-link" onclick={() => navigate('')}>
                {activeRoot ? rootLabel(activeRoot.alias) : t($language, '文件')}
              </button>
              {#each pathSegments as segment, index (index)}
                <span class="sep">/</span>
                <button type="button" class="path-link" onclick={() => navigateToSegment(index)}>
                  {segment}
                </button>
              {/each}
            </div>
            {#if selected.length > 0}
              <div class="pathbar-actions">
                <span class="selection-count"
                  >{t($language, '已选 {count} 项', { count: selected.length })}</span
                >
                <button
                  type="button"
                  class="icon-button"
                  aria-label={t($language, '下载 ZIP')}
                  title={t($language, '下载 ZIP')}
                  onclick={() => void downloadSelectedZip()}
                >
                  <Icon name="download" size={16} />
                </button>
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
                <button
                  type="button"
                  class="icon-button danger"
                  aria-label={t($language, '删除')}
                  title={t($language, '删除')}
                  onclick={() => void deleteSelected()}
                >
                  <Icon name="trash" size={16} />
                </button>
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
                    {roots}
                    {preferences}
                    system={systemFeatures}
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
            {#if appView === 'settings'}
              <!-- kept mounted above so returning to settings does not reload -->
            {:else if appView === 'admin' || appView === 'audit'}
              <div class:view-hidden={appView === 'audit'}>
                <AdminPage
                  {csrf}
                  onAudit={() => (appView = 'audit')}
                  onRootsChanged={() => void refreshRoots()}
                />
              </div>
              {#if appView === 'audit'}
                <AuditPage onBack={() => (appView = 'admin')} />
              {/if}
            {:else if appView === 'shares'}
              <!-- kept mounted above so returning to shares does not reload -->
            {:else}
              {#if searchActive}
                <div class="search-header">
                  <span>{t($language, '搜索')} “{searchQuery}”</span>
                  <div class="search-header-actions">
                    {#if searchIndexing}
                      <span class="muted">{t($language, '索引中…')}</span>
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
                {#if searchResults.length === 0}
                  <div class="files-state">{t($language, '没有找到匹配的文件')}</div>
                {:else}
                  <div class="search-results">
                    {#each searchResults as result (result.path)}
                      <button
                        type="button"
                        class="search-result"
                        onclick={() => openSearchResult(result)}
                      >
                        <Icon name={result.is_dir ? 'folder' : 'file'} size={18} />
                        <span class="file-name">{result.name}</span>
                        <span class="search-path">{result.path}</span>
                        <span class="search-size">
                          {result.is_dir ? '—' : formatSize(result.size)}
                        </span>
                      </button>
                    {/each}
                  </div>
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
                      aria-disabled={!activeRoot || currentReadOnly || uploading}
                      onclick={uploadClick}
                    >
                      <Icon name="upload" size={18} />
                      <span>{uploading ? t($language, '上传中…') : t($language, '上传文件')}</span>
                    </button>
                  </section>
                {:else if view === 'list'}
                  <div class="browser">
                    <table class="file-table" bind:this={fileTable}>
                      <thead>
                        <tr>
                          <th class="check"></th>
                          <th
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
                          {@const nameParts = splitFileName(entry.name)}
                          <tr
                            data-entry-name={entry.name}
                            class:selected={isSelected(entry.name)}
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
                            ondragend={() => (dragPayload = null)}
                            ondragover={(event) => {
                              if (entry.is_dir) allowDrop(event);
                            }}
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
                                >
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
                              {#if isAudioName(entry.name)}
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
                        {@const nameParts = splitFileName(entry.name)}
                        <div
                          class="file-card"
                          data-entry-name={entry.name}
                          class:selected={isSelected(entry.name)}
                          class:marquee-selected={marqueePreview.includes(entry.name)}
                          role="button"
                          tabindex="0"
                          draggable="true"
                          onclick={(event) => handleEntryClick(event, entry)}
                          ondblclick={() => enterEntry(entry)}
                          oncontextmenu={(event) => openContextMenu(event, entry)}
                          ondragstart={(event) => startDrag(event, entry)}
                          ondragend={() => (dragPayload = null)}
                          ondragover={(event) => {
                            if (entry.is_dir) allowDrop(event);
                          }}
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
                          <div class="card-icon">
                            <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={40} />
                            {#if systemFeatures.thumbnails && preferences?.show_thumbnails !== false && !entry.is_dir && entry.has_preview && activeRootId !== null}
                              <img
                                class="thumb-image"
                                src={api.thumbUrl(
                                  activeRootId,
                                  joinPath(currentPath, entry.name),
                                  320,
                                )}
                                alt=""
                                loading="lazy"
                                decoding="async"
                                onerror={hideBrokenThumb}
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
                            >
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
                        {@const nameParts = splitFileName(entry.name)}
                        <div
                          class="file-tile"
                          data-entry-name={entry.name}
                          class:selected={isSelected(entry.name)}
                          class:marquee-selected={marqueePreview.includes(entry.name)}
                          role="button"
                          tabindex="0"
                          draggable="true"
                          onclick={(event) => handleEntryClick(event, entry)}
                          ondblclick={() => enterEntry(entry)}
                          oncontextmenu={(event) => openContextMenu(event, entry)}
                          ondragstart={(event) => startDrag(event, entry)}
                          ondragend={() => (dragPayload = null)}
                          ondragover={(event) => {
                            if (entry.is_dir) allowDrop(event);
                          }}
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
                          <div class="tile-icon">
                            <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={34} />
                            {#if systemFeatures.thumbnails && preferences?.show_thumbnails !== false && !entry.is_dir && entry.has_preview && activeRootId !== null}
                              <img
                                class="thumb-image"
                                src={api.thumbUrl(
                                  activeRootId,
                                  joinPath(currentPath, entry.name),
                                  320,
                                )}
                                alt=""
                                loading="lazy"
                                decoding="async"
                                onerror={hideBrokenThumb}
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
                              >
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
      {#if canPreviewEntry(contextMenu.entry)}
        <button type="button" role="menuitem" onclick={() => runContextAction('preview')}
          >{t($language, '预览')}</button
        >
      {/if}
      {#if !contextMenu.entry.is_dir && isAudioName(contextMenu.entry.name)}
        <button type="button" role="menuitem" onclick={() => runContextAction('play')}
          >{t($language, '播放')}</button
        >
      {/if}
      <button type="button" role="menuitem" onclick={() => runContextAction('properties')}
        >{t($language, '详情')}</button
      >
      <button type="button" role="menuitem" onclick={() => runContextAction('download')}
        >{t($language, '下载')}</button
      >
      <button type="button" role="menuitem" onclick={() => runContextAction('share')}
        >{t($language, '分享')}</button
      >
      {#if !currentReadOnly}
        <button type="button" role="menuitem" onclick={() => runContextAction('rename')}
          >{t($language, '重命名')}</button
        >
      {/if}
      <button type="button" role="menuitem" onclick={() => runContextAction('copy-to')}
        >{t($language, '复制到…')}</button
      >
      {#if !currentReadOnly}
        <button type="button" role="menuitem" onclick={() => runContextAction('move-to')}
          >{t($language, '移动到…')}</button
        >
      {/if}
      <div class="menu-sep"></div>
      {#if currentReadOnly}
        <button type="button" role="menuitem" class="danger" disabled>{t($language, '删除')}</button
        >
      {:else}
        <button
          type="button"
          role="menuitem"
          class="danger"
          onclick={() => runContextAction('delete')}>{t($language, '删除')}</button
        >
      {/if}
    </div>
  {/if}

  {#if blankMenu}
    <div
      class="context-menu"
      class:open-left={blankMenuLeft}
      class:open-up={blankMenuUp}
      bind:this={blankMenuElement}
      role="menu"
      tabindex="-1"
      style={`left:${blankMenu.x}px;top:${blankMenu.y}px`}
      onclick={(event) => event.stopPropagation()}
      onkeydown={menuKeydown}
    >
      <button
        type="button"
        role="menuitem"
        disabled={currentReadOnly}
        onclick={() => {
          blankMenu = null;
          openMkdir();
        }}>{t($language, '新建目录')}</button
      >
      <button
        type="button"
        role="menuitem"
        disabled={currentReadOnly}
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
        disabled={currentReadOnly}
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
          terminalOpen = true;
        }}>{t($language, '打开终端')}</button
      >
      <div class="menu-sep"></div>
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

  {#if preview && activeRootId !== null}
    <PreviewModal
      root={activeRootId}
      {currentPath}
      items={preview.items}
      index={preview.index}
      onClose={() => (preview = null)}
      onIndexChange={(index) => (preview = preview ? { ...preview, index } : null)}
    />
  {/if}

  {#if nameDialog}
    <NameDialog
      open
      title={nameDialog.title}
      label={nameDialog.label}
      initialValue={nameDialog.initialValue}
      confirmLabel={nameDialog.mode === 'mkdir' ? t($language, '创建') : t($language, '重命名')}
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

  {#if terminalOpen && activeRootId !== null}
    <TerminalPanel
      root={activeRootId}
      cwd={currentPath}
      {csrf}
      onClose={() => (terminalOpen = false)}
      onRefresh={() => {
        if (activeRootId !== null) void loadList(activeRootId, currentPath);
      }}
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
    <div class="editor-overlay" role="presentation" onclick={() => (editorOpen = false)}>
      <dialog
        open
        class="editor-panel"
        aria-labelledby="editor-title"
        onclick={(event) => event.stopPropagation()}
      >
        <header class="editor-header">
          <strong id="editor-title">{editorName}</strong>
          <div class="editor-actions">
            {#if isMarkdown(editorName)}
              <button type="button" class="ghost" onclick={() => (editorPreview = !editorPreview)}>
                {editorPreview ? t($language, '编辑') : t($language, '预览')}
              </button>
            {/if}
            <button
              type="button"
              class="icon-button"
              aria-label={t($language, '关闭编辑器')}
              onclick={() => (editorOpen = false)}
            >
              <Icon name="close" size={18} />
            </button>
          </div>
        </header>

        {#if editorPreview}
          <!-- eslint-disable-next-line svelte/no-at-html-tags -->
          <div class="markdown-preview">{@html previewHtml}</div>
        {:else}
          <div class="editor-body">
            <pre
              bind:this={lineNumbersElement}
              class="line-numbers">{#each editorLineNumbers as number (number)}<span>{number}</span
                >{/each}</pre>
            <textarea
              bind:value={editorContent}
              spellcheck="false"
              onscroll={(event) => {
                if (lineNumbersElement) {
                  lineNumbersElement.scrollTop = event.currentTarget.scrollTop;
                }
              }}></textarea>
          </div>
        {/if}

        {#if editorError}
          <p class="editor-error" role="alert">{editorError}</p>
        {/if}

        <footer class="editor-footer">
          <span
            >{editorDirty ? t($language, '有未保存修改') : t($language, '已保存')} · UTF-8 ·
            {editorContent.length} chars</span
          >
          <div>
            <button type="button" class="ghost" onclick={() => (editorOpen = false)}
              >{t($language, '取消')}</button
            >
            <button
              type="button"
              class="primary"
              disabled={editorSaving}
              onclick={() => void saveEditor()}
            >
              {editorSaving ? t($language, '保存中…') : t($language, '保存')}
            </button>
          </div>
        </footer>
      </dialog>
    </div>
  {/if}
{/if}

<AudioPlayer />

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
    stroke-width: 2.1;
  }

  .nav-item:hover:not(:disabled) {
    background: var(--hover);
  }

  .nav-item.active {
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: 700;
  }

  .nav-item.active span {
    font-weight: 700;
  }

  .nav-item:disabled {
    opacity: 0.45;
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
    padding: 6px 2px 0;
    color: var(--text-2);
    font-size: 11.5px;
    text-align: left;
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
    height: var(--topbar-height);
    padding: 0 20px;
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

  .path {
    display: flex;
    min-width: 0;
    flex: 1;
    align-items: center;
    gap: 8px;
    overflow: auto;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 14px;
    white-space: nowrap;
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

  .browser {
    width: 100%;
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

  .line-numbers span {
    display: block;
  }

  .line-numbers {
    width: 52px;
    min-width: 52px;
    margin: 0;
    padding: 16px 8px 16px 0;
    overflow: hidden;
    border-right: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 14.5px;
    line-height: 1.6;
    text-align: right;
    user-select: none;
  }

  .editor-panel textarea {
    min-width: 0;
    flex: 1;
    padding: 16px;
    border: 0;
    outline: 0;
    resize: none;
    background: var(--surface);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 14.5px;
    line-height: 1.6;
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
