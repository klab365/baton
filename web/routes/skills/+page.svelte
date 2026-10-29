<script lang="ts">
import { invalidateAll } from "$app/navigation";
import Plus from "@lucide/svelte/icons/plus";
import Trash2 from "@lucide/svelte/icons/trash-2";
import Button from "../../components/Button.svelte";
import ConfirmationDialog from "../../components/ConfirmationDialog.svelte";
import EmptyState from "../../components/EmptyState.svelte";
import Field from "../../components/Field.svelte";
import PageHeader from "../../components/PageHeader.svelte";
type Skill = {
	id: string;
	description: string;
	instructions: string;
};
type Config = { skills: Skill[] };
let { data } = $props();
const config = $derived(data.configuration as Config | null);
let draft = $state<Skill | null>(null);
let editing = $state(false);
let error = $state("");
let message = $state("");
let busy = $state(false);
let deleting = $state(false);
let importOpen = $state(false);
let githubUrl = $state("");
function splitSkill(source: string) {
	const match = source.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
	if (!match) return { description: "", instructions: source };
	const lines = match[1].split(/\r?\n/);
	const index = lines.findIndex((line) => line.startsWith("description:"));
	let description = "";
	if (index >= 0) {
		const value = lines[index].slice("description:".length).trim();
		if (value === "|" || value === "|-" || value === "|+") {
			description = lines
				.slice(index + 1)
				.filter((line) => line.startsWith("  "))
				.map((line) => line.slice(2))
				.join("\n");
		} else description = value.replace(/^['\"]|['\"]$/g, "");
	}
	return { description, instructions: match[2] };
}
function open(skill?: Skill) {
	if (!config) return;
	editing = !!skill;
	draft = skill
		? structuredClone(skill)
		: { id: "", description: "", instructions: "" };
	error = "";
}
async function save(remove = false) {
	if (!config || !draft) return;
	busy = true;
	error = "";
	try {
		const response = await fetch(
			`/api/skills${editing ? `/${encodeURIComponent(draft.id)}` : ""}`,
			{
				method: remove ? "DELETE" : editing ? "PUT" : "POST",
				headers: { "Content-Type": "application/json" },
				body: remove ? undefined : JSON.stringify({ skill: draft }),
			},
		);
		if (!response.ok)
			throw new Error((await response.json()).error ?? "Could not save skill.");
		draft = null;
		deleting = false;
		await invalidateAll();
		message = remove ? "Skill deleted." : "Skill saved.";
	} catch (cause) {
		error = cause instanceof Error ? cause.message : "Could not save skill.";
	} finally {
		busy = false;
	}
}
async function install() {
	const url = githubUrl.trim();
	if (!url || !config) return;
	busy = true;
	error = "";
	try {
		const parsed = new URL(url);
		const parts = parsed.pathname.split("/").filter(Boolean);
		if (
			parsed.hostname !== "github.com" ||
			parts.length < 5 ||
			parts[2] !== "tree"
		)
			throw new Error("Enter a GitHub repository folder URL.");
		const [, repo, , branch, ...folder] = parts;
		const response = await fetch(
			`https://api.github.com/repos/${parts[0]}/${repo}/contents/${folder.join("/")}/SKILL.md?ref=${encodeURIComponent(branch)}`,
		);
		if (!response.ok)
			throw new Error("Could not find SKILL.md at that GitHub folder.");
		const file = await response.json();
		const source = new TextDecoder().decode(
			Uint8Array.from(atob(file.content.replaceAll("\n", "")), (character) =>
				character.charCodeAt(0),
			),
		);
		const id = folder.at(-1) ?? repo;
		const skillContent = splitSkill(source);
		const saved = await fetch("/api/skills", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ skill: { id, ...skillContent } }),
		});
		if (!saved.ok)
			throw new Error((await saved.json()).error ?? "Could not install skill.");
		await invalidateAll();
		message = "Skill imported from GitHub.";
		githubUrl = "";
		importOpen = false;
	} catch (cause) {
		error = cause instanceof Error ? cause.message : "Import failed.";
	} finally {
		busy = false;
	}
}
</script>
<svelte:head><title>Skills · Baton</title></svelte:head>
<PageHeader title="Skills" description="Create reusable instructions or import a SKILL.md from GitHub.">{#snippet actions()}<Button onclick={() => importOpen = true}>Download from GitHub</Button> <Button disabled={!config} onclick={() => open()}><Plus size={17} /> Create skill</Button>{/snippet}</PageHeader>
{#if error}<p role="alert" class="status status-error">{error}</p>{/if}{#if message}<p role="status" class="status status-success">{message}</p>{/if}
{#if importOpen}<div class="dialog-backdrop" role="presentation"><div class="dialog" role="dialog" aria-modal="true" aria-labelledby="import-skill-title"><header class="dialog-header"><div><span class="eyebrow">GitHub import</span><h2 id="import-skill-title">Download a skill</h2></div></header><form onsubmit={(event) => { event.preventDefault(); install(); }}><div class="dialog-content"><Field label="GitHub skill folder URL" for="github-skill-url" required hint="The folder must contain SKILL.md.">{#snippet children({ describedBy })}<input id="github-skill-url" bind:value={githubUrl} placeholder="https://github.com/owner/repository/tree/main/skills/example" required disabled={busy} aria-describedby={describedBy} />{/snippet}</Field></div><div class="dialog-actions"><Button variant="quiet" type="button" disabled={busy} onclick={() => importOpen = false}>Cancel</Button><Button type="submit" loading={busy}>Download skill</Button></div></form></div></div>{/if}
{#if !config}<EmptyState title="Configuration unavailable" description="Initialize the project before managing skills." />{:else}<div class="layout"><aside><strong>Project skills · {config.skills.length}</strong>{#each config.skills as skill (skill.id)}<button class:selected={draft?.id === skill.id} onclick={() => open(skill)}><b>{skill.id}</b></button>{:else}<p>No skills installed yet.</p>{/each}</aside><section>{#if draft}<form onsubmit={(event) => { event.preventDefault(); save(); }}><span class="eyebrow">{editing ? "Edit skill" : "New skill"}</span><Field label="Skill ID" for="skill-id" required>{#snippet children()}<input id="skill-id" bind:value={draft!.id} disabled={busy || editing} required />{/snippet}</Field><Field label="Description" for="skill-description" required>{#snippet children()}<input id="skill-description" bind:value={draft!.description} required disabled={busy} />{/snippet}</Field><Field label="Instructions" for="skill-instructions" required>{#snippet children()}<textarea id="skill-instructions" bind:value={draft!.instructions} rows={18} required disabled={busy}></textarea>{/snippet}</Field><Button type="submit" loading={busy}>Save skill</Button> <Button variant="secondary" onclick={() => draft = null}>Cancel</Button>{#if editing}<Button variant="danger" disabled={busy} onclick={() => deleting = true}><Trash2 size={16} /> Delete</Button>{/if}</form><ConfirmationDialog bind:open={deleting} title="Delete skill?" loading={busy} onconfirm={() => save(true)}>This removes the skill and its files from <code>.agents/skills/</code>. Remove it from assigned agents first.</ConfirmationDialog>{:else}<EmptyState title="Select or create a skill" description="Skills are reusable instructions assigned to agents." />{/if}</section></div>{/if}
<style>.layout{display:grid;grid-template-columns:16rem 1fr;min-height:30rem;border:1px solid #303137;border-radius:.7rem;background:#1b1c20;overflow:hidden}aside{padding:1rem;background:#16171b;border-right:1px solid #303137}aside>button{display:block;width:100%;margin-top:.5rem;padding:.7rem;text-align:left;color:#dedee2;background:transparent;border:1px solid transparent;border-radius:.5rem;cursor:pointer}aside>button.selected,aside>button:hover{background:#242a38;border-color:#3c4a68}section{padding:1.5rem}form{display:grid;gap:1rem}textarea{width:100%;font:inherit} @media(max-width:700px){.layout{grid-template-columns:1fr}aside{border-right:0;border-bottom:1px solid #303137}}</style>
