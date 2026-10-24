<script lang="ts">
import CheckCircle2 from "@lucide/svelte/icons/check-circle-2";
import KeyRound from "@lucide/svelte/icons/key-round";
import Link2 from "@lucide/svelte/icons/link-2";
import LoaderCircle from "@lucide/svelte/icons/loader-circle";
import Pencil from "@lucide/svelte/icons/pencil";
import Plus from "@lucide/svelte/icons/plus";
import ShieldCheck from "@lucide/svelte/icons/shield-check";
import Trash2 from "@lucide/svelte/icons/trash-2";
import Unplug from "@lucide/svelte/icons/unplug";
import Button from "../../components/Button.svelte";
import DetailPanel from "../../components/DetailPanel.svelte";
import Field from "../../components/Field.svelte";
import PageHeader from "../../components/PageHeader.svelte";
let { data } = $props();
type Provider = {
	id: string;
	name: string;
	kind: "openai" | "chatgpt";
	defaultModel: string;
	baseUrl: string | null;
	models: string[];
	status: string;
};
let providers = $state<Provider[]>([]),
	message = $state(""),
	busy = $state(false),
	device = $state<{ verificationUri: string; userCode: string } | null>(null);
let editorOpen = $state(false);
let form = $state({
	id: "",
	name: "",
	kind: "openai",
	defaultModel: "gpt-4o-mini",
	baseUrl: "",
	apiKey: "",
});
async function request(path: string, method = "GET", body?: unknown) {
	const r = await fetch(`/api/providers${path}`, {
		method,
		headers: { "Content-Type": "application/json" },
		body: body ? JSON.stringify(body) : undefined,
	});
	if (r.status === 204) return null;
	if (!r.headers.get("content-type")?.includes("application/json")) {
		throw new Error(
			"Provider API unavailable. Restart the Baton Rust server and check BATON_API_URL.",
		);
	}
	const result = await r.json();
	if (!r.ok) throw new Error(result.error ?? "Provider request failed");
	return result;
}
async function load() {
	try {
		providers = await request("");
	} catch (e) {
		message = e instanceof Error ? e.message : "Could not load providers";
	}
}
async function save() {
	busy = true;
	message = "";
	try {
		await request(
			form.id ? `/${form.id}/provider` : "",
			form.id ? "PUT" : "POST",
			{
				name: form.name,
				kind: form.kind,
				defaultModel: form.defaultModel,
				baseUrl: form.baseUrl,
				apiKey: form.apiKey,
			},
		);
		reset();
		await load();
	} catch (e) {
		message = e instanceof Error ? e.message : "Could not save provider";
	} finally {
		busy = false;
	}
}
function edit(p: Provider) {
	editorOpen = true;
	form = {
		id: p.id,
		name: p.name,
		kind: p.kind,
		defaultModel: p.defaultModel,
		baseUrl: p.baseUrl ?? "",
		apiKey: "",
	};
	device = null;
}
function reset() {
	editorOpen = false;
	form = {
		id: "",
		name: "",
		kind: "openai",
		defaultModel: "gpt-4o-mini",
		baseUrl: "",
		apiKey: "",
	};
	device = null;
}
async function test(p: Provider) {
	busy = true;
	message = "";
	try {
		await request(`/${p.id}/test`, "POST");
		await load();
	} catch (e) {
		message = e instanceof Error ? e.message : "Connection test failed";
	} finally {
		busy = false;
	}
}
async function auth(p: Provider) {
	busy = true;
	message = "";
	device = null;
	try {
		await request(`/${p.id}/authenticate`, "POST");
		const deadline = Date.now() + 15000;
		while (Date.now() < deadline) {
			const [prompt, latest] = await Promise.all([
				request(`/${p.id}/prompt`),
				request(""),
			]);
			providers = latest;
			const current = latest.find((item: Provider) => item.id === p.id);
			if (prompt?.userCode) device = prompt;
			if (current?.status !== "authenticating") {
				if (current?.status === "available")
					message = "ChatGPT connected successfully.";
				else if (current) message = current.status;
				return;
			}
			if (prompt?.userCode) break;
			await new Promise((resolve) => setTimeout(resolve, 500));
		}
		void watchAuthentication(p.id);
	} catch (e) {
		message = e instanceof Error ? e.message : "Could not start authentication";
	} finally {
		busy = false;
	}
}
async function watchAuthentication(id: string) {
	while (true) {
		await new Promise((resolve) => setTimeout(resolve, 1500));
		try {
			const [latest, prompt] = await Promise.all([
				request(""),
				request(`/${id}/prompt`),
			]);
			providers = latest;
			if (prompt?.userCode) device = prompt;
			const current = latest.find((item: Provider) => item.id === id);
			if (!current || current.status !== "authenticating") {
				device = null;
				if (current?.status === "available")
					message = "ChatGPT connected successfully.";
				else if (current) message = current.status;
				return;
			}
		} catch (e) {
			message =
				e instanceof Error
					? e.message
					: "Could not check ChatGPT authentication status";
			return;
		}
	}
}
async function disconnect(p: Provider, remove = false) {
	if (remove && !confirm(`Delete ${p.name}?`)) return;
	busy = true;
	try {
		await request(
			`/${p.id}/${remove ? "provider" : "disconnect"}`,
			remove ? "DELETE" : "POST",
		);
		if (form.id === p.id) reset();
		await load();
	} catch (e) {
		message = e instanceof Error ? e.message : "Provider request failed";
	} finally {
		busy = false;
	}
}
function statusTone(status: string) {
	return status === "available"
		? "good"
		: status === "authenticating" || status === "untested"
			? "pending"
			: "muted";
}
function statusLabel(status: string) {
	if (status === "available") return "Connected";
	if (status === "authenticating") return "Waiting for ChatGPT sign-in";
	if (status === "untested") return "Not tested";
	if (status.startsWith("Authentication failed"))
		return "Sign-in failed — authenticate again";
	return status;
}
$effect(() => {
	load();
});
</script>

<svelte:head><title>Settings · Baton</title></svelte:head>
<PageHeader eyebrow="Settings" title="Project" description="Project details are read from the current Baton configuration." />
<DetailPanel title="Project details"><dl class="details"><dt>Name</dt><dd>{data.project.name}</dd><dt>Directory</dt><dd><code>{data.project.directory}</code></dd><dt>Configuration version</dt><dd>{data.project.status === "ready" ? data.project.config.schemaVersion : "–"}</dd></dl></DetailPanel>

<section class="provider-settings" aria-labelledby="providers-title">
 <div class="section-heading"><div><h2 id="providers-title">AI providers</h2><p class="muted">Configure the connections available on this computer.</p></div><Button disabled={busy} onclick={() => { reset(); editorOpen = true; }}><Plus size={17} /> Add provider</Button></div>
 <div class="security-note"><ShieldCheck size={20} /><p><strong>Credentials stay on this device.</strong> API keys and ChatGPT sessions are stored in private local storage, never in the project configuration. A ChatGPT subscription does not authorize OpenAI API usage.</p></div>
 {#if message}<p role="alert" class="status status-error">{message}</p>{/if}
 <div class="provider-layout" class:editing={editorOpen}>
  <div class="provider-list" aria-label="Configured providers">
   {#each providers as p (p.id)}
    <article class="provider-card" class:active={form.id === p.id}><header><div class="provider-icon" class:chatgpt={p.kind === "chatgpt"}>{#if p.kind === "openai"}<KeyRound size={20} />{:else}<Link2 size={20} />{/if}</div><div><h3>{p.name}</h3><span>{p.kind === "openai" ? "OpenAI API" : "ChatGPT session"}</span></div><span class:good={statusTone(p.status) === "good"} class:pending={statusTone(p.status) === "pending"} class="provider-status">{#if p.status === "available"}<CheckCircle2 size={14} />{:else if p.status === "authenticating"}<LoaderCircle size={14} />{/if}{statusLabel(p.status)}</span></header><dl><div><dt>Default model</dt><dd>{p.defaultModel}</dd></div><div><dt>Available models</dt><dd>{p.models.length || "Not verified"}</dd></div></dl><footer><Button variant="quiet" onclick={() => edit(p)}><Pencil size={15} /> Edit</Button><Button variant="secondary" disabled={busy} onclick={() => p.kind === "openai" ? test(p) : auth(p)}>{p.kind === "openai" ? "Test connection" : "Authenticate"}</Button><button class="card-menu" aria-label={`Delete ${p.name}`} disabled={busy} onclick={() => disconnect(p, true)}><Trash2 size={16} /></button></footer></article>
   {:else}<div class="provider-empty"><KeyRound size={25} /><strong>No providers yet</strong><span>Add an OpenAI API connection or a ChatGPT session to make models available locally.</span></div>{/each}
  </div>
  {#if editorOpen}<form class="provider-form" onsubmit={(event) => { event.preventDefault(); save(); }}><div class="form-heading"><div><h2>{form.id ? form.name || "Provider" : "Add a provider"}</h2></div><Button variant="quiet" disabled={busy} onclick={reset}>Cancel</Button></div>
   <Field label="Provider name" for="provider-name" required>{#snippet children({ describedBy })}<input id="provider-name" bind:value={form.name} required maxlength="100" aria-describedby={describedBy} placeholder="e.g. Personal OpenAI" disabled={busy} />{/snippet}</Field>
   <Field label="Provider type" for="provider-kind" required hint="Provider type cannot be changed after saving.">{#snippet children({ describedBy })}<div class="select-control"><select id="provider-kind" bind:value={form.kind} disabled={busy || !!form.id} aria-describedby={describedBy}><option value="openai">OpenAI API</option><option value="chatgpt">ChatGPT session</option></select></div>{/snippet}</Field>
   <Field label="Default model" for="provider-model" required hint="Only verified, available models can be used by agents and pipelines.">{#snippet children({ describedBy })}<input id="provider-model" bind:value={form.defaultModel} required aria-describedby={describedBy} disabled={busy} placeholder="gpt-4o-mini" />{/snippet}</Field>
   {#if form.kind === "openai"}<Field label="API base URL" for="provider-base-url" hint="Optional. Leave blank for OpenAI’s standard API endpoint.">{#snippet children({ describedBy })}<input id="provider-base-url" type="url" bind:value={form.baseUrl} aria-describedby={describedBy} disabled={busy} placeholder="https://api.openai.com/v1" />{/snippet}</Field><Field label="API key" for="provider-api-key" hint={form.id ? "Leave blank to retain the current API key." : "Stored locally after saving; it is never shown again."}>{#snippet children({ describedBy })}<input id="provider-api-key" type="password" bind:value={form.apiKey} autocomplete="off" aria-describedby={describedBy} disabled={busy} />{/snippet}</Field>{:else}<div class="auth-hint"><Link2 size={18} /><p>Save this provider, then select <strong>Authenticate</strong> to start the secure device-code flow.</p></div>{/if}
   <div class="form-actions"><Button type="submit" loading={busy}>Save provider</Button>{#if form.id}<Button variant="secondary" disabled={busy} onclick={() => disconnect(providers.find((p) => p.id === form.id)!, false)}><Unplug size={16} /> Disconnect</Button>{/if}</div>
  </form>{/if}
 </div>
 {#if device}<aside class="device-flow"><div class="device-icon"><Link2 size={20} /></div><div><strong>Finish ChatGPT authentication</strong><p>After entering the code, keep this page open while Baton verifies the session. Open <a href={device.verificationUri} target="_blank" rel="noreferrer">the verification page</a> and enter this one-time code. Do not share it.</p><code>{device.userCode}</code></div></aside>{/if}
</section>

<style>
 .provider-settings { margin-top:2rem; }.provider-settings :global(.button) { display:inline-flex; align-items:center; justify-content:center; gap:.45rem; }.provider-settings :global(svg) { flex-shrink:0; }.section-heading,.form-heading { display:flex; align-items:flex-end; justify-content:space-between; gap:1rem; margin-bottom:1.25rem; }.section-heading h2,.form-heading h2 { margin:.2rem 0 0; }.section-heading .muted { margin:.35rem 0 0; }.security-note { display:flex; gap:.65rem; align-items:flex-start; margin-bottom:1.25rem; padding:.85rem 0; color:#647087; font-size:.85rem; }.security-note :global(svg) { flex:none; color:#3055b5; margin-top:.1rem; }.security-note p { margin:0; line-height:1.5; }
 .provider-layout { display:grid; grid-template-columns:1fr; gap:1rem; align-items:start; }.provider-layout.editing { grid-template-columns:minmax(0,1fr) minmax(19rem,1fr); }.provider-list { display:grid; gap:.75rem; }.provider-card,.provider-form { border:1px solid #dbe0e9; border-radius:.7rem; background:#fff; box-shadow:0 3px 10px rgb(33 45 75 / 4%); }.provider-card { overflow:hidden; }.provider-card.active { border-color:#9eb2e8; box-shadow:0 0 0 3px rgb(48 85 181 / 10%); }.provider-card header { display:grid; grid-template-columns:auto minmax(0,1fr) auto; gap:.75rem; align-items:center; padding:1rem 1.1rem .8rem; }.provider-icon { display:grid; place-items:center; width:2.5rem; height:2.5rem; color:#244aab; border-radius:.65rem; background:#edf2ff; }.provider-icon.chatgpt { color:#18794e; background:#ebf8f0; }.provider-card h3 { margin:0; font-size:1rem; }.provider-card header span:not(.provider-status) { color:#69758b; font-size:.78rem; }.provider-status { display:inline-flex; align-items:center; gap:.3rem; max-width:11rem; color:#647087; font-size:.72rem; line-height:1.25; text-align:right; }.provider-status.good { color:#18794e; }.provider-status.pending { color:#a15c00; }.provider-status.pending :global(svg) { animation:spin 1s linear infinite; }.provider-card dl { display:grid; grid-template-columns:1fr 1fr; gap:1rem; margin:0; padding:.85rem 1.1rem; border-top:1px solid #edf0f4; background:#fafbfc; }.provider-card dt { margin:0; color:#69758b; font-size:.72rem; text-transform:uppercase; letter-spacing:.04em; }.provider-card dd { margin:.2rem 0 0; font-size:.86rem; font-weight:650; }.provider-card footer { display:flex; align-items:center; gap:.25rem; padding:.65rem .75rem; }.provider-card footer :global(.button) { padding:.45rem .6rem; font-size:.82rem; }.card-menu { display:grid; place-items:center; width:2.25rem; height:2.25rem; margin-left:auto; border:0; border-radius:.4rem; color:#7b5260; background:transparent; cursor:pointer; }.card-menu:hover:not(:disabled) { color:#b42318; background:#fff0ef; }.provider-empty { display:grid; justify-items:center; gap:.5rem; padding:3rem 2rem; border:1px dashed #b7c0d1; border-radius:.7rem; color:#69758b; text-align:center; line-height:1.5; }.provider-empty :global(svg) { color:#3055b5; }.provider-empty strong { color:#27324a; }
 .provider-form { position:sticky; top:1rem; padding:1.25rem; }.provider-form :global(.field:last-of-type) { margin-bottom:1.5rem; }.form-heading { align-items:start; }.form-heading :global(.button) { padding:.45rem .6rem; }.auth-hint { display:flex; gap:.65rem; margin:1rem 0 1.5rem; padding:.8rem; border-radius:.5rem; color:#3f557e; background:#f4f7ff; font-size:.87rem; line-height:1.45; }.auth-hint :global(svg) { flex:none; color:#3055b5; }.auth-hint p { margin:0; }.form-actions { display:flex; flex-wrap:wrap; gap:.6rem; padding-top:1rem; border-top:1px solid #e7eaf0; }.device-flow { display:flex; gap:1rem; align-items:flex-start; margin-top:1rem; padding:1rem 1.15rem; border:1px solid #ead5a3; border-radius:.7rem; background:#fff8e8; }.device-icon { display:grid; place-items:center; flex:none; width:2.3rem; height:2.3rem; border-radius:.55rem; color:#8a5a00; background:#ffedc6; }.device-flow strong { color:#624000; }.device-flow p { margin:.3rem 0 .75rem; color:#72520c; line-height:1.45; }.device-flow code { display:inline-block; padding:.45rem .7rem; border:1px dashed #c18a26; border-radius:.35rem; color:#4b3100; background:#fffdf7; font-size:1.15rem; font-weight:800; letter-spacing:.12em; }@keyframes spin { to { transform:rotate(360deg); } }
 @media (max-width:720px) { .provider-layout,.provider-layout.editing { grid-template-columns:1fr; }.provider-form { position:static; }.section-heading { align-items:start; flex-direction:column; }.provider-card header { grid-template-columns:auto 1fr; }.provider-status { grid-column:2; text-align:left; }.provider-card footer { flex-wrap:wrap; } }
</style>
