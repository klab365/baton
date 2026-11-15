<script lang="ts">
import { invalidateAll } from "$app/navigation";
import Plus from "@lucide/svelte/icons/plus";
import Trash2 from "@lucide/svelte/icons/trash-2";
import AssignedResources from "../../components/AssignedResources.svelte";
import Button from "../../components/Button.svelte";
import ConfirmationDialog from "../../components/ConfirmationDialog.svelte";
import EmptyState from "../../components/EmptyState.svelte";
import Field from "../../components/Field.svelte";
import MultilineInput from "../../components/MultilineInput.svelte";
import PageHeader from "../../components/PageHeader.svelte";
import ResourceSelect from "../../components/ResourceSelect.svelte";
import type { Agent } from "../../lib/generated/Agent";
import type { PiModelOption } from "../../lib/generated/PiModelOption";
import { piProviders } from "../../lib/pi-providers";

const tools = [
	"read",
	"list",
	"search",
	"shell",
	"write",
	"edit",
	"report_outcome",
];

type Credential = {
	id: string;
	name: string;
	credentialType: string;
	status: string;
};
type CredentialType = { id: string; label: string; provider: string | null };
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
let models = $state<PiModelOption[]>([]);
let modelsLoading = $state(false);
let modelsError = $state("");
let piProvider = $state("");
let piModel = $state("");
let piCustomSelected = $state(false);
let piDefaultSelected = $state(false);
let piModels = $state<PiModelOption[]>([]);
let piModelsLoading = $state(false);
let piModelsError = $state("");
let piModelsRequest = 0;
let piModelDebounce: ReturnType<typeof setTimeout> | undefined;

function parsePiModel(value: string | null): [string, string] {
	if (!value) return ["", ""];
	const separator = value.indexOf("/");
	return separator < 0
		? ["", value]
		: [value.slice(0, separator), value.slice(separator + 1)];
}

function open(agent?: Agent) {
	if (!config) return;
	draft = agent
		? {
				...structuredClone(agent),
				allowedTools: agent.allowedTools ?? [...tools],
				model: agent.model ?? null,
			}
		: {
				id: crypto.randomUUID(),
				instructions: "",
				provider: "rig",
				model: null,
				credentialId: null,
				skillIds: [],
				allowedTools: [],
			};
	revision = config.revision;
	editing = Boolean(agent);
	error = "";
	message = "";
	models = [];
	modelsError = "";
	if (draft.credentialId) void loadModels(draft.credentialId);
	[piProvider, piModel] = parsePiModel(draft.model);
	piCustomSelected =
		Boolean(piProvider) && !piProviders.some((item) => item.id === piProvider);
	piDefaultSelected = draft.provider === "pi" && !draft.model;
	piModelsRequest++;
	clearTimeout(piModelDebounce);
	piModels = [];
	piModelsLoading = false;
	piModelsError = "";
	if (draft.provider === "pi" && piProvider) void loadPiModels(piProvider);
}
async function loadModels(credentialId: string) {
	modelsLoading = true;
	modelsError = "";
	models = [];
	try {
		const response = await fetch(
			`/api/credentials/${encodeURIComponent(credentialId)}/models`,
		);
		if (response.status === 404)
			throw new Error(
				`Credential "${credentialId}" was not found locally. Create it in Settings or select another credential.`,
			);
		if (!response.ok)
			throw new Error((await response.json()).error ?? "Could not list models");
		models = await response.json();
		if (
			draft?.credentialId === credentialId &&
			draft.model &&
			!models.some(
				(model) => model.id === draft?.model?.split("/").slice(1).join("/"),
			)
		)
			draft.model = null;
	} catch (cause) {
		modelsError =
			cause instanceof Error ? cause.message : "Could not list models";
	} finally {
		modelsLoading = false;
	}
}
function selectPiProvider(value: string, custom: boolean) {
	const useDefault = value === "default";
	const provider = useDefault || custom ? "" : value;
	if (useDefault || provider !== piProvider) piModel = "";
	piCustomSelected = custom;
	piDefaultSelected = useDefault;
	piProvider = provider;
	piModelsRequest++;
	clearTimeout(piModelDebounce);
	piModels = [];
	piModelsLoading = false;
	piModelsError = "";
	if (provider) void loadPiModels(provider);
}

function schedulePiModelLoad(provider: string) {
	piModelsRequest++;
	clearTimeout(piModelDebounce);
	piModels = [];
	piModelsError = "";
	piModelsLoading = Boolean(provider);
	if (provider)
		piModelDebounce = setTimeout(() => void loadPiModels(provider), 250);
}

async function loadPiModels(provider: string) {
	const request = ++piModelsRequest;
	piModelsLoading = true;
	piModelsError = "";
	try {
		const response = await fetch(
			`/api/pi/models/${encodeURIComponent(provider)}`,
		);
		if (!response.ok) {
			const body: unknown = await response.json().catch(() => null);
			const message =
				typeof body === "object" &&
				body !== null &&
				"error" in body &&
				typeof body.error === "string"
					? body.error
					: "Could not list Pi models";
			throw new Error(message);
		}
		const options: PiModelOption[] = await response.json();
		if (request === piModelsRequest && provider === piProvider)
			piModels = options;
	} catch (cause) {
		if (request === piModelsRequest && provider === piProvider) {
			piModels = [];
			piModelsError = `${cause instanceof Error ? cause.message : "Pi model list is unavailable"}; enter the model ID manually.`;
		}
	} finally {
		if (request === piModelsRequest) piModelsLoading = false;
	}
}

async function selectCredential(credentialId: string) {
	if (!draft) return;
	draft.credentialId = credentialId || null;
	draft.model = null;
	if (credentialId) await loadModels(credentialId);
	else models = [];
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
				<button class:selected={draft?.id === agent.id} class="agent-item" onclick={() => open(agent)}><span><strong>{agent.id}</strong><small>{agent.model || "No model selected"}</small></span></button>
			{:else}<p class="muted">No agents yet. Create a reusable role for your pipelines.</p>{/each}
		</aside>
		<section class="agent-editor">
			{#if !draft}
				<EmptyState title="Select or create an agent" description="Agents hold instructions, provider credentials, model and skills." />
			{:else}
				<form onsubmit={(event) => { event.preventDefault(); if (draft?.provider === "pi") { const provider = piProvider.trim(); const model = piModel.trim(); if (provider.includes("/")) { error = "A Pi provider ID cannot contain '/'. Enter the provider name only."; return; } if (provider && !/^[A-Za-z0-9_.-]+$/.test(provider)) { error = "Pi provider IDs may contain only letters, numbers, dots, hyphens and underscores."; return; } if (provider && !model) { error = "Select a Pi model for this provider, or select 'Use Pi configured default' to use Pi's configured default."; return; } if (!provider && model) { error = "Choose a provider for this model, or select Pi configured default."; return; } draft.model = provider && model ? `${provider}/${model}` : null; } save(); }}>
					<div class="editor-header"><div><span class="eyebrow">{editing ? "Editing agent" : "New agent"}</span>{#if !editing}<h2>{draft.id || "Untitled agent"}</h2>{/if}</div><div><Button variant="secondary" disabled={busy} onclick={() => { draft = null; error = ""; }}>Cancel</Button> <Button type="submit" loading={busy}>Save agent</Button></div></div>
					<section class="form-section" aria-labelledby="agent-runtime-heading"><header><span class="eyebrow">Runtime</span><h3 id="agent-runtime-heading">Identity and model</h3></header><div class="form-grid">
						<div class="agent-id-field"><Field label="Agent ID" for="agent-id" required hint="IDs cannot be changed after creation.">{#snippet children({ describedBy })}<input id="agent-id" bind:value={draft!.id} required disabled={busy || editing} aria-describedby={describedBy} />{/snippet}</Field></div>
						<Field label="Runtime" for="agent-provider" required hint="Pi uses its local CLI authentication and does not use a Baton credential.">{#snippet children({ describedBy })}<ResourceSelect id="agent-provider" value={draft!.provider} items={[{ id: "rig", name: "Rig" }, { id: "pi", name: "Pi CLI" }, { id: "fake", name: "Fake" }]} placeholder="Select runtime" label="Agent runtime" {describedBy} disabled={busy} onValueChange={(provider) => { if (draft) { draft.provider = provider as Agent["provider"]; if (provider === "pi" || provider === "fake") { draft.credentialId = null; draft.model = null; piProvider = ""; piModel = ""; piCustomSelected = false; piDefaultSelected = false; piModelsRequest++; clearTimeout(piModelDebounce); piModels = []; piModelsLoading = false; piModelsError = ""; } } }} />{/snippet}</Field>
						{#if draft.provider === "rig"}<div class="credential-field"><Field label="Credential" for="agent-credential" required hint="Credentials are managed in Settings and can be reused by agents.">{#snippet children({ describedBy, invalid })}<ResourceSelect id="agent-credential" value={draft!.credentialId ?? ""} items={availableCredentials} placeholder="Select a credential" label="Available credentials" {describedBy} {invalid} required disabled={busy} onValueChange={(credentialId) => void selectCredential(credentialId)} />{/snippet}</Field></div>
						<Field label="Model" for="agent-model" required hint="Available models are loaded from the provider.">{#snippet children({ describedBy, invalid })}<ResourceSelect id="agent-model" value={draft!.model?.split("/").slice(1).join("/") ?? ""} onValueChange={(model) => { if (draft) { const credential = (data.credentials as Credential[]).find((item) => item.id === draft?.credentialId); const type = (data.credentialTypes as CredentialType[]).find((item) => item.id === credential?.credentialType); draft.model = type?.provider && model ? `${type.provider}/${model}` : null; } }} items={models.map((model) => ({ id: model.id, name: model.label }))} placeholder={modelsLoading ? "Loading models…" : "Select a model"} label="Available models" {describedBy} {invalid} required disabled={busy || modelsLoading || !draft!.credentialId} />{#if modelsError}<small role="alert" class="status-error">{modelsError}</small>{/if}{/snippet}</Field>
					{:else if draft.provider === "pi"}<Field label="Pi provider" for="pi-provider" hint="Choose a documented provider or select Other / custom and enter its Pi provider ID.">{#snippet children({ describedBy })}<ResourceSelect id="pi-provider" value={piDefaultSelected ? "default" : piProviders.some((item) => item.id === piProvider) ? piProvider : piCustomSelected ? "custom" : ""} items={[{ id: "default", name: "Use Pi configured default" }, ...piProviders]} placeholder="Select Pi provider" label="Pi providers" {describedBy} disabled={busy} onValueChange={(value) => selectPiProvider(value, value === "custom")} />{#if piCustomSelected || (piProvider && !piProviders.some((item) => item.id === piProvider))}<input aria-label="Custom Pi provider ID" value={piProvider} disabled={busy} placeholder="Pi provider ID" oninput={(event) => { if (event.currentTarget.value !== piProvider) piModel = ""; piDefaultSelected = false; piProvider = event.currentTarget.value; schedulePiModelLoad(piProvider); }} />{/if}{/snippet}</Field><Field label="Pi model" for="pi-model-picker" hint="Select a locally listed model or enter an ID manually. Pi's list may be incomplete; use `pi --offline --list-models PROVIDER` on the runtime host as a fallback. Custom IDs are accepted. To use Pi's configured default, select &quot;Use Pi configured default&quot;; Baton then saves no provider or model. Authentication status: unknown; Pi may use local or environment credentials.">{#snippet children({ describedBy })}<ResourceSelect id="pi-model-picker" value={piModels.some((model) => model.id === piModel) ? piModel : ""} items={piModels.map((model) => ({ id: model.id, name: model.label }))} placeholder={piModelsLoading ? "Loading Pi models…" : "Available Pi models"} label="Pi models" {describedBy} disabled={busy || piModelsLoading || !piModels.length} onValueChange={(model) => piModel = model} />{#if piModelsError}<small role="alert" class="status-error">{piModelsError}</small>{:else if piProvider && !piModelsLoading && !piModels.length}<small role="status">No models were listed; enter a model ID manually or select "Use Pi configured default".</small>{/if}<input id="pi-model" aria-label="Pi model ID (manual)" bind:value={piModel} disabled={busy} aria-describedby={describedBy} placeholder="Enter model ID, e.g. claude-sonnet-4" />{/snippet}</Field>{/if}</div></section>
					<section class="form-section" aria-labelledby="agent-instructions-heading"><header><span class="eyebrow">Instructions</span><h3 id="agent-instructions-heading">Behavior</h3></header><Field label="Instructions" for="agent-prompt" required hint="Define this agent’s reusable role, scope, and response style.">{#snippet children({ describedBy })}<MultilineInput id="agent-prompt" name="instructions" bind:value={draft!.instructions} rows={6} placeholder="Describe this agent's role and behavior." disabled={busy} {describedBy} required />{/snippet}</Field>
					</section>
					<section class="references form-section" aria-label="Tools">
						<AssignedResources id="agent-add-tool" title="Tools" label="Assigned tools" addLabel="Add tool" emptyMessage="No tools assigned." items={tools.map((id) => ({ id }))} selected={draft!.allowedTools ?? []} disabled={busy} onChange={(selected) => { if (draft) draft.allowedTools = selected; }} />
						<p class="muted">Tool selection is not a security sandbox. Shell commands can access and modify files outside the worktree, even without write or edit. Steps with transitions require report_outcome.</p>
					</section>
					<section class="references form-section" aria-label="Skills">
						<AssignedResources id="agent-add-skill" title="Skills" label="Assigned skills" addLabel="Add skill" emptyMessage="No skills assigned." items={config.skills} selected={draft.skillIds} disabled={busy} onChange={(selected) => { if (draft) draft.skillIds = selected; }} />
					</section>
					{#if editing}<div class="delete-row"><Button variant="danger" disabled={busy} onclick={() => deleting = true}><Trash2 size={16} /> Delete agent</Button></div>{/if}
				</form>
				<ConfirmationDialog bind:open={deleting} title="Delete agent?" loading={busy} onconfirm={() => save(true)}>This cannot be undone. Agents used by a pipeline cannot be deleted until the pipeline step is changed or removed.</ConfirmationDialog>
			{/if}
		</section>
	</div>
{/if}

<style>
	.agents-layout { display:grid; grid-template-columns:15rem minmax(0,1fr); min-height:calc(100vh - 12rem); border:1px solid #303137; border-radius:.7rem; background:#1b1c20; overflow:hidden; box-shadow:0 12px 30px #0002; }.agent-list { padding:1rem; background:#16171b; border-right:1px solid #303137; }.list-heading { display:flex; justify-content:space-between; margin-bottom:.8rem; }.list-heading span { background:#28344f; border-radius:999px; padding:.1rem .45rem; color:#aac2ff; font-size:.8rem; }.agent-item { display:flex; gap:.65rem; width:100%; padding:.7rem; border:1px solid transparent; border-radius:.5rem; background:transparent; text-align:left; color:#dedee2; cursor:pointer; }.agent-item:hover,.agent-item.selected { border-color:#3c4a68; background:#242a38; }.agent-item span { min-width:0; }.agent-item strong,.agent-item small { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }.agent-item small { color:#92939b; font-size:.75rem; }.agent-editor { padding:1.5rem; }.editor-header { display:flex; align-items:start; justify-content:space-between; gap:1rem; margin-bottom:1.5rem; }.editor-header h2 { margin:.2rem 0 0; }.form-section { margin-top:1.5rem; padding-top:1.25rem; border-top:1px solid #303137; }.editor-header + .form-section { margin-top:0; }.form-section > header { margin-bottom:1rem; }.form-section h3 { margin:.15rem 0 0; color:#e8e8eb; font-size:.95rem; }.form-grid { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:1rem; }.agent-id-field { grid-column:1/-1; }.form-grid :global(.field) { display:grid; grid-template-rows:auto 2.6rem auto; margin:0; }.form-grid :global(.field-hint) { margin:.3rem 0; }.references .muted { font-size:.85rem; }.delete-row { display:flex; justify-content:flex-end; margin-top:1.5rem; padding-top:1rem; border-top:1px solid #303137; } @media (max-width:720px) { .agents-layout { min-height:0; } .agents-layout,.form-grid { grid-template-columns:1fr; }.form-grid :global(.field) { grid-template-rows:auto auto auto; }.agent-list { border-right:0; border-bottom:1px solid #303137; }.editor-header { flex-direction:column; } }
</style>
