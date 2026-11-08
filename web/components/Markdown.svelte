<script lang="ts">
import { renderMarkdown } from "$lib/markdown";

let { value }: { value: string } = $props();
let html = $derived(renderMarkdown(value));
</script>

<div class="markdown">{@html html}</div>

<style>
	.markdown { overflow-wrap: anywhere; }
	.markdown :global(p), .markdown :global(ul), .markdown :global(ol),
	.markdown :global(blockquote), .markdown :global(pre) { margin: 0.75em 0; }
	.markdown :global(ul) { list-style: disc; padding-left: 1.5rem; }
	.markdown :global(ol) { list-style: decimal; padding-left: 1.5rem; }
	.markdown :global(li > ul), .markdown :global(li > ol) { margin: 0.25em 0; }
	.markdown :global(h1), .markdown :global(h2), .markdown :global(h3),
	.markdown :global(h4), .markdown :global(h5), .markdown :global(h6) { margin: 1em 0 0.5em; font-weight: 700; line-height: 1.2; }
	.markdown :global(h1) { font-size: 2em; }
	.markdown :global(h2) { font-size: 1.5em; }
	.markdown :global(h3) { font-size: 1.25em; }
	.markdown :global(blockquote) { border-left: 3px solid var(--border-strong); padding-left: 1rem; color: var(--text-muted); }
	.markdown :global(code) { background: var(--surface-hover); border-radius: 0.25rem; padding: 0.1em 0.25em; }
	.markdown :global(pre) { overflow-x: auto; white-space: pre; padding: 0.75rem; background: var(--surface-hover); border-radius: 0.4rem; }
	.markdown :global(pre code) { padding: 0; background: transparent; }
	.markdown :global(a) { text-decoration: underline; }
	.markdown :global(> :first-child) { margin-top: 0; }
	.markdown :global(> :last-child) { margin-bottom: 0; }
</style>
