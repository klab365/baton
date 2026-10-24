<script lang="ts">
import { invalidateAll } from "$app/navigation";
import ArrowDown from "@lucide/svelte/icons/arrow-down";
import ArrowUp from "@lucide/svelte/icons/arrow-up";
import Box from "@lucide/svelte/icons/box";
import ChevronRight from "@lucide/svelte/icons/chevron-right";
import Cpu from "@lucide/svelte/icons/cpu";
import GripVertical from "@lucide/svelte/icons/grip-vertical";
import Plus from "@lucide/svelte/icons/plus";
import Trash2 from "@lucide/svelte/icons/trash-2";
import Button from "../../components/Button.svelte";
import ConfirmationDialog from "../../components/ConfirmationDialog.svelte";
import EmptyState from "../../components/EmptyState.svelte";
import Field from "../../components/Field.svelte";
import PageHeader from "../../components/PageHeader.svelte";
import ResourceSelect from "../../components/ResourceSelect.svelte";
import type { Agent } from "../../lib/generated/Agent";
import type { Pipeline } from "../../lib/generated/Pipeline";
import type { PipelineStep as Step } from "../../lib/generated/PipelineStep";
type PipelineConfiguration = {
	revision: string;
	pipelines: Pipeline[];
	agents: Agent[];
};
let { data } = $props();
let draft = $state<Pipeline | null>(null);
let revision = $state("");
let editing = $state(false);
let error = $state("");
let busy = $state(false);
let message = $state("");
let deleting = $state(false);
let expandedStepIds = $state<Set<string>>(new Set());
const config = $derived(data.configuration as PipelineConfiguration | null);
function open(pipeline?: Pipeline) {
	if (!config) return;
	draft = pipeline
		? structuredClone(pipeline)
		: { id: crypto.randomUUID(), name: "", steps: [] };
	expandedStepIds = new Set();
	revision = config.revision;
	editing = Boolean(pipeline);
	error = "";
	message = "";
}
function add() {
	if (!config) return;
	const id = crypto.randomUUID();
	draft?.steps.push({ id, name: "", agentId: config.agents[0]?.id ?? "" });
	expandedStepIds = new Set([...expandedStepIds, id]);
}
function toggleStep(id: string) {
	const next = new Set(expandedStepIds);
	next.has(id) ? next.delete(id) : next.add(id);
	expandedStepIds = next;
}
function move(index: number, delta: number) {
	if (!draft) return;
	const steps = [...draft.steps];
	[steps[index], steps[index + delta]] = [steps[index + delta], steps[index]];
	draft.steps = steps;
}
async function save(remove = false) {
	if (!draft) return;
	busy = true;
	error = "";
	message = "";
	try {
		const response = await fetch(
			`/api/pipelines${editing ? `/${encodeURIComponent(draft.id)}` : ""}`,
			{
				method: remove ? "DELETE" : editing ? "PUT" : "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({
					revision,
					...(remove ? {} : { pipeline: draft }),
				}),
			},
		);
		if (!response.ok) {
			const body = await response.json();
			throw new Error(body.error ?? "Could not save pipeline.");
		}
		draft = null;
		deleting = false;
		await invalidateAll();
		message = remove ? "Pipeline deleted." : "Pipeline saved.";
	} catch (cause) {
		error = cause instanceof Error ? cause.message : "Could not save pipeline.";
	} finally {
		busy = false;
	}
}
</script>

<svelte:head><title>Pipelines · Baton</title></svelte:head>
<PageHeader title="Pipelines" description="Build a clear, ordered handoff between your configured agents.">
 {#snippet actions()}<Button disabled={!config?.agents.length || busy} onclick={() => open()}><Plus size={17} /> Create pipeline</Button>{/snippet}
</PageHeader>
{#if error}<p role="alert" class="status status-error">{error}</p>{/if}
{#if message}<p role="status" class="status status-success">{message}</p>{/if}
{#if !config}
 <EmptyState title="Configuration unavailable" description="Initialize or repair project configuration in Settings before editing pipelines." />
{:else}
 <div class="pipeline-workbench">
  <aside class="pipeline-sidebar" aria-label="Pipeline library">
   <div class="sidebar-heading"><div><span class="eyebrow">Library</span><h2>Your pipelines</h2></div><span class="count">{config.pipelines.length}</span></div>
   <p class="muted">Pipelines use agents already defined in project configuration.</p>
   {#if !config.agents.length}
    <div class="agent-warning"><Cpu size={18} /><div><strong>No agents available</strong><span>Create and save an agent in Agents before creating a pipeline.</span></div></div>
   {/if}
   <div class="pipeline-list">
   {#each config.pipelines as pipeline (pipeline.id)}
    <button class:selected={draft?.id === pipeline.id} class="pipeline-item" onclick={() => open(pipeline)}><span class="pipeline-item-icon"><Box size={18} /></span><span><strong>{pipeline.name}</strong><small>{pipeline.steps.length} {pipeline.steps.length === 1 ? "step" : "steps"}</small></span><ChevronRight size={17} /></button>
   {:else}<div class="empty-library"><Box size={24} /><strong>No pipelines yet</strong><span>Create your first ordered agent handoff.</span></div>{/each}
   </div>
  </aside>
  <section class="editor" aria-label="Pipeline editor">
   {#if !draft}
    <div class="editor-placeholder"><div class="placeholder-icon"><Box size={28} /></div><h2>Select or create a pipeline</h2><p>Define an ordered sequence of agents. Each card receives the output from the one above it.</p><Button disabled={!config.agents.length} onclick={() => open()}><Plus size={17} /> Create pipeline</Button></div>
   {:else}
    <form onsubmit={(event) => { event.preventDefault(); save(); }}>
     <div class="editor-toolbar"><div><span class="eyebrow">{editing ? "Editing pipeline" : "New pipeline"}</span><Field label="Pipeline name" for="pipeline-name" required>{#snippet children({ describedBy })}<input id="pipeline-name" value={draft?.name ?? ""} oninput={(event) => { if (draft) draft.name = event.currentTarget.value; }} required aria-describedby={describedBy} disabled={busy} placeholder="e.g. Product delivery" />{/snippet}</Field></div><div class="toolbar-actions"><Button variant="secondary" disabled={busy} onclick={() => { draft = null; error = ""; }}>Cancel</Button><Button type="submit" loading={busy} disabled={!draft.steps.length || !config.agents.length}>Save changes</Button></div></div>
     <div class="flow-caption"><span>Task input</span><span>Each step hands its result to the next</span><span>Final output</span></div>
     <ol class="step-flow" aria-label="Ordered pipeline steps">
      {#each draft.steps as step, index (step.id)}
       <li class="step-node">
        <div class="flow-line" aria-hidden="true">{#if index > 0}<ArrowDown size={18} />{/if}</div>
        <article class="step-card">
         <header class="step-header"><div class="step-number">{index + 1}</div><button type="button" class="step-title" aria-expanded={expandedStepIds.has(step.id)} aria-controls={`step-details-${step.id}`} onclick={() => toggleStep(step.id)}><span>Agent step {index + 1}</span><strong>{step.name || "Untitled step"}</strong><small>{index === 0 ? "Receives task input" : "Receives output from previous step"}</small></button><div class="step-controls"><button type="button" class="icon-button" aria-label={`Move ${step.name || `step ${index + 1}`} up`} disabled={busy || index === 0} onclick={() => move(index, -1)}><ArrowUp size={17} /></button><button type="button" class="icon-button" aria-label={`Move ${step.name || `step ${index + 1}`} down`} disabled={busy || index === draft.steps.length - 1} onclick={() => move(index, 1)}><ArrowDown size={17} /></button><button type="button" class="icon-button remove-step" aria-label={`Remove ${step.name || `step ${index + 1}`}`} disabled={busy} onclick={() => { if (draft) draft.steps = draft.steps.filter((item) => item.id !== step.id); }}><Trash2 size={16} /></button><GripVertical class="drag-hint" size={19} aria-hidden="true" /></div></header>
         {#if expandedStepIds.has(step.id)}<div class="step-body" id={`step-details-${step.id}`}>
          <Field label="Step name" for={`name-${step.id}`} required>{#snippet children()}<input id={`name-${step.id}`} bind:value={step.name} required disabled={busy} placeholder="e.g. Research request" />{/snippet}</Field>
          <Field label="Agent" for={`agent-${step.id}`} required>{#snippet children({ describedBy, invalid })}<ResourceSelect id={`agent-${step.id}`} bind:value={step.agentId} items={config.agents} placeholder="Select an agent" kind="agent" {describedBy} {invalid} disabled={busy} />{/snippet}</Field>
         </div>{/if}
        </article>
       </li>
      {/each}
     </ol>
     {#if !draft.steps.length}<div class="empty-flow"><p>Add at least one agent step before saving.</p></div>{/if}
     <button type="button" class="add-step" disabled={busy || !config.agents.length} onclick={add}><Plus size={18} /><span>Add agent step</span></button>
     <div class="editor-footer"><span>{draft.steps.length} {draft.steps.length === 1 ? "step" : "steps"} in this pipeline</span>{#if editing}<Button variant="danger" disabled={busy} onclick={() => { deleting = true; }}><Trash2 size={16} /> Delete pipeline</Button>{/if}</div>
    </form>
    <ConfirmationDialog bind:open={deleting} title="Delete pipeline?" loading={busy} onconfirm={() => save(true)}>Existing task snapshots are preserved. This removes the pipeline from project configuration.</ConfirmationDialog>
   {/if}
  </section>
 </div>
{/if}

<style>
 .pipeline-workbench { display:grid; grid-template-columns: 16rem minmax(0,1fr); min-height: 39rem; border:1px solid #dbe0e9; border-radius:.8rem; background:#fff; overflow:hidden; box-shadow:0 12px 30px rgb(33 45 75 / 6%); }
 .pipeline-sidebar { padding:1rem; background:#f8f9fc; border-right:1px solid #e2e6ef; }.sidebar-heading { display:flex; justify-content:space-between; align-items:center; }.sidebar-heading h2 { font-size:1rem; margin:.15rem 0 0; }.count { border-radius:999px; background:#e8edfb; color:#244aab; font-size:.78rem; font-weight:700; padding:.2rem .5rem; }.pipeline-sidebar .muted { font-size:.82rem; margin:1rem 0; }
 .pipeline-list { display:grid; gap:.35rem; }.pipeline-item { display:grid; grid-template-columns:auto 1fr auto; align-items:center; gap:.65rem; width:100%; text-align:left; border:1px solid transparent; border-radius:.55rem; padding:.7rem; background:transparent; color:#27324a; cursor:pointer; font:inherit; }.pipeline-item:hover { background:#fff; border-color:#dbe0e9; }.pipeline-item.selected { background:#edf2ff; border-color:#bfcdf3; }.pipeline-item-icon { display:grid; place-items:center; color:#3055b5; }.pipeline-item strong,.pipeline-item small { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }.pipeline-item small { color:#69758b; font-size:.75rem; margin-top:.15rem; }.empty-library { display:grid; justify-items:center; text-align:center; gap:.45rem; padding:2rem .5rem; color:#69758b; font-size:.8rem; }.empty-library strong { color:#27324a; }.agent-warning { display:flex; gap:.55rem; padding:.75rem; border:1px solid #ead5a3; border-radius:.55rem; background:#fff8e8; color:#6e4a00; font-size:.76rem; line-height:1.4; }.agent-warning span { display:block; margin-top:.2rem; }
 .editor { min-width:0; background:#fff; }.editor-placeholder { display:grid; justify-items:center; align-content:center; min-height:39rem; text-align:center; padding:2rem; }.editor-placeholder p { max-width:31rem; color:#5c667a; line-height:1.55; margin:.5rem 0 1.3rem; }.placeholder-icon { display:grid; place-items:center; width:3.5rem; height:3.5rem; border-radius:1rem; color:#3055b5; background:#edf2ff; }
 .editor-toolbar { display:flex; justify-content:space-between; gap:1rem; align-items:flex-end; padding:1.5rem 1.5rem 1rem; border-bottom:1px solid #e7eaf0; }.editor-toolbar :global(.field) { margin:0; min-width:min(22rem, 100%); }.toolbar-actions { display:flex; gap:.5rem; flex-wrap:wrap; }.flow-caption { display:flex; justify-content:space-between; padding:.75rem 2rem; color:#647087; font-size:.76rem; background:#fcfcfd; border-bottom:1px solid #edf0f4; }.flow-caption span:nth-child(2) { color:#3055b5; font-weight:700; }
 .step-flow { list-style:none; padding:1.5rem 2rem .5rem; margin:0; background:linear-gradient(90deg,#fff,#f8faff 50%,#fff); }.step-node { position:relative; }.flow-line { display:grid; place-items:center; height:2.2rem; color:#6d85c6; }.step-node:first-child .flow-line { display:none; }.step-card { border:1px solid #d9e0ee; border-radius:.7rem; overflow:hidden; background:#fff; box-shadow:0 3px 10px rgb(33 45 75 / 5%); }.step-header { display:flex; align-items:center; gap:.75rem; padding:.8rem 1rem; background:#f7f9fe; border-bottom:1px solid #e4e8f1; }.step-number { display:grid; place-items:center; flex:none; width:1.8rem; height:1.8rem; border-radius:50%; background:#3055b5; color:#fff; font-size:.8rem; font-weight:800; }.step-title { min-width:0; flex:1; border:0; padding:0; background:transparent; text-align:left; font:inherit; cursor:pointer; }.step-title:hover strong { color:#244aab; }.step-title span,.step-title small { display:block; color:#69758b; font-size:.72rem; }.step-title strong { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:#202c44; margin:.1rem 0; }.step-controls { display:flex; align-items:center; gap:.2rem; }.icon-button { display:grid; place-items:center; width:2rem; height:2rem; border:0; border-radius:.4rem; background:transparent; color:#52607a; cursor:pointer; }.icon-button:hover:not(:disabled) { background:#e7edfa; color:#244aab; }.icon-button:disabled { opacity:.35; cursor:not-allowed; }:global(.drag-hint) { color:#a7b0c1; }.step-body { display:grid; grid-template-columns:1fr 1fr; gap:1rem; padding:1rem; }.remove-step { color:#8b4250; }.remove-step:hover:not(:disabled) { color:#b42318; background:#fff0ef; }.add-step { display:flex; align-items:center; justify-content:center; gap:.45rem; width:calc(100% - 4rem); margin:.5rem 2rem 1.5rem; border:1px dashed #9badcf; border-radius:.65rem; padding:.8rem; background:#f8faff; color:#244aab; font:inherit; font-weight:700; cursor:pointer; }.add-step:hover:not(:disabled) { background:#edf2ff; }.add-step:disabled { opacity:.5; cursor:not-allowed; }.empty-flow { margin:0 2rem 1rem; padding:.75rem; color:#a15c00; text-align:center; background:#fff7e5; border-radius:.5rem; }.editor-footer { display:flex; justify-content:space-between; align-items:center; padding:1rem 1.5rem; border-top:1px solid #e7eaf0; color:#69758b; font-size:.8rem; }
 @media (max-width: 720px) { .pipeline-workbench { grid-template-columns:1fr; }.pipeline-sidebar { border-right:0; border-bottom:1px solid #e2e6ef; }.pipeline-list { grid-template-columns:repeat(auto-fit,minmax(12rem,1fr)); }.editor-toolbar { align-items:stretch; flex-direction:column; }.editor-toolbar :global(.field) { min-width:0; }.step-flow { padding:1rem; }.step-body { grid-template-columns:1fr; }.add-step { width:calc(100% - 2rem); margin-inline:1rem; }.flow-caption { padding-inline:1rem; }.flow-caption span:nth-child(2) { display:none; } }
</style>
