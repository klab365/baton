<script lang="ts">
import { onMount } from "svelte";
import Plus from "@lucide/svelte/icons/plus";
import KeyRound from "@lucide/svelte/icons/key-round";
import Button from "../../components/Button.svelte";
import ConfirmationDialog from "../../components/ConfirmationDialog.svelte";
import PageHeader from "../../components/PageHeader.svelte";
import ResourceSelect from "../../components/ResourceSelect.svelte";
let { data } = $props();
type Credential = {
	id: string;
	name: string;
	credentialType: string;
	status: "available" | "missing_secret" | "unavailable";
};
type CredentialType = {
	id: string;
	label: string;
	authentication: "secret_input" | "device_code";
	supportsTest: boolean;
	configurationFields: {
		id: string;
		label: string;
		required: boolean;
		secret: boolean;
	}[];
};
let credentials = $state<Credential[]>([]);
let credentialTypes = $state<CredentialType[]>([]);
let credentialName = $state("");
let credentialType = $state("");
let credentialSecret = $state("");
let credentialConfiguration = $state<Record<string, string>>({});
let authCredentialId = $state("");
let authStatus = $state<{
	status: string;
	verificationUri?: string;
	userCode?: string;
	message?: string;
} | null>(null);
let authTimer: ReturnType<typeof setTimeout> | undefined;
let error = $state("");
let busy = $state(false);
let showForm = $state(false);
let credentialToDelete = $state<Credential | null>(null);
let deleteDialogOpen = $state(false);
async function load() {
	busy = true;
	try {
		const [credentialResponse, typeResponse] = await Promise.all([
			fetch("/api/credentials"),
			fetch("/api/credential-types"),
		]);
		if (!credentialResponse.ok || !typeResponse.ok)
			throw new Error("Could not load credentials");
		credentials = await credentialResponse.json();
		credentialTypes = await typeResponse.json();
		credentialType ||= credentialTypes[0]?.id ?? "";
	} catch (cause) {
		error =
			cause instanceof Error ? cause.message : "Credential loading failed";
	} finally {
		busy = false;
	}
}
async function pollAuthentication() {
	if (!authCredentialId) return;
	try {
		const response = await fetch(
			`/api/credentials/${encodeURIComponent(authCredentialId)}/authentication`,
		);
		if (!response.ok) throw new Error("Could not read authentication status");
		authStatus = await response.json();
		if (authStatus?.status === "available") await load();
		else if (["pending", "not_started"].includes(authStatus?.status ?? ""))
			authTimer = setTimeout(pollAuthentication, 2000);
	} catch (cause) {
		error =
			cause instanceof Error ? cause.message : "Authentication status failed";
	}
}
async function startAuthentication() {
	if (!credentialName.trim()) return;
	error = "";
	if (authStatus?.status !== "failed" || !authCredentialId)
		authCredentialId = crypto.randomUUID();
	authStatus = { status: "starting" };
	try {
		const response = await fetch(
			`/api/credentials/${encodeURIComponent(authCredentialId)}/authenticate`,
			{
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ name: credentialName.trim() }),
			},
		);
		if (!response.ok)
			throw new Error(
				(await response.json()).error ?? "Could not start authentication",
			);
		authStatus = await response.json();
		credentialName = "";
		await load();
		if (authStatus?.status === "pending")
			authTimer = setTimeout(pollAuthentication, 1500);
	} catch (cause) {
		error = cause instanceof Error ? cause.message : "Authentication failed";
	}
}
async function saveCredential() {
	if (!credentialName.trim() || !credentialType || !credentialSecret) return;
	await mutate(
		`/api/credentials/${encodeURIComponent(crypto.randomUUID())}`,
		"PUT",
		{
			name: credentialName.trim(),
			credentialType,
			configuration: credentialConfiguration,
			secret: credentialSecret,
		},
	);
	credentialName = "";
	credentialSecret = "";
	credentialConfiguration = {};
}
async function deleteCredential(credential: Credential) {
	await mutate(
		`/api/credentials/${encodeURIComponent(credential.id)}`,
		"DELETE",
		{},
	);
	credentialToDelete = null;
	deleteDialogOpen = false;
}
async function mutate(url: string, method: string, body: unknown) {
	busy = true;
	error = "";
	try {
		const response = await fetch(url, {
			method,
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		});
		if (!response.ok)
			throw new Error(
				(await response.json()).error ?? "Could not update credential",
			);
		await load();
	} catch (cause) {
		error = cause instanceof Error ? cause.message : "Credential update failed";
	} finally {
		busy = false;
	}
}
onMount(() => {
	void load();
	return () => clearTimeout(authTimer);
});
</script>

<svelte:head><title>Settings · Baton</title></svelte:head>
<PageHeader title="Settings" />
{#if error && !showForm}<p role="alert" class="error">{error}</p>{/if}
{#if showForm}
	<div class="dialog-backdrop" role="presentation">
		<div class="dialog credential-dialog" role="dialog" aria-modal="true" aria-labelledby="add-credential-title">
			<header class="dialog-header"><span class="dialog-icon credential-dialog-icon" aria-hidden="true"><KeyRound size={19} /></span><div><span class="eyebrow">Provider access</span><h2 id="add-credential-title">Add credential</h2></div></header>
			<div class="dialog-content"><p class="credential-description">Connect a provider for use by your agents. Secrets remain stored locally.</p>{#if error}<p role="alert" class="error">{error}</p>{/if}<div class="credential-builder">
			<label for="credential-type">Credential type</label>
			<ResourceSelect id="credential-type" bind:value={credentialType} items={credentialTypes.map((type) => ({ id: type.id, name: type.label }))} placeholder="Select a credential type" label="Credential types" showId={false} disabled={busy} onValueChange={() => credentialConfiguration = {}} />
			{#each credentialTypes.find((type) => type.id === credentialType)?.configurationFields ?? [] as field}
				<label for={`credential-config-${field.id}`}>{field.label}</label>
				<input id={`credential-config-${field.id}`} bind:value={credentialConfiguration[field.id]} placeholder={field.id === "baseUrl" ? "http://localhost:11434" : ""} disabled={busy} />
			{/each}
			<label for="credential-name">Name</label>
			<input id="credential-name" bind:value={credentialName} placeholder="e.g. Personal OpenAI" disabled={busy} />
			{#if credentialTypes.find((type) => type.id === credentialType)?.authentication === "device_code"}
				{#if authStatus}<p role="status">{authStatus.message ?? authStatus.status}{#if authStatus.verificationUri}<br /><a href={authStatus.verificationUri} target="_blank" rel="noreferrer">Open sign-in page</a>{/if}{#if authStatus.userCode}<br /><strong>Code: {authStatus.userCode}</strong>{/if}</p>{/if}
			{:else}
				<label for="credential-secret">Secret</label>
				<input id="credential-secret" type="password" bind:value={credentialSecret} autocomplete="off" disabled={busy} />
			{/if}
			</div></div>
			<div class="dialog-actions">
				<Button variant="quiet" disabled={busy} onclick={() => (showForm = false)}>Cancel</Button>
				{#if credentialTypes.find((type) => type.id === credentialType)?.authentication === "device_code"}
					<Button disabled={busy || !credentialName.trim() || authStatus?.status === "pending"} onclick={startAuthentication}>Connect account</Button>
				{:else}
					<Button disabled={busy || !credentialName.trim() || !credentialType || !credentialSecret} onclick={saveCredential}>Save credential</Button>
				{/if}
			</div>
		</div>
	</div>
{/if}
<section class="credentials-section" aria-labelledby="saved-credentials">
	<header><div class="credential-heading"><h2 id="saved-credentials">Credentials</h2><span>{credentials.length}</span></div><div class="credential-section-actions"><Button disabled={busy} onclick={() => (showForm = true)}><Plus size={16} /> Add credential</Button></div></header>
	<div class="credential-list">
		{#each credentials as credential (credential.id)}
			<article class="credential-card"><div><strong>{credential.name}</strong><code>{credential.credentialType}</code><span class="credential-status">{credential.status.replaceAll("_", " ")}</span></div><Button variant="danger" disabled={busy} onclick={() => { credentialToDelete = credential; deleteDialogOpen = true; }}>Delete</Button></article>
		{:else}<p class="empty-hint">No provider credentials yet. Add one to connect an agent.</p>{/each}
	</div>
</section>
<ConfirmationDialog bind:open={deleteDialogOpen} title="Delete credential?" loading={busy} onconfirm={() => credentialToDelete && deleteCredential(credentialToDelete)}>Agents using this credential will no longer be able to start new runs until you assign another available credential.</ConfirmationDialog>

<style>
	.credential-dialog-icon { color:var(--link); background:var(--surface-raised); }
	.credential-description { margin:0 0 1rem; }
	.credential-builder { display:grid; gap:.45rem; }
	.credential-builder label { margin-top:.35rem; color:var(--text); font-size:.86rem; font-weight:650; }
	.credential-list { display:grid; gap:.65rem; }
	.credential-card { display:flex; align-items:center; gap:1rem; padding:.85rem 1rem; border:1px solid var(--border); border-radius:.65rem; background:var(--surface-raised); }
	.credential-card > div { display:grid; gap:.18rem; min-width:0; flex:1; }
	.credential-card code,.credential-card span { overflow-wrap:anywhere; color:var(--text-muted); font-size:.78rem; }
	.empty-hint,.error { padding:.85rem; border-radius:.5rem; background:var(--surface-raised); color:var(--text-muted); }
	.error { background:#3a2024; color:#ffd9dc; }
	.credentials-section { margin-top:1.5rem; }.credentials-section > header { display:flex; align-items:end; justify-content:space-between; gap:1rem; margin-bottom:.8rem; }.credentials-section h2 { margin:.15rem 0 0; color:#ededf0; font-size:1rem; }.credential-heading { display:flex; align-items:center; gap:.55rem; }.credential-heading > span { border-radius:999px; padding:.15rem .5rem; background:#28344f; color:#aac2ff; font-size:.75rem; }.credential-section-actions { display:flex; align-items:center; gap:.7rem; }
</style>
