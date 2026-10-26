<script lang="ts">
import Button from "../../components/Button.svelte";
import DetailPanel from "../../components/DetailPanel.svelte";
import PageHeader from "../../components/PageHeader.svelte";
import type { RuntimeProfile } from "../../lib/generated/RuntimeProfile";
import type { RuntimeKind } from "../../lib/generated/RuntimeKind";
let { data } = $props();
let profiles = $state<RuntimeProfile[]>([]);
let revision = $state("");
let error = $state("");
let busy = $state(false);
let provider = $state("");
let model = $state("");
let name = $state("");
async function load() {
	busy = true;
	error = "";
	try {
		const [listed, agents] = await Promise.all([
			fetch("/api/runtime-profiles"),
			fetch("/api/agents"),
		]);
		if (!listed.ok || !agents.ok)
			throw new Error("Could not load project runtime configuration");
		profiles = await listed.json();
		revision = (await agents.json()).revision;
	} catch (cause) {
		error =
			cause instanceof Error ? cause.message : "Runtime configuration failed";
	} finally {
		busy = false;
	}
}
async function save() {
	const runtimeKind = "rig" as RuntimeKind;
	const profile: RuntimeProfile = {
		id: `rig.${provider}.${model}`,
		name: name.trim() || `Rig · ${provider} · ${model}`,
		runtimeKind,
		model: { provider, model },
	};
	await mutate(
		`/api/runtime-profiles/${encodeURIComponent(profile.id)}`,
		"PUT",
		{ revision, profile },
	);
}
async function remove(profile: RuntimeProfile) {
	await mutate(
		`/api/runtime-profiles/${encodeURIComponent(profile.id)}`,
		"DELETE",
		{ revision },
	);
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
				(await response.json()).error ?? "Could not update runtime profiles",
			);
		provider = "";
		model = "";
		name = "";
		await load();
	} catch (cause) {
		error =
			cause instanceof Error ? cause.message : "Runtime profile update failed";
	} finally {
		busy = false;
	}
}
$effect(() => {
	load();
});
</script>

<svelte:head><title>Settings · Baton</title></svelte:head>
<PageHeader title="Settings" description="Configure reusable runtime/provider/model combinations in project configuration." />
<DetailPanel title="Project details"><dl><dt>Name</dt><dd>{data.project.name}</dd><dt>Directory</dt><dd><code>{data.project.directory}</code></dd></dl></DetailPanel>
<section>
	<h2>Runtime profiles</h2>
	<p class="intro">Create a reusable embedded-runtime choice, then assign it to one or more agents. Baton never stores provider tokens.</p>
	{#if error}<p role="alert" class="error">{error}</p>{/if}
	<div class="profile-builder">
		<div class="builder-heading"><span class="step">1</span><div><h3>Describe a Rig model</h3><p>Provider credentials and model discovery will be supplied by the Rig integration.</p></div></div>
		<label for="runtime-provider">Provider</label>
		<input id="runtime-provider" bind:value={provider} placeholder="e.g. openai" disabled={busy} />
		<label for="runtime-model">Model</label>
		<input id="runtime-model" bind:value={model} placeholder="e.g. gpt-4.1" disabled={busy} />
		{#if provider && model}
			<label for="profile-name">Profile name <span>(optional)</span></label>
			<input id="profile-name" bind:value={name} placeholder={`Rig · ${provider} · ${model}`} disabled={busy} />
			<div class="profile-preview"><span>Profile ID</span><code>rig.{provider.replace(/[^a-zA-Z0-9._-]/g, "_")}.{model.replace(/[^a-zA-Z0-9._-]/g, "_")}</code></div>
			<Button disabled={busy} onclick={save}>Add runtime profile</Button>
		{/if}
	</div>
	<div class="profiles-heading"><div><h3>Configured profiles</h3><p>Stored in <code>.baton/config.json</code>; assign profiles in the Agents page.</p></div><span>{profiles.length}</span></div>
	{#if !profiles.length}<p class="empty-hint">No runtime profiles yet. Add a provider and model above.</p>{/if}
	<div class="profile-list">
		{#each profiles as profile (profile.id)}
			<article class="profile-card"><div class="profile-icon">{profile.runtimeKind === "rig" ? "R" : "F"}</div><div class="profile-info"><strong>{profile.name}</strong><code>{profile.id}</code><span>{profile.model ? `${profile.model.provider} · ${profile.model.model}` : `${profile.runtimeKind} default`}</span></div><Button variant="danger" disabled={busy} onclick={() => remove(profile)}>Delete</Button></article>
		{/each}
	</div>
</section>

<style>
	.intro,.builder-heading p,.profiles-heading p { color:#69758b; line-height:1.5; }
	.profile-builder { max-width:44rem; margin:1.5rem 0 2.5rem; padding:1.25rem; border:1px solid #dbe0e9; border-radius:.75rem; background:#fff; box-shadow:0 3px 12px rgb(33 45 75 / 5%); }
	.builder-heading { display:flex; align-items:center; gap:.75rem; margin-bottom:.9rem; }.step { display:grid; place-items:center; width:1.8rem; height:1.8rem; flex:none; border-radius:50%; background:#edf2ff; color:#244aab; font-weight:700; }.builder-heading h3,.profiles-heading h3 { margin:0; font-size:1rem; }.builder-heading p,.profiles-heading p { margin:.2rem 0 0; font-size:.82rem; }
	.profile-builder label { display:block; margin:.65rem 0 .35rem; color:#27324a; font-size:.86rem; font-weight:650; }.profile-builder label span { color:#69758b; font-weight:400; }.profile-builder input { width:100%; min-height:2.8rem; padding:.65rem .75rem; border:1px solid #cbd2df; border-radius:.45rem; background:#fff; color:#172033; font:inherit; }.profile-builder input:focus { outline:3px solid rgb(48 85 181 / 16%); border-color:#3055b5; }.profile-builder :global(.button) { margin-top:1rem; }.profile-preview { display:flex; flex-wrap:wrap; gap:.5rem; align-items:center; margin-top:.8rem; color:#69758b; font-size:.8rem; }.profile-preview code { color:#27324a; }
	.profiles-heading { display:flex; align-items:center; justify-content:space-between; margin:1.5rem 0 .85rem; }.profiles-heading > span { min-width:1.8rem; padding:.15rem .5rem; border-radius:999px; background:#edf2ff; color:#244aab; text-align:center; font-size:.82rem; }.profile-list { display:grid; gap:.65rem; }.profile-card { display:flex; align-items:center; gap:.85rem; padding:.85rem 1rem; border:1px solid #dbe0e9; border-radius:.65rem; background:#fff; }.profile-icon { display:grid; place-items:center; width:2.5rem; height:2.5rem; flex:none; border-radius:.6rem; background:#edf2ff; color:#244aab; font-size:1.3rem; font-weight:700; }.profile-info { display:grid; gap:.18rem; min-width:0; flex:1; }.profile-info strong { color:#27324a; }.profile-info code,.profile-info span { overflow-wrap:anywhere; color:#69758b; font-size:.78rem; }.empty-hint { padding:.85rem; border-radius:.5rem; background:#f8f9fc; color:#69758b; font-size:.86rem; line-height:1.5; }.error { padding:.75rem; border-radius:.5rem; background:#fff0ef; color:#b42318; }
</style>
