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
	skills: { id: string; description: string }[];
};
let { data } = $props();
const config = $derived(data.configuration as Configuration | null);
const availableCredentials = $derived(
	(data.credentials as Credential[])
		.filter(
			(credential) =>
				credential.status === "available" &&
				(data.credentialTypes as CredentialType[]).some(
					(type) => type.id === credential.credentialType && type.provider,
				),
		)
		.map((credential) => ({ id: credential.id, name: credential.name })),
);
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
function addSkill() {
	if (!draft || !config) return;
	const id = config.skills.find(
		(skill) => !draft!.skillIds.includes(skill.id),
	)?.id;
	if (id) draft.skillIds = [...draft.skillIds, id];
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
			{#each config.agents as agent (agent.id)}
				<button class:selected={draft?.id === agent.id} class="agent-item" onclick={() => open(agent)}><span><strong>{agent.id}</strong><small>{agent.model ? `${agent.model.provider} · ${agent.model.model}` : agent.runtimeKind}</small></span></button>
			{:else}<p class="muted">No agents yet. Create a reusable role for your pipelines.</p>{/each}
		</aside>
		<section class="agent-editor">
			{#if !draft}
				<EmptyState title="Select or create an agent" description="Agents hold instructions, provider credentials, model and skills." />
			{:else}
				<form onsubmit={(event) => { event.preventDefault(); save(); }}>
					<div class="editor-header"><div><span class="eyebrow">{editing ? "Editing agent" : "New agent"}</span>{#if !editing}<h2>{draft.id || "Untitled agent"}</h2>{/if}</div><div><Button variant="secondary" disabled={busy} onclick={() => { draft = null; error = ""; }}>Cancel</Button> <Button type="submit" loading={busy}>Save agent</Button></div></div>
					<section class="form-section" aria-labelledby="agent-runtime-heading"><header><span class="eyebrow">Runtime</span><h3 id="agent-runtime-heading">Identity and model</h3></header><div class="form-grid">
						<div class="agent-id-field"><Field label="Agent ID" for="agent-id" required hint="IDs cannot be changed after creation.">{#snippet children({ describedBy })}<input id="agent-id" bind:value={draft!.id} required disabled={busy || editing} aria-describedby={describedBy} />{/snippet}</Field></div>
						<div class="credential-field"><Field label="Credential" for="agent-credential" required hint="Credentials are managed in Settings and can be reused by agents.">{#snippet children({ describedBy, invalid })}<ResourceSelect id="agent-credential" value={draft!.credentialId ?? ""} items={availableCredentials} placeholder="Select a credential" label="Available credentials" {describedBy} {invalid} required disabled={busy} onValueChange={(credentialId) => void selectCredential(credentialId)} />{/snippet}</Field></div>
						<Field label="Provider" for="agent-provider" required hint="Derived from the selected credential.">{#snippet children({ describedBy })}<input id="agent-provider" value={draft!.model!.provider} readonly aria-describedby={describedBy} />{/snippet}</Field>
						<Field label="Model" for="agent-model" required hint="Available models are loaded from the provider.">{#snippet children({ describedBy, invalid })}<ResourceSelect id="agent-model" bind:value={draft!.model!.model} items={models.map((model) => ({ id: model.id, name: model.label }))} placeholder={modelsLoading ? "Loading models…" : "Select a model"} label="Available models" {describedBy} {invalid} required disabled={busy || modelsLoading || !draft!.credentialId} />{#if modelsError}<small role="alert">{modelsError}</small>{/if}{/snippet}</Field>
					</div></section>
					<section class="form-section" aria-labelledby="agent-instructions-heading"><header><span class="eyebrow">Instructions</span><h3 id="agent-instructions-heading">Behavior</h3></header><Field label="System prompt" for="agent-prompt" required hint="Define this agent’s role, scope, and response style.">{#snippet children({ describedBy })}<textarea id="agent-prompt" name="systemPrompt" bind:value={draft!.systemPrompt} rows={6} placeholder="Describe this agent's role and behavior." disabled={busy} aria-describedby={describedBy}></textarea>{/snippet}</Field>
					</section>
					<section class="references form-section" aria-labelledby="agent-skills-heading"><header class="skills-header"><div><span class="eyebrow">Capabilities</span><h3 id="agent-skills-heading">Skills</h3></div><Button variant="secondary" disabled={busy || !config.skills.some((skill) => !(draft?.skillIds.includes(skill.id) ?? false))} onclick={addSkill}><Plus size={16} /> Add skill</Button></header>{#if draft!.skillIds.length}<ol class="assigned-skills">{#each draft!.skillIds as skillId, index}<li><Field label="Skill" for={`agent-skill-${index}`}>{#snippet children({ describedBy, invalid })}<ResourceSelect id={`agent-skill-${index}`} value={skillId} items={config.skills.filter((skill) => skill.id === skillId || !draft!.skillIds.includes(skill.id)).map((skill) => ({ id: skill.id, name: skill.id }))} placeholder="Select a skill" label="Available skills" {describedBy} {invalid} disabled={busy} onValueChange={(id) => { if (draft) draft.skillIds = draft.skillIds.map((item, itemIndex) => itemIndex === index ? id : item); }} />{/snippet}</Field><button type="button" class="remove-skill" aria-label={`Remove ${skillId}`} disabled={busy} onclick={() => { if (draft) draft.skillIds = draft.skillIds.filter((_, itemIndex) => itemIndex !== index); }}><Trash2 size={15} /></button></li>{/each}</ol>{:else}<p class="muted">No skills assigned. Add skills to give this agent reusable guidance.</p>{/if}</section>
					{#if editing}<div class="delete-row"><Button variant="danger" disabled={busy} onclick={() => deleting = true}><Trash2 size={16} /> Delete agent</Button></div>{/if}
				</form>
				<ConfirmationDialog bind:open={deleting} title="Delete agent?" loading={busy} onconfirm={() => save(true)}>This cannot be undone. Agents used by a pipeline cannot be deleted until the pipeline step is changed or removed.</ConfirmationDialog>
			{/if}
		</section>
	</div>
{/if}

<style>
	.agents-layout { display:grid; grid-template-columns:15rem minmax(0,1fr); min-height:calc(100vh - 12rem); border:1px solid #303137; border-radius:.7rem; background:#1b1c20; overflow:hidden; box-shadow:0 12px 30px #0002; }.agent-list { padding:1rem; background:#16171b; border-right:1px solid #303137; }.list-heading { display:flex; justify-content:space-between; margin-bottom:.8rem; }.list-heading span { background:#28344f; border-radius:999px; padding:.1rem .45rem; color:#aac2ff; font-size:.8rem; }.agent-item { display:flex; gap:.65rem; width:100%; padding:.7rem; border:1px solid transparent; border-radius:.5rem; background:transparent; text-align:left; color:#dedee2; cursor:pointer; }.agent-item:hover,.agent-item.selected { border-color:#3c4a68; background:#242a38; }.agent-item span { min-width:0; }.agent-item strong,.agent-item small { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }.agent-item small { color:#92939b; font-size:.75rem; }.agent-editor { padding:1.5rem; }.editor-header { display:flex; align-items:start; justify-content:space-between; gap:1rem; margin-bottom:1.5rem; }.editor-header h2 { margin:.2rem 0 0; }.form-section { margin-top:1.5rem; padding-top:1.25rem; border-top:1px solid #303137; }.editor-header + .form-section { margin-top:0; }.form-section > header { margin-bottom:1rem; }.form-section h3 { margin:.15rem 0 0; color:#e8e8eb; font-size:.95rem; }.form-grid { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:1rem; }.agent-id-field,.credential-field { grid-column:1/-1; }.form-grid :global(.field) { display:grid; grid-template-rows:auto 2.6rem auto; margin:0; }.form-grid :global(.field-hint) { margin:.3rem 0; }.references .muted { font-size:.85rem; }.skills-header { display:flex; align-items:end; justify-content:space-between; gap:1rem; }.skills-header :global(.button) { flex:none; }.assigned-skills { display:grid; gap:.65rem; padding:0; margin:0; list-style:none; }.assigned-skills li { display:grid; grid-template-columns:minmax(0,1fr) auto; align-items:end; gap:.7rem; padding:.8rem; border:1px solid #393a42; border-radius:.6rem; background:#191a1e; }.assigned-skills :global(.field) { margin:0; }.remove-skill { display:grid; place-items:center; width:2rem; height:2rem; border:0; border-radius:.4rem; background:transparent; color:#e3858b; cursor:pointer; }.remove-skill:hover:not(:disabled) { color:#ffadb2; background:#3a2024; }.delete-row { display:flex; justify-content:flex-end; margin-top:1.5rem; padding-top:1rem; border-top:1px solid #303137; } @media (max-width:720px) { .agents-layout { min-height:0; } .agents-layout,.form-grid { grid-template-columns:1fr; }.form-grid :global(.field) { grid-template-rows:auto auto auto; }.agent-list { border-right:0; border-bottom:1px solid #303137; }.editor-header { flex-direction:column; } }
</style>
