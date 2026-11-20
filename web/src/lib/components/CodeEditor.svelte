<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { language, t } from '../i18n';

  let {
    value,
    filename = '',
    readonly = false,
    onchange,
  }: {
    value: string;
    filename?: string;
    readonly?: boolean;
    onchange: (next: string) => void;
  } = $props();

  let host: HTMLDivElement | null = $state(null);
  let ready = $state(false);
  let failed = $state(false);
  let view: import('@codemirror/view').EditorView | null = null;
  let readOnlyCompartment: import('@codemirror/state').Compartment | null = null;
  let makeReadOnlyExtensions:
    | ((locked: boolean) => import('@codemirror/state').Extension)
    | null = null;
  let destroyed = false;

  async function loadLanguage(name: string): Promise<import('@codemirror/state').Extension | null> {
    const extension = name.toLowerCase().split('.').pop() ?? '';
    if (['js', 'jsx', 'mjs', 'cjs', 'ts', 'tsx'].includes(extension)) {
      const { javascript } = await import('@codemirror/lang-javascript');
      return javascript({ typescript: extension.startsWith('ts'), jsx: extension === 'jsx' || extension === 'tsx' });
    }
    if (extension === 'json') {
      return (await import('@codemirror/lang-json')).json();
    }
    if (['py', 'pyw'].includes(extension)) {
      return (await import('@codemirror/lang-python')).python();
    }
    if (extension === 'rs') {
      return (await import('@codemirror/lang-rust')).rust();
    }
    if (['html', 'htm', 'svelte'].includes(extension)) {
      return (await import('@codemirror/lang-html')).html();
    }
    if (['css', 'scss'].includes(extension)) {
      return (await import('@codemirror/lang-css')).css();
    }
    if (['md', 'markdown'].includes(extension)) {
      return (await import('@codemirror/lang-markdown')).markdown();
    }
    if (['yaml', 'yml'].includes(extension)) {
      return (await import('@codemirror/lang-yaml')).yaml();
    }
    if (extension === 'sql') {
      return (await import('@codemirror/lang-sql')).sql();
    }
    if (['sh', 'bash', 'zsh'].includes(extension)) {
      const [{ StreamLanguage }, { shell }] = await Promise.all([
        import('@codemirror/language'),
        import('@codemirror/legacy-modes/mode/shell'),
      ]);
      return StreamLanguage.define(shell);
    }
    return null;
  }

  onMount(() => {
    void (async () => {
      try {
        const [
          viewModule,
          stateModule,
          commandsModule,
          languageModule,
        ] = await Promise.all([
          import('@codemirror/view'),
          import('@codemirror/state'),
          import('@codemirror/commands'),
          import('@codemirror/language'),
        ]);
        const language = await loadLanguage(filename);
        if (destroyed || !host) return;
        const { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection, dropCursor } =
          viewModule;
        const { EditorState, Compartment } = stateModule;
        const { defaultKeymap, history, historyKeymap, indentWithTab } = commandsModule;
        const { syntaxHighlighting, defaultHighlightStyle, bracketMatching, indentOnInput } =
          languageModule;
        readOnlyCompartment = new Compartment();
        makeReadOnlyExtensions = (locked) => [
          EditorView.editable.of(!locked),
          EditorState.readOnly.of(locked),
        ];
        const extensions = [
          lineNumbers(),
          history(),
          drawSelection(),
          dropCursor(),
          indentOnInput(),
          bracketMatching(),
          highlightActiveLine(),
          syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
          keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
          readOnlyCompartment.of([
            EditorView.editable.of(!readonly),
            EditorState.readOnly.of(readonly),
          ]),
          EditorView.lineWrapping,
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onchange(update.state.doc.toString());
          }),
          EditorView.theme({
            '&': { height: '100%', backgroundColor: 'var(--surface)', color: 'var(--text)' },
            '.cm-content': {
              padding: '16px 0',
              fontFamily: 'var(--font-mono)',
              fontSize: '14.5px',
              lineHeight: '1.6',
            },
            '.cm-scroller': { overflow: 'auto', overscrollBehavior: 'contain' },
            '.cm-gutters': {
              border: '0',
              backgroundColor: 'var(--surface)',
              color: 'var(--text-2)',
            },
            '.cm-activeLine': {
              backgroundColor: 'color-mix(in srgb, var(--text) 5%, transparent)',
            },
            '.cm-activeLineGutter': { backgroundColor: 'transparent' },
            '&.cm-focused': { outline: 'none' },
            '.cm-cursor': { borderLeftColor: 'var(--text)' },
            '.cm-selectionBackground, ::selection': {
              backgroundColor: 'color-mix(in srgb, var(--accent) 24%, transparent) !important',
            },
          }),
        ];
        if (language) extensions.push(language);
        view = new EditorView({
          state: EditorState.create({ doc: value, extensions }),
          parent: host,
        });
        ready = true;
        requestAnimationFrame(() => view?.focus());
      } catch (error) {
        failed = true;
        console.warn('CodeMirror failed to load; falling back to textarea', error);
      }
    })();
    return () => {
      destroyed = true;
      view?.destroy();
      view = null;
    };
  });

  $effect(() => {
    const next = value;
    if (!view) return;
    if (view.state.doc.toString() !== next) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next } });
    }
  });

  $effect(() => {
    const locked = readonly;
    if (!view || !readOnlyCompartment || !makeReadOnlyExtensions) return;
    view.dispatch({
      effects: readOnlyCompartment.reconfigure(makeReadOnlyExtensions(locked)),
    });
  });

  onDestroy(() => {
    view?.destroy();
    view = null;
  });
</script>

<div class="code-editor">
  <div class="code-host" bind:this={host}></div>
  {#if !ready}
    <textarea
      class="code-fallback"
      value={value}
      readonly={readonly}
      spellcheck="false"
      oninput={(event) => onchange(event.currentTarget.value)}
    ></textarea>
  {/if}
  {#if failed}
    <p class="code-error">
      {readonly ? t($language, '只读') : t($language, '编辑器加载失败，使用纯文本模式')}
    </p>
  {/if}
</div>

<style>
  .code-editor {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .code-host {
    height: 100%;
  }

  .code-fallback {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
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

  .code-error {
    position: absolute;
    right: 12px;
    bottom: 12px;
    margin: 0;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-2);
    font-size: 12px;
  }
</style>
