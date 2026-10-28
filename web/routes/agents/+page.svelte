<script lang="ts">
import { invalidateAll } from "$app/navigation";
import Plus from "@lucide/svelte/icons/plus";
import Trash2 from "@lucide/svelte/icons/trash-2";
import Button from "../../components/Button.svelte";
import ConfirmationDialog from "../../components/ConfirmationDialog.svelte";
import EmptyState from "../../components/EmptyState.svelte";
import Field from "../../components/Field.svelte";
import PageHeader from "../../components/PageHeader.svelte";
import type { Agent } from "../../lib/generated/Agent";

type Credential = {
	id: string;
	name: string;
	credentialType: string;
	status: string;
};
type CredentialType = { id: string; label: string; provider: string | null };
type ModelOption = { id: string; label: string };
type Configuration = {
	revision: string;
	agents: Agent[];
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
let models = $state<ModelOption[]>([]);
let modelsLoading = $state(false);
let modelsError = $state("");

function open(agent?: Agent) {
	if (!config) return;
	draft = agent
		? {
				...structuredClone(agent),
				model: agent.model ?? { provider: "", model: "" },
			}
		: {
				id: crypto.randomUUID(),
				systemPrompt: "",
				runtimeKind: "rig",
				model: { provider: "openai", model: "" },
				credentialId: null,
				rules: [],
				skillIds: [],
			};
	revision = config.revision;
	editing = Boolean(agent);
	error = "";
	message = "";
	models = [];
	modelsError = "";
	if (draft.credentialId) void loadModels(draft.credentialId);
}
async function loadModels(credentialId: string) {
	modelsLoading = true;
	modelsError = "";
	models = [];
	try {
		const response = await fetch(
			`/api/credentials/${encodeURIComponent(credentialId)}/models`,
		);
		if (!response.ok)
			throw new Error((await response.json()).error ?? "Could not list models");
		models = await response.json();
		if (
			draft?.credentialId === credentialId &&
			draft.model &&
			!models.some((model) => model.id === draft?.model?.model)
		)
			draft.model.model = "";
	} catch (cause) {
		modelsError =
			cause instanceof Error ? cause.message : "Could not list models";
	} finally {
		modelsLoading = false;
	}
}
async function selectCredential(credentialId: string) {
	if (!draft) return;
	draft.credentialId = credentialId || null;
	const credential = (data.credentials as Credential[]).find(
		(item) => item.id === credentialId,
	);
	const type = (data.credentialTypes as CredentialType[]).find(
		(item) => item.id === credential?.credentialType,
	);
	draft.model = { provider: type?.provider ?? "", model: "" };
	if (credentialId) await loadModels(credentialId);
	else models = [];
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
	{#snippet actions()}<Button disabled={!config || busy} onclick={() => open()}><Plus size={17} /> Create agent</Button>{/snippet}
</PageHeader>
{#if error}<p role="alert" class="status status-error">{error}</p>{/if}
{#if message}<p role="status" class="status status-success">{message}</p>{/if}
{#if !config}
	<EmptyState title="Configuration unavailable" description="Initialize or repair project configuration in Settings before editing agents." />
{:else}
	<div class="agents-layout">
		<aside aria-label="Agent library" class="agent-list">
			<div class="list-heading"><strong>Your agents</strong><span>{config.agents.length}</span></div>
			<p class="availability-warning">Choose a provider, model, and credential directly on each agent.</p>
			{#each config.agents as agent (agent.id)}
				<button class:selected={draft?.id === agent.id} class="agent-item" onclick={() => open(agent)}><span><strong>{agent.id}</strong><small>{agent.model ? `${agent.model.provider} · ${agent.model.model}` : agent.runtimeKind}</small></span></button>
			{:else}<p class="muted">No agents yet. Create a reusable role for your pipelines.</p>{/each}
		</aside>
		<section class="agent-editor">
			{#if !draft}
				<EmptyState title="Select or create an agent" description="Agents hold instructions, provider credentials, model, rules and skills." />
			{:else}
				<form onsubmit={(event) => { event.preventDefault(); save(); }}>
					<div class="editor-header"><div><span class="eyebrow">{editing ? "Editing agent" : "New agent"}</span>{#if !editing}<h2>{draft.id || "Untitled agent"}</h2>{/if}</div><div><Button variant="secondary" disabled={busy} onclick={() => { draft = null; error = ""; }}>Cancel</Button> <Button type="submit" loading={busy}>Save agent</Button></div></div>
					<div class="form-grid">
						<Field label="Agent ID" for="agent-id" required hint="IDs cannot be changed after creation.">{#snippet children({ describedBy })}<input id="agent-id" bind:value={draft!.id} required disabled={busy || editing} aria-describedby={describedBy} />{/snippet}</Field>
						<Field label="Credential" for="agent-credential" required hint="Credentials are managed in Settings and can be reused by agents.">{#snippet children({ describedBy })}<select id="agent-credential" value={draft!.credentialId ?? ""} onchange={(event) => selectCredential(event.currentTarget.value)} required disabled={busy} aria-describedby={describedBy}><option value="">Select a credential</option>{#each (data.credentials as Credential[]).filter((credential) => credential.status === "available" && (data.credentialTypes as CredentialType[]).some((type) => type.id === credential.credentialType && type.provider)) as credential}<option value={credential.id}>{credential.name}</option>{/each}</select>{/snippet}</Field>
						<Field label="Provider" for="agent-provider" required>{#snippet children({ describedBy })}<input id="agent-provider" value={draft!.model!.provider} readonly aria-describedby={describedBy} />{/snippet}</Field>
						<Field label="Model" for="agent-model" required>{#snippet children({ describedBy })}<select id="agent-model" bind:value={draft!.model!.model} required disabled={busy || modelsLoading || !draft!.credentialId} aria-describedby={describedBy}><option value="">{modelsLoading ? "Loading models…" : "Select a model"}</option>{#each models as model}<option value={model.id}>{model.label}</option>{/each}</select>{#if modelsError}<small role="alert">{modelsError}</small>{/if}{/snippet}</Field>
					</div>
					<Field label="System prompt" for="agent-prompt" required>{#snippet children({ describedBy })}<textarea id="agent-prompt" name="systemPrompt" bind:value={draft!.systemPrompt} rows={6} placeholder="Describe this agent's role and behavior." disabled={busy} aria-describedby={describedBy}></textarea>{/snippet}</Field>
					<Field label="Rules" for="agent-rules" hint="One non-empty rule per line.">{#snippet children({ describedBy })}<textarea id="agent-rules" name="rules" value={draft!.rules.join("\n")} oninput={(event) => { if (draft) draft.rules = event.currentTarget.value.split("\n").filter((rule: string) => rule.trim()); }} rows={4} placeholder="Be concise" disabled={busy} aria-describedby={describedBy}></textarea>{/snippet}</Field>
					<div class="references"><div><h3>Skills</h3>{#each config.skills as skill}<label><input type="checkbox" checked={draft!.skillIds.includes(skill.id)} onchange={() => { if (draft) draft.skillIds = toggle(draft.skillIds, skill.id); }} disabled={busy} /> {skill.name}</label>{:else}<p class="muted">No skills configured.</p>{/each}</div></div>
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
