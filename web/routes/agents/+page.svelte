<script lang="ts">
import { invalidateAll } from "$app/navigation";
import Plus from "@lucide/svelte/icons/plus";
import Trash2 from "@lucide/svelte/icons/trash-2";
import Button from "../../components/Button.svelte";
import ConfirmationDialog from "../../components/ConfirmationDialog.svelte";
import EmptyState from "../../components/EmptyState.svelte";
import Field from "../../components/Field.svelte";
import PageHeader from "../../components/PageHeader.svelte";
import ResourceSelect from "../../components/ResourceSelect.svelte";
import type { Agent } from "../../lib/generated/Agent";

type Provider = { id: string; name: string; models: string[] };
type Configuration = {
	revision: string;
	agents: Agent[];
	providers: Provider[];
	tools: { id: string; name: string }[];
	skills: { id: string; name: string }[];
};
let { data } = $props();
const config = $derived(data.configuration as Configuration | null);
let draft = $state<Agent | null>(null);
let revision = $state("");
let editing = $state(false);
let busy = $state(false);
let error = $state("");
let message = $state("");
let deleting = $state(false);

function open(agent?: Agent) {
	if (!config) return;
	const provider = config.providers[0];
	draft = agent
		? structuredClone(agent)
		: {
				id: crypto.randomUUID(),
				systemPrompt: "",
				model: {
					provider: provider?.id ?? "",
					model: provider?.models[0] ?? "",
					maxOutputTokens: null,
				},
				rules: [],
				toolIds: [],
				skillIds: [],
			};
	revision = config.revision;
	editing = Boolean(agent);
	error = "";
	message = "";
}
function models() {
	return (
		config?.providers.find((provider) => provider.id === draft?.model.provider)
			?.models ?? []
	);
}
function selectProvider(id: string) {
	if (!draft || !config) return;
	draft.model.provider = id;
	draft.model.model =
		config.providers.find((provider) => provider.id === id)?.models[0] ?? "";
}
function toggle(ids: string[], id: string) {
	return ids.includes(id) ? ids.filter((item) => item !== id) : [...ids, id];
}
async function save(remove = false) {
	if (!draft) return;
	busy = true;
	error = "";
	message = "";
	try {
		const response = await fetch(
			`/api/agents${editing ? `/${encodeURIComponent(draft.id)}` : ""}`,
			{
				method: remove ? "DELETE" : editing ? "PUT" : "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ revision, ...(remove ? {} : { agent: draft }) }),
			},
		);
		if (!response.ok) {
			const body = await response.json();
			throw new Error(body.error ?? "Could not save agent.");
		}
		draft = null;
		deleting = false;
		await invalidateAll();
		message = remove ? "Agent deleted." : "Agent saved.";
	} catch (cause) {
		error = cause instanceof Error ? cause.message : "Could not save agent.";
	} finally {
		busy = false;
	}
}
</script>

<svelte:head><title>Agents · Baton</title></svelte:head>
<PageHeader title="Agents" description="Define reusable roles for pipeline steps.">
	{#snippet actions()}<Button disabled={!config?.providers.length || busy} onclick={() => open()}><Plus size={17} /> Create agent</Button>{/snippet}
</PageHeader>
{#if error}<p role="alert" class="status status-error">{error}</p>{/if}
{#if message}<p role="status" class="status status-success">{message}</p>{/if}
{#if !config}
	<EmptyState title="Configuration unavailable" description="Initialize or repair project configuration in Settings before editing agents." />
{:else}
	<div class="agents-layout">
		<aside aria-label="Agent library" class="agent-list">
			<div class="list-heading"><strong>Your agents</strong><span>{config.agents.length}</span></div>
			{#if !config.providers.length}<p class="availability-warning">No verified local models are available. Configure and test a provider in Settings before creating or editing agents.</p>{/if}
			{#each config.agents as agent (agent.id)}
				<button class:selected={draft?.id === agent.id} class="agent-item" onclick={() => open(agent)}><span><strong>{agent.id}</strong><small>{agent.model.model}</small></span></button>
			{:else}<p class="muted">No agents yet. Create a reusable role for your pipelines.</p>{/each}
		</aside>
		<section class="agent-editor">
			{#if !draft}
				<EmptyState title="Select or create an agent" description="Agents hold a system prompt, verified model choice, rules, and optional tools and skills." />
			{:else}
				<form onsubmit={(event) => { event.preventDefault(); save(); }}>
					<div class="editor-header"><div><span class="eyebrow">{editing ? "Editing agent" : "New agent"}</span>{#if !editing}<h2>{draft.id || "Untitled agent"}</h2>{/if}</div><div><Button variant="secondary" disabled={busy} onclick={() => { draft = null; error = ""; }}>Cancel</Button> <Button type="submit" loading={busy} disabled={!config.providers.length}>Save agent</Button></div></div>
					<div class="form-grid">
						<Field label="Agent ID" for="agent-id" required hint="IDs cannot be changed after creation.">{#snippet children({ describedBy })}<input id="agent-id" bind:value={draft!.id} required disabled={busy || editing} aria-describedby={describedBy} />{/snippet}</Field>
						<Field label="Provider" for="agent-provider" required hint="Only verified local providers are shown.">{#snippet children({ describedBy })}<ResourceSelect id="agent-provider" value={draft!.model.provider} onValueChange={selectProvider} items={config.providers.map((provider) => ({ id: provider.id, name: provider.name }))} kind="provider" placeholder="Select a provider" disabled={busy || !config.providers.length} {describedBy} />{/snippet}</Field>
						<Field label="Model" for="agent-model" required hint="Only models verified as available are shown.">{#snippet children({ describedBy })}<ResourceSelect id="agent-model" bind:value={draft!.model.model} items={models().map((model) => ({ id: model }))} kind="model" placeholder="Select a model" disabled={busy || !models().length} {describedBy} />{/snippet}</Field>
						<Field label="Maximum output tokens" for="agent-tokens" hint="Optional; leave blank for the provider default.">{#snippet children({ describedBy })}<input id="agent-tokens" type="number" min="1" value={draft!.model.maxOutputTokens ?? ""} oninput={(event) => { if (draft) draft.model.maxOutputTokens = event.currentTarget.value ? Number(event.currentTarget.value) : null; }} disabled={busy} aria-describedby={describedBy} />{/snippet}</Field>
					</div>
					<Field label="System prompt" for="agent-prompt" required>{#snippet children({ describedBy })}<textarea id="agent-prompt" name="systemPrompt" bind:value={draft!.systemPrompt} rows={6} placeholder="Describe this agent's role and behavior." disabled={busy} aria-describedby={describedBy}></textarea>{/snippet}</Field>
					<Field label="Rules" for="agent-rules" hint="One non-empty rule per line.">{#snippet children({ describedBy })}<textarea id="agent-rules" name="rules" value={draft!.rules.join("\n")} oninput={(event) => { if (draft) draft.rules = event.currentTarget.value.split("\n").filter((rule: string) => rule.trim()); }} rows={4} placeholder="Be concise" disabled={busy} aria-describedby={describedBy}></textarea>{/snippet}</Field>
					<div class="references"><div><h3>Tools</h3>{#each config.tools as tool}<label><input type="checkbox" checked={draft!.toolIds.includes(tool.id)} onchange={() => { if (draft) draft.toolIds = toggle(draft.toolIds, tool.id); }} disabled={busy} /> {tool.name}</label>{:else}<p class="muted">No tools configured.</p>{/each}</div><div><h3>Skills</h3>{#each config.skills as skill}<label><input type="checkbox" checked={draft!.skillIds.includes(skill.id)} onchange={() => { if (draft) draft.skillIds = toggle(draft.skillIds, skill.id); }} disabled={busy} /> {skill.name}</label>{:else}<p class="muted">No skills configured.</p>{/each}</div></div>
					{#if editing}<div class="delete-row"><Button variant="danger" disabled={busy} onclick={() => deleting = true}><Trash2 size={16} /> Delete agent</Button></div>{/if}
				</form>
				<ConfirmationDialog bind:open={deleting} title="Delete agent?" loading={busy} onconfirm={() => save(true)}>This cannot be undone. Agents used by a pipeline cannot be deleted until the pipeline step is changed or removed.</ConfirmationDialog>
			{/if}
		</section>
	</div>
{/if}

<style>
	.agents-layout { display:grid; grid-template-columns:15rem minmax(0,1fr); border:1px solid #dbe0e9; border-radius:.7rem; background:#fff; overflow:hidden; }.agent-list { padding:1rem; background:#f8f9fc; border-right:1px solid #dbe0e9; }.list-heading { display:flex; justify-content:space-between; margin-bottom:.8rem; }.list-heading span { background:#e8edfb; border-radius:999px; padding:.1rem .45rem; color:#244aab; font-size:.8rem; }.agent-item { display:flex; gap:.65rem; width:100%; padding:.7rem; border:1px solid transparent; border-radius:.5rem; background:transparent; text-align:left; color:#27324a; cursor:pointer; }.agent-item:hover,.agent-item.selected { border-color:#bfcdf3; background:#edf2ff; }.agent-item span { min-width:0; }.agent-item strong,.agent-item small { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }.agent-item small { color:#69758b; font-size:.75rem; }.availability-warning { padding:.7rem; border-radius:.5rem; background:#fff7e5; color:#704b00; font-size:.8rem; line-height:1.4; }.agent-editor { padding:1.5rem; }.editor-header { display:flex; align-items:start; justify-content:space-between; gap:1rem; margin-bottom:1.5rem; }.editor-header h2 { margin:.2rem 0 0; }.form-grid,.references { display:grid; grid-template-columns:1fr 1fr; gap:1rem; }.references { margin-top:1.5rem; padding-top:1rem; border-top:1px solid #e7eaf0; }.references h3 { font-size:.9rem; margin:0 0 .6rem; }.references label { display:block; margin:.45rem 0; font-size:.9rem; }.references .muted { font-size:.85rem; }.delete-row { display:flex; justify-content:flex-end; margin-top:1.5rem; padding-top:1rem; border-top:1px solid #e7eaf0; } @media (max-width:720px) { .agents-layout,.form-grid,.references { grid-template-columns:1fr; }.agent-list { border-right:0; border-bottom:1px solid #dbe0e9; }.editor-header { flex-direction:column; } }
</style>
