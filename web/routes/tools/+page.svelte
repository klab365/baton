<script lang="ts">
import { invalidateAll } from "$app/navigation";
import Button from "../../components/Button.svelte";
import PageHeader from "../../components/PageHeader.svelte";
let { data } = $props();
let tools = $derived(
	data.tools as {
		revision: string;
		enabled: { id: string }[];
		catalog: {
			id: string;
			version: number;
			permission: string;
			max_output_bytes: number;
			session_timeout_seconds: number;
		}[];
	},
);
let busy = $state("");
let error = $state("");
function enabled(id: string) {
	return tools.enabled.some((tool) => tool.id === id);
}
async function change(id: string) {
	busy = id;
	error = "";
	try {
		const response = await fetch(`/api/tools/${encodeURIComponent(id)}`, {
			method: enabled(id) ? "DELETE" : "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ revision: tools.revision }),
		});
		if (!response.ok) {
			const body = await response.json();
			throw new Error(body.error ?? "Could not update tool.");
		}
		await invalidateAll();
	} catch (cause) {
		error = cause instanceof Error ? cause.message : "Could not update tool.";
	} finally {
		busy = "";
	}
}
</script>
<svelte:head><title>Tools · Baton</title></svelte:head>
<PageHeader title="Built-in tools" description="Enable host-defined tools for this project, then assign enabled tools to agents." />
{#if error}<p role="alert" class="status status-error">{error}</p>{/if}
<div class="tools">{#each tools.catalog as tool}
	<article><div><h2>{tool.id}</h2><p><code>{tool.permission}</code> · v{tool.version} · {tool.max_output_bytes} byte output · {tool.session_timeout_seconds}s session</p></div><Button variant={enabled(tool.id) ? "secondary" : "primary"} disabled={Boolean(busy)} onclick={() => change(tool.id)}>{busy === tool.id ? "Saving…" : enabled(tool.id) ? "Disable" : "Enable"}</Button></article>
{:else}<p class="muted">No built-in tools are available.</p>{/each}</div>
<style>.tools { display:grid; gap:.75rem; } article { display:flex; align-items:center; justify-content:space-between; gap:1rem; border:1px solid #dbe0e9; border-radius:.7rem; padding:1rem; background:#fff; } h2 { margin:0; font-size:1rem; } p { margin:.35rem 0 0; color:#69758b; font-size:.85rem; } code { color:#27324a; }</style>
